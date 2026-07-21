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
use crate::app_config::{AppConfigStore, SecureScreenMode};
use crate::migrations::{APP_CONFIG_SCHEMA_VERSION, run_app_migrations};

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

    run_app_migrations(&state).await;

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
    // m0003 converted the deprecated secure_screen:false into Off.
    assert_eq!(reloaded.secure_screen_mode, Some(SecureScreenMode::Off));
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
    // A pre-split app.json (schema 1) so the registry actually runs on the
    // first pass (a brand-new install now starts at the target via Default).
    std::fs::write(dir.path().join("app.json"), r#"{"schema_version":1}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    run_app_migrations(&state).await;
    let after_first = AppConfigStore::new(dir.path()).get();
    assert_eq!(after_first.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(after_first.lock_mode, LockMode::Idle(300));

    // Second run is a no-op (schema_version already at target).
    run_app_migrations(&state).await;
    let after_second = AppConfigStore::new(dir.path()).get();
    assert_eq!(after_second.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(after_second.lock_mode, LockMode::Idle(300));
}

/// (c) fresh install: no repo.json → no error, marks the migration done with
/// default prefs (nothing to copy).
#[tokio::test]
async fn migrate_noops_and_marks_done_when_no_repo_json() {
    let dir = tempfile::tempdir().unwrap();
    // A pre-split app.json (schema 1) with no repo.json: m0002 has nothing to
    // copy and marks itself done, then m0003 converts the default bool.
    std::fs::write(dir.path().join("app.json"), r#"{"schema_version":1}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    run_app_migrations(&state).await;

    let reloaded = AppConfigStore::new(dir.path()).get();
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    // Defaults remain (nothing was copied).
    assert_eq!(reloaded.lock_mode, LockMode::Immediate);
    assert!(reloaded.autosync);
}

/// m0003 converts a v1 `secure_screen:true` (the default) to `None`, which is
/// `Sensitive` via the frontend — so a default user's app.json stays
/// byte-identical (no `secure_screen_mode` key written).
#[tokio::test]
async fn m0003_maps_default_true_to_none_and_stays_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":1,"secure_screen":true}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    run_app_migrations(&state).await;

    let reloaded = AppConfigStore::new(dir.path()).get();
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert!(
        reloaded.secure_screen_mode.is_none(),
        "true ⇒ None (Sensitive)"
    );
    let on_disk = std::fs::read_to_string(dir.path().join("app.json")).unwrap();
    assert!(
        !on_disk.contains("secure_screen_mode"),
        "default user stays byte-identical; got: {on_disk}",
    );
}

/// Core regression: a v2 file (already config-scope-migrated) with real
/// `lock_mode`/`autosync` + a slim repo.json must NOT have those prefs
/// overwritten — m0002 is skipped by the schema gate, so only m0003 runs.
#[tokio::test]
async fn v2_file_does_not_roll_back_scope_prefs() {
    let dir = tempfile::tempdir().unwrap();
    // A slim repo.json (post-split shape: no behavior prefs).
    std::fs::write(
        dir.path().join("repo.json"),
        r#"{"url":"https://x/repo.git","local_path":"/p"}"#,
    )
    .unwrap();
    // A v2 app.json with non-default scope prefs + secure_screen off.
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":2,"secure_screen":false,"lock_mode":{"idle":300},"autosync":false}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    run_app_migrations(&state).await;

    let reloaded = AppConfigStore::new(dir.path()).get();
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(reloaded.secure_screen_mode, Some(SecureScreenMode::Off));
    // m0002 was skipped (schema already 2), so the real prefs survive untouched.
    assert_eq!(reloaded.lock_mode, LockMode::Idle(300));
    assert!(!reloaded.autosync);
}

/// m0003 leaves an already-pinned mode alone: a v2 file that already carries
/// `secure_screen_mode:"off"` keeps it even though `secure_screen:true` would
/// otherwise map to None.
#[tokio::test]
async fn m0003_preserves_an_already_pinned_mode() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":2,"secure_screen":true,"secure_screen_mode":"off"}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    run_app_migrations(&state).await;

    let reloaded = AppConfigStore::new(dir.path()).get();
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(
        reloaded.secure_screen_mode,
        Some(SecureScreenMode::Off),
        "already-pinned mode is not overwritten by the bool",
    );
}

/// `save()` failure in m0002's copy branch propagates as `Err` (the `save()?`
/// contract), so the engine leaves `schema_version` below target and m0003
/// never runs — then a retry after the failure clears completes both steps.
/// This pins both the `?` propagation and the engine's "Err stops the chain"
/// invariant, which are otherwise defended only by the `debug_assert_eq!`.
#[tokio::test]
async fn m0002_save_failure_in_copy_branch_leaves_schema_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("repo.json"), OLD_REPO_JSON).unwrap();
    std::fs::write(dir.path().join("app.json"), r#"{"schema_version":1}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    // `save()` writes `app.tmp` then renames it over `app.json`, so a directory
    // at the tmp path makes the write fail on every platform (no chmod). m0002
    // must propagate that Err instead of marking itself done.
    std::fs::create_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        AppConfigStore::new(dir.path()).get().schema_version,
        1,
        "a failed save must not bump schema_version (read fresh off disk)"
    );

    // Clear the block and retry — the engine re-enters m0002 (schema still < 2)
    // and completes both steps to the target.
    std::fs::remove_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    let reloaded = AppConfigStore::new(dir.path()).get();
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(reloaded.lock_mode, LockMode::Idle(300)); // copied from OLD_REPO_JSON
}

/// The "nothing to copy" branch (no `repo.json`) must also propagate a save
/// failure as `Err` — this is the `let _ = save()` → `save()?` fix. Marking the
/// migration done without persisting the bump would trip the engine's
/// `debug_assert_eq!` (and silently skip the step in release).
#[tokio::test]
async fn m0002_save_failure_in_noop_branch_leaves_schema_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    // No repo.json → m0002's "nothing to copy" branch.
    std::fs::write(dir.path().join("app.json"), r#"{"schema_version":1}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()),
    );

    std::fs::create_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        AppConfigStore::new(dir.path()).get().schema_version,
        1,
        "noop-branch save failure must not mark the migration done"
    );

    std::fs::remove_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        AppConfigStore::new(dir.path()).get().schema_version,
        APP_CONFIG_SCHEMA_VERSION,
    );
}
