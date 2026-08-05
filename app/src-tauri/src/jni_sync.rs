// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Headless background-sync entry for the Android `WorkManager` Worker.
//!
//! [`run_headless_sync`] is the pure, host-testable core (gates + `Store` +
//! pull-only `sync_with` + the attention flag). The `#[no_mangle]` JNI shim at
//! the bottom is `#[cfg(target_os = "android")]`; on other targets this module
//! still compiles so the core is unit-testable without a device. The JNI symbol
//! lands in `libgpm_lib.so` (the existing cdylib) and is called by
//! `xyz.yzx9.gpm.backgroundsync.SyncWorker` over JNI.

use std::path::PathBuf;
use std::sync::Arc;

use rustpass::{Store, SyncOutcome};
use serde::Serialize;

use crate::app_config::AppConfigStore;

/// The result crossed back to `Kotlin` as JSON, then mapped to a `WorkManager`
/// `Result` (`ok`/`skipped` → `Result.success()`, `error` → `retry`/`failure`).
#[allow(dead_code)] // used by the Android-only JNI shim + the host gate tests.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum BackgroundSyncResult {
    /// The sync ran and produced this outcome (fast-forwarded or diverged).
    Ok { outcome: SyncOutcome },
    /// Skipped before touching the network (no publish/pull). Not an error.
    Skipped { reason: &'static str },
    /// An error worth a `WorkManager` retry (mapped to `failure` after N tries).
    Error { message: String },
}

/// Run one best-effort background sync (pull-only) headlessly — no
/// Tauri runtime, no `AppHandle`. The JNI shim constructs this from a config
/// dir + a base64 master key the Kotlin Worker retrieved from the Keystore.
///
/// Gates (defense-in-depth, since the Worker also gates): cadence ≠ `Off`
/// (`pref.json`), `is_repo_ready`, and `AutoSync`-on (background sync is linked
/// to `AutoSync`). Runs under App Lock too (R064): the worker reads only the
/// auth-free master key — never the biometric-gated vault key — so `repo.json`
/// (git credential) is readable while the identity stays gated. Then a pull-only
/// `run_best_effort_sync` (the shared private-slot +
/// 30s deadline helper). On a divergence or an authenticity-blocked
/// fast-forward, atomically creates a passive attention marker file (NOT a
/// `pref.json` field, so the write can't race a foreground pref write) for the
/// next foreground to consume. The full `SyncOutcome` carries secret entry
/// names and is not persisted.
#[allow(dead_code)] // called by the Android-only JNI shim + the host gate tests.
pub(crate) async fn run_headless_sync(
    config_dir: PathBuf,
    master_key_b64: String,
) -> BackgroundSyncResult {
    // pref.json is plaintext — readable without the master key, so the cadence
    // gate works pre-unlock too.
    let app_cfg = AppConfigStore::new(&config_dir).await;
    if app_cfg.background_sync().is_off() {
        return BackgroundSyncResult::Skipped { reason: "disabled" };
    }

    // The master key arrives base64-encoded (the existing Rust↔Keystore IPC
    // shape). Missing ⇒ the store isn't set up yet (R064: the auth-free master
    // is permanent, so its absence is never "App Lock on") — skip, don't error.
    let Some(master_key) = crate::decode_master_key(&master_key_b64) else {
        return BackgroundSyncResult::Skipped { reason: "no_key" };
    };

    let store = Arc::new(Store::new(config_dir, Some(master_key)));
    // R064: the worker is pull-only — it reads `repo.json`/`app.json` under the
    // auth-free master but never the identity. Drop the vault_seal bridge so no
    // identity key sits in worker memory, and migrate ONLY `repo_config` (the
    // vault-tier files are under the distinct vault key the worker lacks).
    store.set_vault_key(None);
    app_cfg.set_store(Arc::clone(&store));
    if let Err(e) = store.migrate_repo_seal().await {
        return BackgroundSyncResult::Error {
            message: e.to_string(),
        };
    }
    if let Err(e) = store.resolve_storage().await {
        return BackgroundSyncResult::Error {
            message: e.to_string(),
        };
    }
    if !store.is_repo_ready() {
        return BackgroundSyncResult::Skipped {
            reason: "not_ready",
        };
    }
    // AutoSync gate. Reading the persisted flag unseals app.json via the
    // master key (already injected above).
    if let Err(e) = app_cfg.reload_behavior().await {
        return BackgroundSyncResult::Error {
            message: e.to_string(),
        };
    }
    if !app_cfg.get_behavior().autosync {
        return BackgroundSyncResult::Skipped {
            reason: "autosync_off",
        };
    }

    // Pull-only: the heavy-autofill persona is read-only, so push is dead
    // weight. Foreground autosync_write still publishes occasional creates.
    let store_for_sync = Arc::clone(&store);
    let result = crate::write::run_best_effort_sync(|slot, cancel| async move {
        store_for_sync.sync_with(&slot, Some(cancel), None).await
    })
    .await;

    match result {
        Ok(outcome) => {
            // Persist a passive marker only (the full SyncOutcome leaks entry
            // names). A dedicated marker file — NOT a pref.json field — so the
            // write can't race a concurrent foreground pref write.
            let needs_attention = matches!(outcome, SyncOutcome::Diverged(_))
                || matches!(&outcome, SyncOutcome::FastForwarded(r) if r.authenticity.blocked);
            let marker_res = if needs_attention {
                app_cfg.set_sync_attention_marker().await
            } else {
                Ok(())
            };
            if let Err(e) = marker_res {
                log::warn!("bg-sync: could not persist attention marker: {e}");
            }
            BackgroundSyncResult::Ok { outcome }
        }
        Err(e) => BackgroundSyncResult::Error {
            message: e.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Android JNI shim — not compiled on other targets (verified via
// `just android-debug`, not on the host).
// ---------------------------------------------------------------------------
#[cfg(target_os = "android")]
mod jni {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use jni::EnvUnowned;
    use jni::errors::LogErrorAndDefault;
    use jni::objects::{JClass, JString};
    use tokio::runtime::Runtime;

    use super::BackgroundSyncResult;

    /// A single-threaded tokio runtime owned by the JNI entry. WorkManager's
    /// `ExistingPeriodicWorkPolicy::KEEP`/`REPLACE` prevents concurrent
    /// `block_on`s; `worker_threads(1)` is still safe if that invariant ever
    /// breaks (A2).
    fn runtime() -> &'static Runtime {
        static RT: OnceLock<Runtime> = OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("gpm bg-sync runtime")
        })
    }

    /// Entry point for `SyncWorker.nativeSync(configDir, masterKeyB64)`.
    /// Returns the [`BackgroundSyncResult`] serialized as JSON.
    ///
    /// # Safety
    /// JNI surface — called from Kotlin with valid string arguments.
    // JNI FFI entry: the `unsafe(no_mangle)` attribute is intrinsic to exporting
    // a JVM-callable symbol (edition 2024 requires the `unsafe(...)` wrapper).
    // The function body itself is safe Rust.
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_xyz_yzx9_gpm_backgroundsync_SyncWorker_nativeSync<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _class: JClass<'local>,
        config_dir: JString<'local>,
        master_key_b64: JString<'local>,
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| -> jni::errors::Result<JString<'local>> {
                // Kotlin passes valid non-null Strings; if a read fails the value
                // degrades to "" so the sync skips on `no_key` / `not_ready`
                // rather than throwing.
                let config_dir: String = config_dir.try_to_string(env).unwrap_or_default();
                let master_key_b64: String = master_key_b64.try_to_string(env).unwrap_or_default();

                // `catch_unwind` so a Rust panic (e.g. a poisoned `Mutex`) inside
                // the sync returns an error JSON instead of a null. (`with_env`
                // already catches panics, but the `LogErrorAndDefault` policy
                // below would turn one into a null — this inner guard preserves
                // the error-JSON contract. Kotlin's `catch(Throwable)` can't
                // catch a native abort either way.)
                let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime().block_on(super::run_headless_sync(
                        PathBuf::from(config_dir),
                        master_key_b64,
                    ))
                })) {
                    Ok(r) => r,
                    Err(_) => BackgroundSyncResult::Error {
                        message: "internal panic".to_string(),
                    },
                };
                let json = serde_json::to_string(&result).unwrap_or_else(|_| {
                    r#"{"status":"error","message":"serialize_failed"}"#.to_string()
                });
                env.new_string(json)
            })
            // `LogErrorAndDefault` (not `ThrowRuntimeExAndDefault`): a JNI failure
            // logs and returns a null string instead of throwing — the Kotlin
            // Worker treats null as failure, matching the prior `unwrap_or(null)`.
            .resolve::<LogErrorAndDefault>()
    }
}

