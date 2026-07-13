// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Config-scope migration integration tests (RFC 0038).
//!
//! Desktop tests run with `master_key = None` ⇒ seal passthrough, so a
//! pre-split `repo.json` is plaintext on disk and can be seeded directly. The
//! app-lock path (master key biometric-gated → sealed read fails
//! `SEAL_KEY_UNAVAILABLE` → soft-skip → retry on `app_unlock`) is
//! Android/Keystore-flavored and needs a git-backed keyed store to simulate, so
//! it is covered by the manual app-lock verification step instead.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};

use rustpass::{LockMode, Store};

use crate::AppState;
use crate::app_config::{APP_CONFIG_SCHEMA_VERSION, AppConfigStore};
use crate::migrate::migrate_config_scope;

/// Build an `AppState` over `store` + `app_config` with inert default caches.
/// The migration only touches `app_config`, `store`, and the `lock_mode` /
/// `clipboard_clear_secs` caches, so the rest are defaults.
fn build_state(store: Arc<Store>, app_config: AppConfigStore) -> AppState {
    AppState {
        store,
        app_config,
        lock_timer: Mutex::new(None),
        lock_generation: Arc::new(AtomicU64::new(0)),
        pending_identity: Mutex::new(None),
        lock_mode: Mutex::new(LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        clipboard_clear_handle: Mutex::new(None),
        clipboard_clear_generation: Arc::new(AtomicU64::new(0)),
        app_lock_enabled: AtomicBool::new(false),
        app_locked: AtomicBool::new(false),
        seal_migrate_state: AtomicU8::new(0),
        backend_resolve_state: AtomicU8::new(0),
        active_cancel_token: Mutex::new(None),
    }
}

/// A pre-split `repo.json` with the 5 behavior prefs at non-default values.
const OLD_REPO_JSON: &str = r#"{
    "url":"https://x/repo.git","local_path":"/p",
    "lock_mode":{"idle":300},
    "view_clear_secs":0,
    "clipboard_clear_secs":180,
    "autosync":false,
    "biometric_app_lock":true
}"#;

/// (a) compat regression + (e) preserve: a pre-split repo.json's non-default
/// behavior prefs land in app.json, and the existing app prefs (`secure_screen` /
/// locale) are not wiped.
#[tokio::test]
async fn migrate_copies_non_default_prefs_and_preserves_app_prefs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("repo.json"), OLD_REPO_JSON).unwrap();
    // Pre-existing app.json with non-default app prefs the migration must keep.
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":1,"secure_screen":false,"locale":"zh-CN"}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    migrate_config_scope(&state).await;

    let reloaded = AppConfigStore::new(dir.path()).get();
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    // The 5 behavior prefs copied from the legacy repo.json.
    assert_eq!(reloaded.lock_mode, LockMode::Idle(300));
    assert_eq!(reloaded.view_clear_secs, Some(0));
    assert_eq!(reloaded.clipboard_clear_secs, Some(180));
    assert!(!reloaded.autosync);
    assert!(reloaded.biometric_app_lock);
    // secure_screen / locale preserved (mutate-not-replace).
    assert!(!reloaded.secure_screen);
    assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    // The Store's injected autosync cache was re-pushed to the migrated value
    // (the D1 invariant — autosync_write must not read a stale pre-migration
    // `true` when the user had autosync off).
    assert!(
        !state.store.autosync(),
        "migration must re-push autosync into the Store cache"
    );
}

/// (b) idempotent: re-running after `schema_version` is bumped is a no-op.
#[tokio::test]
async fn migrate_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("repo.json"), OLD_REPO_JSON).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    migrate_config_scope(&state).await;
    let after_first = AppConfigStore::new(dir.path()).get();
    assert_eq!(after_first.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(after_first.lock_mode, LockMode::Idle(300));

    // Second run is a no-op (schema_version already at target).
    migrate_config_scope(&state).await;
    let after_second = AppConfigStore::new(dir.path()).get();
    assert_eq!(after_second.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(after_second.lock_mode, LockMode::Idle(300));
}

/// (c) fresh install: no repo.json → no error, marks the migration done with
/// default prefs (nothing to copy).
#[tokio::test]
async fn migrate_noops_and_marks_done_when_no_repo_json() {
    let dir = tempfile::tempdir().unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    migrate_config_scope(&state).await;

    let reloaded = AppConfigStore::new(dir.path()).get();
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    // Defaults remain (nothing was copied).
    assert_eq!(reloaded.lock_mode, LockMode::Immediate);
    assert!(reloaded.autosync);
}
