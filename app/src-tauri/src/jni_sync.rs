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

use rustpass::SyncOutcome;
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
/// key present, cadence ≠ `Off`, and `AutoSync`-on (background sync is linked
/// to `AutoSync`). The per-repo migrate/resolve/`is_repo_ready` gates run
/// inside [`sync_registered_repositories`], one facade per registered repo.
/// Runs under App Lock too (R064): the worker
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
    // Shared headless gate chain (key decode → device facade → schema gate →
    // config reload); see `headless::app_context` for the R074/R064/D2
    // rationale behind the order.
    let (master_key, app_cfg) =
        match crate::headless::app_context(&config_dir, &master_key_b64).await {
            crate::headless::HeadlessGates::Skipped(reason) => {
                return BackgroundSyncResult::Skipped { reason };
            }
            crate::headless::HeadlessGates::Error { message } => {
                return BackgroundSyncResult::Error { message };
            }
            crate::headless::HeadlessGates::Ready {
                master_key,
                app_cfg,
            } => (master_key, app_cfg),
        };
    // Cadence gate.
    if app_cfg.background_sync().is_off() {
        return BackgroundSyncResult::Skipped { reason: "disabled" };
    }
    // AutoSync gate (background sync is linked to AutoSync; autosync loaded above
    // from the merged config). The per-repo migrate/resolve/ready gates run INSIDE
    // `sync_registered_repositories` with the worker fan-out — each repo's facade
    // is built there at `repositories/<id>/`.
    if !app_cfg.get_behavior().autosync {
        return BackgroundSyncResult::Skipped {
            reason: "autosync_off",
        };
    }
    // Fan out over the registered repositories (built at repositories/<id>/).
    sync_registered_repositories(&config_dir, master_key, &app_cfg).await
}

/// Iterate the registered repositories and pull-sync each ready one. The
/// host-testable core of the worker fan-out: [`run_headless_sync`] runs the
/// device-level gates (key, schema, cadence, autosync), then this handles the
/// per-repo work.
///
/// Per-repo error-tolerant (D2): a facade build, `repo.json` read, or clone
/// missing mid-migrate skips THAT repo (`continue`), never a hard `Error` — the
/// next `WorkManager` fire retries. Returns `Skipped { "no_repositories" }` when
/// the registry is empty, `Skipped { "no_syncable_repo" }` when none was ready,
/// else `Ok` with the last synced outcome (one repo in Step 1).
#[allow(dead_code)] // called by run_headless_sync + the host gate tests.
async fn sync_registered_repositories(
    config_dir: &std::path::Path,
    master_key: [u8; 32],
    app_cfg: &AppConfigStore,
) -> BackgroundSyncResult {
    let ids = app_cfg.get().repositories.clone();
    if ids.is_empty() {
        return BackgroundSyncResult::Skipped {
            reason: "no_repositories",
        };
    }
    let mut synced: Option<SyncOutcome> = None;
    for id_str in &ids {
        // Build a facade at repositories/<id>/. Migrate ONLY repo_config (the
        // vault-tier files are under the distinct vault key the worker lacks).
        let id = crate::registry::RepoId::from(id_str.clone());
        let store = crate::build_repo_facade(config_dir, Some(master_key), &id);
        store.set_vault_key(None);
        // migrate_repo_seal also normalizes a legacy absent `crypto` field (a
        // repo.json content rewrite). Kept before resolve_storage/pull by
        // convention (migrations before ops); a pull does not touch repo.json
        // (it lives in the config dir, not the git working tree), so there is
        // no pull-vs-migration race. Cross-writer serialization is handled by
        // the ConfigLock every repo.json writer takes (R097); see
        // Config::normalize_repo_config_crypto.
        if store.migrate_repo_seal().await.is_err() {
            continue;
        }
        if store.resolve_storage().await.is_err() {
            continue;
        }
        if !store.is_repo_ready() {
            continue;
        }
        // Pull-only: the heavy-autofill persona is read-only, so push is dead
        // weight. Foreground autosync_write still publishes occasional creates.
        let store_for_sync = Arc::clone(&store);
        let result = crate::write::run_best_effort_sync(|slot, cancel| async move {
            store_for_sync.sync_with(&slot, Some(cancel), None).await
        })
        .await;
        if let Ok(outcome) = result {
            // Persist a passive marker only (the full SyncOutcome leaks entry
            // names). A dedicated marker file — NOT a pref.json field — so the
            // write can't race a concurrent foreground pref write.
            let needs_attention = matches!(outcome, SyncOutcome::Diverged(_))
                || matches!(&outcome, SyncOutcome::FastForwarded(r) if r.authenticity.blocked);
            if needs_attention && let Err(e) = app_cfg.set_sync_attention_marker().await {
                log::warn!("bg-sync: could not persist attention marker: {e}");
            }
            synced = Some(outcome);
        }
    }
    match synced {
        Some(outcome) => BackgroundSyncResult::Ok { outcome },
        None => BackgroundSyncResult::Skipped {
            reason: "no_syncable_repo",
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
    /// worker proceeds past the cadence + autosync gates to `no_repositories`
    /// (empty registry, nothing to sync) — pinning that the key-first order loads
    /// the sealed cadence correctly. Also pins the R064 chunk-7 wiring: the
    /// pull-only worker never touches vault-tier files (a plaintext identity is
    /// left untouched; a revert to `migrate_seal` would `SealKeyUnavailable` on
    /// it ⇒ `Error`).
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
                    reason: "no_repositories"
                }
            ),
            "with key + cadence + autosync, the worker proceeds past the gates to \
             no_repositories (empty registry, no repo to sync)"
        );
        assert_eq!(
            std::fs::read(dir.path().join("identity")).unwrap(),
            b"plaintext-identity",
            "headless worker must not touch vault-tier identity"
        );
    }

    /// D2: a `WorkManager` fire that lands mid-m0010 (schema still < current) must
    /// skip, never error — the repo files may be half-moved.
    #[tokio::test]
    async fn skips_when_schema_below_current() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let master = rustpass::seal::generate_master_key().unwrap();
        let master_b64 = crate::B64.encode(master);
        // Seed a pre-m0010 app.json (schema 9) with a registered repo id, sealed
        // with the master key. The schema gate fires before the cadence gate.
        let store = Arc::new(Store::new(dir.path().to_path_buf(), Some(master)));
        let cfg = crate::app_config::AppConfig {
            schema_version: 9,
            repositories: vec!["0123456789abcdef0123456789abcdef".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_vec(&cfg).unwrap();
        store.save_app_config(&json).await.unwrap();
        drop(store);

        let res = run_headless_sync(dir.path().to_path_buf(), master_b64).await;
        assert!(
            matches!(
                res,
                BackgroundSyncResult::Skipped {
                    reason: "mid_migrate"
                }
            ),
            "schema < current must skip (mid-migrate): {res:?}"
        );
    }
}
