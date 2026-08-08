// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Headless background-sync entry for the Android `WorkManager` Worker.
//!
//! [`run_headless_sync`] is the pure, host-testable core (gates + `Store` +
//! pull-only `sync_with` + the attention flag). The `#[no_mangle]` JNI shim at
//! the bottom is `#[cfg(target_os = "android")]`; on other targets this module
//! still compiles so the core is unit-testable without a device. The JNI symbol
//! lands in `libgpm_lib.so` (the existing cdylib) and is called by
//! `xyz.yzx9.gpm.SyncWorker` over JNI.

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
/// Gates (defense-in-depth, since the Worker also gates): the auth-free master
/// key present, cadence ≠ `Off`, `is_repo_ready`, and `AutoSync`-on (background
/// sync is linked to `AutoSync`). Runs under App Lock too (R064): the worker
/// reads only the auth-free master key — never the biometric-gated vault key —
/// so `repo.json` (git credential) is readable while the identity stays gated.
/// R074: cadence + `AutoSync` now live in the sealed merged `app.json`, so the
/// key is decoded + the config loaded BEFORE the gates (no plaintext `pref.json`
/// to read first anymore). Then a pull-only `run_best_effort_sync` (the shared
/// private-slot + 30s deadline helper). On a divergence or an authenticity-
/// blocked fast-forward, atomically creates a passive attention marker file for
/// the next foreground to consume. The full `SyncOutcome` carries secret entry
/// names and is not persisted.
#[allow(dead_code)] // called by the Android-only JNI shim + the host gate tests.
pub(crate) async fn run_headless_sync(
    config_dir: PathBuf,
    master_key_b64: String,
) -> BackgroundSyncResult {
    // R074: the auth-free key must be decoded FIRST — cadence + autosync now
    // live in the sealed merged app.json, which is unreadable without it. The
    // key arrives base64-encoded (the Rust↔Keystore IPC shape). Missing ⇒ the
    // store isn't set up yet (R064: the auth-free master is permanent, so its
    // absence is never "App Lock on") — skip, don't error.
    let Some(master_key) = crate::decode_master_key(&master_key_b64) else {
        return BackgroundSyncResult::Skipped { reason: "no_key" };
    };

    let app_cfg = AppConfigStore::new(&config_dir).await;
    let store = Arc::new(Store::new(config_dir, Some(master_key)));
    // R064: the worker is pull-only — it reads `repo.json`/`app.json` under the
    // auth-free master but never the identity. Drop the vault_seal bridge so no
    // identity key sits in worker memory, and migrate ONLY `repo_config` (the
    // vault-tier files are under the distinct vault key the worker lacks).
    store.set_vault_key(None);
    app_cfg.set_store(Arc::clone(&store));
    // Load the sealed merged config so the gates below read the persisted
    // cadence + autosync (not cold-start defaults).
    if let Err(e) = app_cfg.reload().await {
        return BackgroundSyncResult::Error {
            message: e.to_string(),
        };
    }
    // Cadence gate.
    if app_cfg.background_sync().is_off() {
        return BackgroundSyncResult::Skipped { reason: "disabled" };
    }
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
    // AutoSync gate (autosync loaded above from the merged config).
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
    /// breaks.
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
    pub extern "system" fn Java_xyz_yzx9_gpm_SyncWorker_nativeSync<'local>(
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

    use std::sync::Arc;

    use rustpass::Store;

    use crate::app_config::BackgroundSyncCadence;
    use base64::Engine;

    use super::*;

    /// Seed `dir` with a sealed merged app config carrying `cadence`, via a
    /// desktop-passthrough Store. R074: cadence lives in the sealed merged
    /// `app.json`, so a worker must load it with the auth-free key.
    async fn seed_cadence(dir: &std::path::Path, cadence: BackgroundSyncCadence) {
        let app_cfg = AppConfigStore::new(dir).await;
        app_cfg.set_store(Arc::new(Store::new(dir.to_path_buf(), None)));
        app_cfg
            .set_background_sync(cadence)
            .await
            .expect("set cadence");
    }

    #[tokio::test]
    async fn skips_when_disabled() {
        // R074: cadence is read from the sealed merged app.json AFTER the key is
        // decoded + the config loaded. A real key + the default (Off) cadence ⇒
        // the cadence gate fires (no app.json yet ⇒ reload defaults to Off).
        let dir = tempfile::TempDir::new().expect("tempdir");
        let master = rustpass::seal::generate_master_key().unwrap();
        let master_b64 = crate::B64.encode(master);
        let res = run_headless_sync(dir.path().to_path_buf(), master_b64).await;
        assert!(matches!(
            res,
            BackgroundSyncResult::Skipped { reason: "disabled" }
        ));
    }

    #[tokio::test]
    async fn skips_with_no_key() {
        // R074: the auth-free key is decoded FIRST (cadence is sealed). A bad key
        // ⇒ no_key before the cadence gate, regardless of the persisted cadence.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let res = run_headless_sync(dir.path().to_path_buf(), String::from("not-a-key")).await;
        assert!(matches!(
            res,
            BackgroundSyncResult::Skipped { reason: "no_key" }
        ));
    }

    /// R074: with a real key + a non-Off cadence in the sealed merged config, the
    /// worker proceeds past the cadence + autosync gates to `not_ready` (no
    /// repo.json) — pinning that the key-first order loads the sealed cadence
    /// correctly. Also pins the R064 chunk-7 wiring: the pull-only worker never
    /// touches vault-tier files (a plaintext identity is left untouched; a revert
    /// to `migrate_seal` would `SealKeyUnavailable` on it ⇒ `Error`).
    #[tokio::test]
    async fn headless_sync_proceeds_past_cadence_and_leaves_identity_untouched() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        seed_cadence(dir.path(), BackgroundSyncCadence::Hours1).await;
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
            "with key + cadence, the worker proceeds to not_ready (no repo.json)"
        );
        assert_eq!(
            std::fs::read(dir.path().join("identity")).unwrap(),
            b"plaintext-identity",
            "headless worker must not touch vault-tier identity"
        );
    }
}