#[cfg(test)]
mod tests {

    use crate::app_config::BackgroundSyncCadence;
    use base64::Engine;

    use super::*;

    #[tokio::test]
    async fn skips_when_disabled() {
        // Default cadence is Off ⇒ the cadence gate fires before the key or
        // repo is touched. No Store / Keystore needed.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let res = run_headless_sync(dir.path().to_path_buf(), String::new()).await;
        assert!(matches!(
            res,
            BackgroundSyncResult::Skipped { reason: "disabled" }
        ));
    }

    #[tokio::test]
    async fn skips_with_no_key_when_enabled() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        // Enable background sync (cadence non-Off) so the cadence gate passes.
        let app_cfg = AppConfigStore::new(dir.path()).await;
        app_cfg
            .set_background_sync(BackgroundSyncCadence::Hours1)
            .await
            .expect("set cadence");
        // A non-32-byte "key" fails to decode ⇒ no_key skip (AppLock / not set
        // up). Still no Store or Keystore needed.
        let res = run_headless_sync(dir.path().to_path_buf(), String::from("not-a-key")).await;
        assert!(matches!(
            res,
            BackgroundSyncResult::Skipped { reason: "no_key" }
        ));
    }

    /// Pins the R064 chunk-7 wiring: `run_headless_sync` must call
    /// `migrate_repo_seal` (not `migrate_seal`) so the pull-only worker never
    /// touches vault-tier files. With a plaintext identity + the wiped
    /// `vault_seal`, a revert to `migrate_seal` would `SealKeyUnavailable` on the
    /// plaintext identity ⇒ `Error`; repo-only migrate skips it cleanly and the
    /// worker skips `not_ready` (no repo.json).
    #[tokio::test]
    async fn headless_sync_leaves_plaintext_identity_untouched() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let app_cfg = AppConfigStore::new(dir.path()).await;
        app_cfg
            .set_background_sync(BackgroundSyncCadence::Hours1)
            .await
            .expect("set cadence");
        let master = rustpass::seal::generate_master_key().unwrap();
        let master_b64 = crate::B64.encode(master);
        std::fs::write(dir.path().join("identity"), b"plaintext-identity").unwrap();

        let res = run_headless_sync(dir.path().to_path_buf(), master_b64).await;

        assert!(
            matches!(
                res,
                BackgroundSyncResult::Skipped {
                    reason: "not_ready"
                }
            ),
            "pull-only worker should skip cleanly, not error on vault-tier files"
        );
        assert_eq!(
            std::fs::read(dir.path().join("identity")).unwrap(),
            b"plaintext-identity",
            "headless worker must not touch vault-tier identity"
        );
    }
}
