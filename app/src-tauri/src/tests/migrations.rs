// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Config-scope migration integration tests (RFC 0038 + RFC 0058).
//!
//! Desktop tests run with `master_key = None` ⇒ seal passthrough, so a
//! pre-split `repo.json` is plaintext on disk and can be seeded directly. The
//! app-lock path (master key biometric-gated → sealed read fails
//! `SEAL_KEY_UNAVAILABLE` → soft-skip → retry on `app_unlock`) is simulated for
//! the m0005 app-lock cases via a keyless Store plus the `app_lock_enabled`
//! flag; the half-migrated recovery case (m0005 wrote `pref.json` but the
//! sealed write is deferred) is exercised against the schema-based
//! behavior-load discriminator.

use std::path::Path;
use std::sync::atomic::{self, AtomicBool, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};

use rustpass::{LockMode, Store};

use crate::AppState;
use crate::app_config::{
    AppConfig, AppConfigStore, BackgroundSyncCadence, PeekOutcome, SecureScreenMode,
    VERBOSE_WINDOW_SECS, now_unix,
};
use crate::migrations::{APP_CONFIG_SCHEMA_VERSION, run_app_migrations};

/// Build an `AppState` over `store` + `app_config` with inert default caches.
/// The migration only touches `app_config`, `store`, and the `lock_mode` /
/// `clipboard_clear_secs` caches, so the rest are defaults.
fn build_state(store: Arc<Store>, app_config: AppConfigStore) -> AppState {
    // Bind the store so the behavior-slot writes/reads (m0005's sealed split,
    // the behavior setters) work in tests — mirrors init_state's `set_store`.
    app_config.set_store(Arc::clone(&store));
    AppState {
        store,
        app_config,
        app_handle: None,
        lock_timer: crate::identity::IdleTimer::new(),
        pending_identity: Mutex::new(None),
        lock_mode: Mutex::new(LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        clipboard_clear_handle: Mutex::new(None),
        clipboard_clear_generation: Arc::new(AtomicU64::new(0)),
        app_lock_enabled: AtomicBool::new(false),
        app_locked: Arc::new(AtomicBool::new(false)),
        gate_idle_timer: crate::identity::IdleTimer::new(),
        last_activity_at: Mutex::new(std::time::Instant::now()),
        identity_coupled: AtomicBool::new(false),
        seal_migrate_state: AtomicU8::new(0),
        backend_resolve_state: AtomicU8::new(0),
        active_cancel_slot: Arc::new(Mutex::new(None)),
        verbose_timer: Mutex::new(None),
        verbose_generation: Arc::new(AtomicU64::new(0)),
    }
}

/// Construct a fresh store over `dir`, bind `store` (so the sealed behavior slot
/// is readable), reload behavior from disk, and return the merged view. Mirrors
/// `init_state`/`app_unlock`: `AppConfigStore::new` alone loads only the
/// plaintext display half; the sealed behavior half is loaded post-unlock via
/// `reload_behavior`. Tests that assert behavior fields after a run must go
/// through here to verify true on-disk persistence.
async fn reload_at(dir: &Path, store: &Arc<Store>) -> AppConfig {
    let ac = AppConfigStore::new(dir).await;
    ac.set_store(Arc::clone(store));
    ac.reload_behavior().await.ok();
    ac.get()
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
/// behavior prefs land in the post-split files, the existing app pref (locale)
/// is preserved, and the deprecated `secure_screen:false` converts to
/// `secure_screen_mode:Off` (m0003).
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
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    // The 5 behavior prefs copied from the legacy repo.json.
    assert_eq!(reloaded.lock_mode, LockMode::Idle(300));
    assert_eq!(reloaded.view_clear_secs, Some(0));
    assert_eq!(reloaded.clipboard_clear_secs, Some(180));
    assert!(!reloaded.autosync);
    assert!(reloaded.biometric_app_lock);
    // locale preserved (mutate-not-replace).
    assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    // m0003 converted the deprecated secure_screen:false into Off.
    assert_eq!(reloaded.secure_screen_mode, Some(SecureScreenMode::Off));
    // The Store's injected autosync cache was re-pushed to the migrated value
    // (the invariant — autosync_write must not read a stale pre-migration
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
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;
    let after_first = reload_at(dir.path(), &state.store).await;
    assert_eq!(after_first.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(after_first.lock_mode, LockMode::Idle(300));

    // Second run is a no-op (schema_version already at target).
    run_app_migrations(&state).await;
    let after_second = reload_at(dir.path(), &state.store).await;
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
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
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
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert!(
        reloaded.secure_screen_mode.is_none(),
        "true ⇒ None (Sensitive)"
    );
    // R074: pref.json is collapsed into the sealed merged app.json (desktop:
    // plaintext AppConfig). The default-sensitive user has secure_screen_mode =
    // None, so the merged file omits the key (byte-identical on default).
    let app_on_disk = std::fs::read_to_string(dir.path().join("app.json")).unwrap();
    assert!(
        !app_on_disk.contains("secure_screen_mode"),
        "default user stays byte-identical; got: {app_on_disk}",
    );
}

/// Core regression: a v2 file (already config-scope-migrated) with real
/// `lock_mode`/`autosync` + a slim repo.json must NOT have those prefs
/// overwritten — m0002 is skipped by the schema gate, so only m0003/m0004/m0005
/// run.
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
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
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
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(
        reloaded.secure_screen_mode,
        Some(SecureScreenMode::Off),
        "already-pinned mode is not overwritten by the bool",
    );
}

/// `write_app_json_raw` failure in m0002's copy branch propagates as `Err`
/// (the `?` contract), so the engine leaves `schema_version` below target and
/// m0003 never runs — then a retry after the failure clears completes both
/// steps. This pins both the `?` propagation and the engine's "Err stops the
/// chain" invariant, which are otherwise defended only by the
/// `debug_assert_eq!`.
#[tokio::test]
async fn m0002_save_failure_in_copy_branch_leaves_schema_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("repo.json"), OLD_REPO_JSON).unwrap();
    std::fs::write(dir.path().join("app.json"), r#"{"schema_version":1}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    // `write_app_json_raw` (via `save_atomic`) writes `app.tmp` then renames it
    // over `app.json`, so a directory at the tmp path makes the write fail on
    // every platform (no chmod). m0002 must propagate that Err instead of
    // marking itself done.
    std::fs::create_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        reload_at(dir.path(), &state.store).await.schema_version,
        1,
        "a failed save must not bump schema_version (read fresh off disk)"
    );

    // Clear the block and retry — the engine re-enters m0002 (schema still < 2)
    // and completes both steps to the target.
    std::fs::remove_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    let reloaded = reload_at(dir.path(), &state.store).await;
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
        AppConfigStore::new(dir.path()).await,
    );

    std::fs::create_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        reload_at(dir.path(), &state.store).await.schema_version,
        1,
        "noop-branch save failure must not mark the migration done"
    );

    std::fs::remove_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        reload_at(dir.path(), &state.store).await.schema_version,
        APP_CONFIG_SCHEMA_VERSION,
    );
}

/// m0004 carries a previously pinned `"debug"` level into the verbose flag: a v3
/// file with `log_level:"debug"` lands a `verbose_until` deadline ~one window
/// ahead, so the upgrade keeps Debug then expires under the same time-box (RFC
/// 0055). `schema_version` bumps to 4 (then m0005 bumps it to 5).
#[tokio::test]
async fn m0004_carries_pinned_debug_into_verbose() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":3,"log_level":"debug"}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    let deadline = reloaded
        .verbose_until
        .expect("debug ⇒ verbose_until stamped");
    assert!(deadline > now_unix(), "deadline is in the future");
    assert!(deadline <= now_unix() + VERBOSE_WINDOW_SECS);
}

/// Every level other than `"debug"` collapses to the Info default: a v3 file
/// with `log_level` of `"warn"` / `"info"` / `"error"` leaves `verbose_until`
/// as `None` (no verbose session started).
#[tokio::test]
async fn m0004_collapses_non_debug_levels_to_info_default() {
    for level in ["warn", "info", "error"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("app.json"),
            format!(r#"{{"schema_version":3,"log_level":"{level}"}}"#),
        )
        .unwrap();
        let state = build_state(
            Arc::new(Store::new(dir.path().to_path_buf(), None)),
            AppConfigStore::new(dir.path()).await,
        );

        run_app_migrations(&state).await;

        let reloaded = reload_at(dir.path(), &state.store).await;
        assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
        assert!(
            reloaded.verbose_until.is_none(),
            "{level} must collapse to the Info default"
        );
    }
}

/// m0004 leaves an already-set `verbose_until` alone even when `log_level` is
/// `"debug"` — a partially-migrated file re-running this step keeps its value
/// rather than restamping the deadline.
#[tokio::test]
async fn m0004_preserves_an_already_set_verbose_until() {
    let dir = tempfile::tempdir().unwrap();
    let pinned = now_unix() + 42; // an arbitrary pre-existing deadline
    std::fs::write(
        dir.path().join("app.json"),
        format!(r#"{{"schema_version":3,"log_level":"debug","verbose_until":{pinned}}}"#),
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(
        reloaded.verbose_until,
        Some(pinned),
        "an existing verbose_until is not overwritten by the debug carry-over"
    );
}
/// `m0005` defers (`Pending`) under the app-launch lock when the master key is
/// not yet injected: app.json stays plaintext-legacy (NOT sealed — that would
/// lock the user out of their own prefs under the wiped key), `pref.json` is
/// written (the display half is plaintext, no key needed), and `schema_version`
/// does NOT advance. Seeds schema-4 so `m0002`/`m0003`/`m0004_verbose_from_debug`
/// are gated out and only `m0005` runs. The corrected guard is
/// `app_lock_enabled && !has_master_key()`.
#[tokio::test]
async fn m0005_pending_under_app_lock_leaves_app_json_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":4,"lock_mode":{"idle":120},"autosync":false}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)), // keyless cold start
        AppConfigStore::new(dir.path()).await,
    );
    state.app_lock_enabled.store(true, atomic::Ordering::SeqCst);

    run_app_migrations(&state).await;

    let on_disk = std::fs::read(dir.path().join("app.json")).unwrap();
    assert!(
        !rustpass::seal::is_envelope(&on_disk),
        "app-lock cold start must NOT seal app.json (the key is withheld)"
    );
    assert_eq!(
        state.app_config.get_pref().schema_version,
        4,
        "schema must not advance while the sealed write is deferred"
    );
    assert!(
        dir.path().join("pref.json").exists(),
        "pref.json is written (display half is plaintext, no key needed)"
    );
}

/// REGRESSION for the corrected m0005 guard. `app_locked` is cleared in
/// `app_unlock` AFTER `run_app_migrations` — so at the `app_unlock` migration
/// call the key IS in memory (injected there) but `app_locked` is still `true`.
/// Guarding on `app_locked` would defer forever. This test mirrors that
/// ordering: `app_lock_enabled` stays `true` throughout (as it does on mobile),
/// the first run defers (key withheld), then injecting the master key — with
/// `app_lock_enabled` still `true` — lets the second run complete. Seeds
/// schema-4 so only `m0005` runs. Pins that the guard keys off master-key
/// presence, not the `app_locked` runtime flag.
#[tokio::test]
async fn m0005_completes_when_master_key_injected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":4,"lock_mode":{"idle":120},"autosync":false,"secure_screen_mode":"always"}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)), // keyless cold start
        AppConfigStore::new(dir.path()).await,
    );
    state.app_lock_enabled.store(true, atomic::Ordering::SeqCst);

    // First run: app-lock cold start, master key withheld → m0005 defers.
    run_app_migrations(&state).await;
    let after_first = std::fs::read(dir.path().join("app.json")).unwrap();
    assert!(
        !rustpass::seal::is_envelope(&after_first),
        "deferred on first run"
    );
    assert_eq!(state.app_config.get_pref().schema_version, 4);

    // Inject the master key (mirrors app_unlock, which bridges both setters while
    // the keys aren't split). app_lock_enabled stays true — the corrected guard
    // must still proceed.
    let key = rustpass::seal::generate_master_key().unwrap();
    state.store.set_master_key(Some(key));
    state.store.set_vault_key(Some(key));
    run_app_migrations(&state).await;

    // Completed: app.json is now a sealed envelope, schema advanced to target.
    let after_second = std::fs::read(dir.path().join("app.json")).unwrap();
    assert!(
        rustpass::seal::is_envelope(&after_second),
        "completed once the master key is present"
    );
    assert_eq!(
        state.app_config.get_pref().schema_version,
        APP_CONFIG_SCHEMA_VERSION,
    );
}

/// m0007 desktop path: `app_handle` is `None` (no Keystore) and App Lock is off,
/// so there is nothing to relocate — the migration just advances the schema.
/// Seeds schema-6 (post-`m0006`) so only `m0007` runs. Pins the no-op + the bump.
#[tokio::test]
async fn m0007_no_op_on_desktop_advances_schema() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pref.json"), r#"{"schema_version":6}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );
    // app_handle None + app_lock_enabled false ⇒ desktop no-op: bump to target.
    run_app_migrations(&state).await;
    assert_eq!(
        state.app_config.get_pref().schema_version,
        APP_CONFIG_SCHEMA_VERSION,
    );
}

/// m0007 defers (`Pending`) at cold start under App Lock: `app_lock_enabled` is
/// on but the vault key is not yet injected (it arrives only after the
/// `app_unlock` biometric prompt). The engine must leave schema below target so
/// the next `app_unlock` retries. Mirrors `m0005`'s app-lock guard test. (The
/// App-Lock-ON keystore path — mint + rekey + delete — needs a real Keystore +
/// `BiometricPrompt` and is covered on-device, not here.)
#[tokio::test]
async fn m0007_pends_under_app_lock_without_vault_key() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pref.json"), r#"{"schema_version":6}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)), // keyless: vault_seal unkeyed
        AppConfigStore::new(dir.path()).await,
    );
    state.app_lock_enabled.store(true, atomic::Ordering::SeqCst);

    run_app_migrations(&state).await;

    // Pending ⇒ schema stays at 6 (the engine returns without bumping).
    assert_eq!(
        state.app_config.get_pref().schema_version,
        6,
        "m0007 must defer until the vault key is injected at app_unlock"
    );
}

/// REGRESSION (desktop half-migrated recovery). If a prior `m0005` run sealed
/// the behavior half (step 7) but crashed before bumping the pref schema (step
/// 8), the next run must NOT re-derive display prefs from `app.json`. On desktop
/// the post-split `app.json` is a plaintext `BehaviorConfig` (passthrough seal)
/// that parses as a degenerate V4 — display fields defaulted, `schema_version`
/// defaulted to 4 — so an unconditional step-5 overwrite would silently clobber
/// the user's `locale`/`theme_mode` held in `pref.json`. `pref.json` (already
/// written) is authoritative for the display half; the `is_envelope` recovery
/// signal does NOT catch this on desktop because the passthrough file is
/// plaintext, not an envelope.
#[tokio::test]
async fn m0005_preserves_display_prefs_on_desktop_half_migrated_recovery() {
    let dir = tempfile::tempdir().unwrap();
    // Desktop store (master key None ⇒ passthrough-plaintext seal). Simulate a
    // prior m0005 run that sealed behavior into app.json then crashed before the
    // schema bump (pref.json still at schema 4, m0004_verbose's result).
    let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
    store
        .save_app_behavior(r#"{"lock_mode":{"idle":120},"autosync":false}"#.as_bytes())
        .await
        .unwrap();
    // pref.json holds the user's REAL display prefs at the pre-bump schema 4.
    // (Carries `secure_screen` from main's schema-4 shape — V4 must tolerate
    // it; serde ignores unknown keys, the deprecated field doesn't reach
    // PrefConfig.)
    std::fs::write(
        dir.path().join("pref.json"),
        r#"{"schema_version":4,"secure_screen":true,"locale":"zh-CN","theme_mode":"dark"}"#,
    )
    .unwrap();
    let state = build_state(store, AppConfigStore::new(dir.path()).await);
    // `new()` loaded pref.json's real display prefs (not the app.json defaults).
    assert_eq!(state.app_config.get_pref().locale.as_deref(), Some("zh-CN"));

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    // Display prefs survive the recovery — the load-bearing assertion.
    assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    assert_eq!(reloaded.theme_mode.as_deref(), Some("dark"));
    // Behavior is still recovered from the sealed (plaintext-passthrough) slot.
    assert_eq!(reloaded.lock_mode, LockMode::Idle(120));
    assert!(!reloaded.autosync);
}

/// (REGRESSION) A missing `app.json` (fresh install / post-reset) is a no-op:
/// the engine's raw peek returns `None` ⇒ skip all migrations, write no file,
/// error nowhere. An earlier engine design seeded the gate with `unwrap_or(1)`,
/// which would have made m0002 read a non-existent file every startup — this
/// pins the corrected `unwrap_or(APP_CONFIG_SCHEMA_VERSION)`.
#[tokio::test]
async fn missing_app_json_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    // No repo.json and no app.json — a true fresh install.
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    // No app.json was written (nothing to migrate from).
    assert!(
        !dir.path().join("app.json").exists(),
        "fresh install must not write an app.json during migrations"
    );
    // The cache stays at the default (target schema).
    assert_eq!(
        state.app_config.get().schema_version,
        APP_CONFIG_SCHEMA_VERSION,
    );
}

/// A corrupt `app.json` peeks as `Corrupt` (R074: present-but-unparseable ⇒
/// never silently `Absent`) and halts the chain — the app boots on defaults,
/// matching `new()`, without panicking or wiping prefs by skipping to target.
#[tokio::test]
async fn corrupt_app_json_halts_on_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.json"), "{not json").unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    // The corrupt file is left in place (the chain halted, not rewrote it); the
    // next settings save overwrites it. The cache holds the default.
    assert_eq!(
        state.app_config.get().schema_version,
        APP_CONFIG_SCHEMA_VERSION,
    );
}

/// m0002 seeds the `AppState` security caches (`lock_mode`, `clipboard_clear_secs`)
/// directly from its just-written V2 snapshot (via `apply_security_caches_from`),
/// NOT via a mid-chain cache reload. Pins that seeding so a refactor cannot
/// silently leave the caches stale until the next launch.
#[tokio::test]
async fn m0002_seeds_security_caches_from_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("repo.json"), OLD_REPO_JSON).unwrap();
    std::fs::write(dir.path().join("app.json"), r#"{"schema_version":1}"#).unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    // OLD_REPO_JSON pins lock_mode=Idle(300) and clipboard_clear_secs=180; the
    // AppState caches must reflect those migrated values immediately.
    assert_eq!(
        *state.lock_mode.lock().unwrap(),
        LockMode::Idle(300),
        "m0002 must seed the lock_mode cache from its snapshot"
    );
    assert_eq!(
        *state.clipboard_clear_secs.lock().unwrap(),
        180,
        "m0002 must seed the clipboard_clear_secs cache from its snapshot"
    );
}

/// (load-bearing) After the chain runs, the IN-MEMORY cache must reflect the
/// migrated values — the end-of-chain reload is what feeds the post-migration
/// `effective_log_filter` read in `init_state` / `app_unlock`. The other tests
/// re-read a fresh store off disk, which would hide a stale cache; this reads
/// `state.app_config` directly.
#[tokio::test]
async fn post_migration_cache_reflects_migrated_values() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":3,"log_level":"debug"}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    // The in-memory cache (not a fresh store) sees m0004's carried deadline.
    let deadline = state
        .app_config
        .get()
        .verbose_until
        .expect("m0004 carried debug into verbose_until; cache must see it");
    assert!(deadline > now_unix());
    assert!(deadline <= now_unix() + VERBOSE_WINDOW_SECS);
    assert_eq!(
        state.app_config.effective_log_filter(),
        log::LevelFilter::Debug,
        "the post-migration runtime gate reads the migrated deadline off the cache"
    );
}

/// (REGRESSION) A V1 `app.json` that peeks fine (`schema_version` parses) but
/// fails full V1 deserialization — a wrong-type field like `"locale":123` —
/// must fall back to a V1 default whose `secure_screen` matches the serde
/// default (`true`), so m0003 maps it to `None` (Sensitive), NOT `Off`. An
/// earlier revision derived `AppConfigV1`'s `Default` (bool ⇒ `false`), which
/// silently downgraded screen-capture protection to `Off` on a corrupt file.
#[tokio::test]
async fn corrupt_v1_file_keeps_sensitive_screen_default() {
    let dir = tempfile::tempdir().unwrap();
    // schema_version parses (peek ⇒ 1) but locale:123 is the wrong type, so the
    // full V1 read fails and m0002 falls back to AppConfigV1::default().
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":1,"locale":123}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(
        reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION,
        "m0002 bumps schema past the corrupt V1"
    );
    assert!(
        reloaded.secure_screen_mode.is_none(),
        "corrupt V1 must keep the Sensitive default, not downgrade to Off"
    );
}

/// (REGRESSION) A schema-2 `app.json` that peeks fine (`schema_version` parses)
/// but fails full V2 deserialization — e.g. a wrong-type `lock_mode` — must heal
/// to target, not strand the schema and warn-loop every launch. m0003 mirrors
/// m0002's V1 fallback: it uses `AppConfigV2::default()`, the chain advances,
/// and the corrupt field reverts to its default (the unparseable value is
/// unrecoverable anyway). Before the fix, m0003/m0004 hard-failed (`?`) and the
/// engine retried the same failing read on every startup.
#[tokio::test]
async fn corrupt_v2_file_heals_to_target_via_fallback() {
    let dir = tempfile::tempdir().unwrap();
    // schema_version parses (peek ⇒ 2) but lock_mode is the wrong type, so the
    // full V2 read in m0003 fails and m0003 falls back to AppConfigV2::default().
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":2,"lock_mode":{"idle":"not-a-number"}}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(
        reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION,
        "corrupt V2 must heal to target, not strand the schema"
    );
    assert_eq!(
        reloaded.lock_mode,
        LockMode::default(),
        "corrupt lock_mode reverts to the default via the V2 fallback"
    );
}

/// (backward-compat) A main-shipped schema-4 `app.json` that still carries the
/// deprecated `secure_screen: bool` (main's m0003 preserves it) and/or
/// `log_level` (cleared by main's m0004 only when the conversion fired) must
/// upgrade cleanly: V4 reads the file without `deny_unknown_fields`, serde
/// ignores the deprecated keys, and the resulting `PrefConfig` has NEITHER
/// (they were never on `PrefConfig` to begin with). Pins the no-`deny_unknown_fields`
/// invariant on V4 — without it, V4 would reject the file and m0005 would fall
/// through to the "unparseable" branch, dropping the user's locale/theme.
#[tokio::test]
async fn v4_reads_main_shiped_schema_4_with_deprecated_keys() {
    let dir = tempfile::tempdir().unwrap();
    // Schema-4 file carrying both deprecated keys (main's m0003 left
    // secure_screen in place; main's m0004 leaves log_level when not "debug").
    std::fs::write(
        dir.path().join("app.json"),
        r#"{
            "schema_version":4,
            "secure_screen":true,
            "log_level":"info",
            "secure_screen_mode":"off",
            "locale":"zh-CN",
            "theme_mode":"dark",
            "lock_mode":{"idle":120},
            "autosync":false
        }"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    run_app_migrations(&state).await;

    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    // The user's prefs survive the upgrade — the load-bearing assertion.
    assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    assert_eq!(reloaded.theme_mode.as_deref(), Some("dark"));
    assert_eq!(reloaded.lock_mode, LockMode::Idle(120));
    assert!(!reloaded.autosync);
    assert_eq!(
        reloaded.secure_screen_mode,
        Some(SecureScreenMode::Off),
        "the persisted mode survives; the deprecated bool is dropped at V4"
    );
    // R074: pref.json is collapsed into the sealed merged app.json (desktop:
    // plaintext AppConfig). The deprecated `secure_screen` bool + `log_level`
    // must NOT survive into it (matched with the trailing `":` so the modern
    // `secure_screen_mode` field — which DOES survive — is not a false positive).
    let app_on_disk = std::fs::read_to_string(dir.path().join("app.json")).unwrap();
    assert!(
        !app_on_disk.contains("\"secure_screen\":") && !app_on_disk.contains("log_level"),
        "merged app config drops the deprecated secure_screen bool + log_level; got: {app_on_disk}",
    );
}

/// (half-migrated behavior load) The REAL load-bearing path for a half-migrated
/// behavior cache is `reload_behavior` (called by `init_state` / `app_unlock`),
/// NOT the engine's end-of-chain `reload()` — that only runs on a COMPLETED
/// chain, and m0005 returns `Pending` before it reaches the reload. When m0005
/// has written `pref.json` (preserving schema 4) but the sealed behavior write is
/// deferred, `app.json` is still the plaintext single-file V4; `new()` leaves the
/// behavior cache at default (pref.json already exists), and `reload_behavior`
/// must lift the behavior fields off the plaintext file — parsed as
/// `BehaviorConfig` via field overlap (no `deny_unknown_fields`). Pins that path
/// directly, instead of the engine-reload path the old test claimed to exercise.
#[tokio::test]
async fn reload_behavior_loads_half_migrated_plaintext_app_json() {
    let dir = tempfile::tempdir().unwrap();
    // Half-migrated state: pref.json already split off (schema preserved at 4),
    // app.json still the plaintext single-file V4 (sealed write deferred).
    std::fs::write(
        dir.path().join("pref.json"),
        r#"{"schema_version":4,"locale":"zh-CN"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"schema_version":4,"lock_mode":{"idle":120},"autosync":false}"#,
    )
    .unwrap();
    let state = build_state(
        Arc::new(Store::new(dir.path().to_path_buf(), None)),
        AppConfigStore::new(dir.path()).await,
    );

    // new() saw pref.json exist ⇒ behavior cache starts at default (not lifted
    // off the plaintext app.json).
    let before = state.app_config.get_behavior();
    assert_eq!(before.lock_mode, LockMode::default());
    assert!(
        before.autosync,
        "new() defaults autosync true when pref.json exists"
    );

    // reload_behavior (the init_state/app_unlock path) lifts behavior off the
    // still-plaintext V4 app.json.
    state.app_config.reload_behavior().await.ok();

    let behavior = state.app_config.get_behavior();
    assert_eq!(
        behavior.lock_mode,
        LockMode::Idle(120),
        "reload_behavior must lift lock_mode off the plaintext V4 app.json"
    );
    assert!(
        !behavior.autosync,
        "reload_behavior must lift autosync off the plaintext V4 app.json"
    );
}

// ---------------------------------------------------------------------------
// R074 — m0008 collapse + the 3-state PeekOutcome gate
// ---------------------------------------------------------------------------

/// Seed a schema-7 two-file state (plaintext `pref.json` display + the sealed
/// behavior slot) so `m0008` is the only migration that runs. Desktop passthrough
/// ⇒ the behavior slot is plaintext `BehaviorConfig` JSON.
async fn seed_schema_7(dir: &Path, store: &Arc<Store>, pref_json: &str, behavior_json: &str) {
    std::fs::write(dir.join("pref.json"), pref_json).unwrap();
    store
        .save_app_behavior(behavior_json.as_bytes())
        .await
        .unwrap();
}

/// m0008 (desktop): pref.json(7) + the sealed behavior slot collapse into a
/// single sealed merged `app.json`(8); `pref.json` is deleted; every value
/// (display + behavior + cadence) survives.
#[tokio::test]
async fn m0008_collapses_pref_and_behavior_into_sealed_app_json() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
    seed_schema_7(
        dir.path(),
        &store,
        r#"{"schema_version":7,"locale":"zh-CN","theme_mode":"dark","background_sync":"1h"}"#,
        r#"{"lock_mode":{"idle":120},"autosync":false,"gate_idle":"off"}"#,
    )
    .await;
    let state = build_state(Arc::clone(&store), AppConfigStore::new(dir.path()).await);

    run_app_migrations(&state).await;

    // pref.json is gone — the load-bearing "zero plaintext" assertion.
    assert!(
        !dir.path().join("pref.json").exists(),
        "m0008 must delete pref.json"
    );
    // The merged app.json carries schema 8 + every value from both halves.
    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    assert_eq!(reloaded.theme_mode.as_deref(), Some("dark"));
    assert_eq!(reloaded.background_sync, BackgroundSyncCadence::Hours1);
    assert_eq!(reloaded.lock_mode, LockMode::Idle(120));
    assert!(!reloaded.autosync);
}

/// m0008 half-migrated recovery: a prior run wrote the merged `app.json`(8) but
/// crashed before deleting `pref.json`(7). Re-run detects the already-merged
/// file (typed dispatch), deletes the stale `pref.json`, and does NOT re-seal —
/// the merged file's values win, not the stale pref.json's.
#[tokio::test]
async fn m0008_half_migrated_recovery_deletes_stale_pref_json() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
    // The merged app.json@8 a prior m0008 run wrote.
    store
        .save_app_config(
            r#"{"schema_version":8,"locale":"zh-CN","lock_mode":{"idle":120}}"#.as_bytes(),
        )
        .await
        .unwrap();
    // Stale pref.json@7 left by the crash (carries a DIFFERENT locale to prove
    // it is not re-merged over the authoritative merged file).
    std::fs::write(
        dir.path().join("pref.json"),
        r#"{"schema_version":7,"locale":"stale"}"#,
    )
    .unwrap();
    let state = build_state(Arc::clone(&store), AppConfigStore::new(dir.path()).await);

    run_app_migrations(&state).await;

    assert!(
        !dir.path().join("pref.json").exists(),
        "the stale pref.json is deleted on recovery"
    );
    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(
        reloaded.locale.as_deref(),
        Some("zh-CN"),
        "the merged file is authoritative, not the stale pref.json"
    );
}

/// m0008 with no behavior slot (`NO_IDENTITY`): `pref.json` + default behavior
/// collapse into the merged file; `pref.json` is deleted.
#[tokio::test]
async fn m0008_merges_pref_with_default_behavior_when_no_slot() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
    std::fs::write(
        dir.path().join("pref.json"),
        r#"{"schema_version":7,"locale":"en"}"#,
    )
    .unwrap();
    // No app.json behavior slot.
    let state = build_state(Arc::clone(&store), AppConfigStore::new(dir.path()).await);

    run_app_migrations(&state).await;

    assert!(!dir.path().join("pref.json").exists());
    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(reloaded.locale.as_deref(), Some("en"));
    // Behavior defaulted (no slot to merge from).
    assert_eq!(reloaded.lock_mode, LockMode::default());
    assert!(
        reloaded.autosync,
        "autosync defaults true with no behavior slot"
    );
}

/// `PeekOutcome::Absent`: no config files at all ⇒ `Absent` ⇒ the chain skips.
#[tokio::test]
async fn peek_absent_when_no_config_files() {
    let dir = tempfile::tempdir().unwrap();
    let ac = AppConfigStore::new(dir.path()).await;
    ac.set_store(Arc::new(Store::new(dir.path().to_path_buf(), None)));
    assert_eq!(ac.peek_schema_version().await, PeekOutcome::Absent);
}

/// `PeekOutcome::Version` (old world): `pref.json` present ⇒ plaintext read.
#[tokio::test]
async fn peek_version_from_pref_json() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pref.json"), r#"{"schema_version":7}"#).unwrap();
    let ac = AppConfigStore::new(dir.path()).await;
    ac.set_store(Arc::new(Store::new(dir.path().to_path_buf(), None)));
    assert_eq!(ac.peek_schema_version().await, PeekOutcome::Version(7));
}

/// `PeekOutcome::Version` (new world): the sealed merged `app.json` ⇒ `Version(8)`.
#[tokio::test]
async fn peek_version_from_sealed_merged_app_json() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
    store
        .save_app_config(r#"{"schema_version":8}"#.as_bytes())
        .await
        .unwrap();
    let ac = AppConfigStore::new(dir.path()).await;
    ac.set_store(store);
    assert_eq!(ac.peek_schema_version().await, PeekOutcome::Version(8));
}

/// `PeekOutcome::Corrupt`: `app.json` present but unparseable ⇒ `Corrupt` (never
/// silently `Absent`). The chain halts + logs rather than wiping prefs.
#[tokio::test]
async fn peek_corrupt_when_app_json_unparseable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.json"), "{not json").unwrap();
    let ac = AppConfigStore::new(dir.path()).await;
    ac.set_store(Arc::new(Store::new(dir.path().to_path_buf(), None)));
    assert_eq!(ac.peek_schema_version().await, PeekOutcome::Corrupt);
}

/// m0008 under a REAL master key (not desktop passthrough): the sealed behavior
/// slot + `pref.json` collapse into a sealed merged envelope, readable back under
/// the same key. The other m0008 tests use `Store(None)` passthrough; this pins
/// the keyed-seal integration through the migration (the seal layer itself is
/// unit-tested in rustpass).
#[tokio::test]
async fn m0008_collapses_under_real_master_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = rustpass::seal::generate_master_key().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), Some(key)));
    seed_schema_7(
        dir.path(),
        &store,
        r#"{"schema_version":7,"locale":"zh-CN","background_sync":"6h"}"#,
        r#"{"lock_mode":{"idle":300},"autosync":false}"#,
    )
    .await;
    let state = build_state(Arc::clone(&store), AppConfigStore::new(dir.path()).await);

    run_app_migrations(&state).await;

    assert!(!dir.path().join("pref.json").exists(), "pref.json deleted");
    // The merged app.json is a real seal envelope now (sealed under the key).
    let on_disk = std::fs::read(dir.path().join("app.json")).unwrap();
    assert!(
        rustpass::seal::is_envelope(&on_disk),
        "merged app.json is a sealed envelope under the real key"
    );
    // Readable back under the SAME key, with every value preserved.
    let reloaded = reload_at(dir.path(), &state.store).await;
    assert_eq!(reloaded.schema_version, APP_CONFIG_SCHEMA_VERSION);
    assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    assert_eq!(reloaded.background_sync, BackgroundSyncCadence::Hours6);
    assert_eq!(reloaded.lock_mode, LockMode::Idle(300));
    assert!(!reloaded.autosync);
}

/// m0008 propagates a sealed-write failure as `Err` (the `?` contract): a dir at
/// `app.tmp` blocks `save_atomic`'s temp write, schema stays below target, and a
/// retry after clearing the block completes. Mirrors m0002's save-failure tests.
#[tokio::test]
async fn m0008_write_failure_leaves_schema_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
    seed_schema_7(
        dir.path(),
        &store,
        r#"{"schema_version":7,"locale":"en"}"#,
        r#"{"lock_mode":{"idle":120}}"#,
    )
    .await;
    let state = build_state(Arc::clone(&store), AppConfigStore::new(dir.path()).await);

    // Block save_atomic's temp write (`app.tmp` is a dir ⇒ fs::write fails on
    // every platform). m0008 must propagate that Err instead of marking done.
    std::fs::create_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        reload_at(dir.path(), &state.store).await.schema_version,
        7,
        "a failed merged write must not advance the schema"
    );
    assert!(
        dir.path().join("pref.json").exists(),
        "pref.json survives a failed merged write"
    );

    // Clear the block + retry — m0008 completes to target.
    std::fs::remove_dir(dir.path().join("app.tmp")).unwrap();
    run_app_migrations(&state).await;
    assert_eq!(
        reload_at(dir.path(), &state.store).await.schema_version,
        APP_CONFIG_SCHEMA_VERSION,
    );
    assert!(!dir.path().join("pref.json").exists());
}

/// m0008 propagates `Err` (not delete+Done) when `app.json` unseals OK but
/// parses as neither `AppConfig@8` nor `BehaviorConfig` — AEAD-valid garbage
/// (a sealing-code bug or an incompatible downgrade). `pref.json` survives for
/// recovery and the app stays in the old-world layout rather than forcing
/// re-setup (the merged `app.json` was never written, so a `Done` would peek
/// `Corrupt` next launch and wipe the display prefs). Desktop passthrough:
/// "unseals OK" = the file reads; invalid JSON fails both parses.
#[tokio::test]
async fn m0008_unparseable_app_json_propagates_err_and_keeps_pref() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
    seed_schema_7(
        dir.path(),
        &store,
        r#"{"schema_version":7,"locale":"zh-CN"}"#,
        r#"{"lock_mode":{"idle":120}}"#,
    )
    .await;
    // Overwrite app.json with AEAD-valid-but-unparseable content (desktop
    // passthrough ⇒ invalid JSON fails both the AppConfig and BehaviorConfig
    // parses).
    std::fs::write(dir.path().join("app.json"), "{not json").unwrap();
    let state = build_state(Arc::clone(&store), AppConfigStore::new(dir.path()).await);

    run_app_migrations(&state).await;
    // m0008 surfaced Err ⇒ schema did not advance, pref.json survives.
    assert_eq!(
        reload_at(dir.path(), &state.store).await.schema_version,
        7,
        "an unparseable app.json must not advance the schema"
    );
    assert!(
        dir.path().join("pref.json").exists(),
        "pref.json survives an unparseable app.json (no silent wipe)"
    );
}

/// m0008 propagates a tampered sealed behavior slot as `Err`: the engine
/// retries, schema stays below target. Never silent-defaults behavior prefs.
#[tokio::test]
async fn m0008_corrupt_sealed_propagates_err() {
    let dir = tempfile::tempdir().unwrap();
    let key = rustpass::seal::generate_master_key().unwrap();
    let store = Arc::new(Store::new(dir.path().to_path_buf(), Some(key)));
    seed_schema_7(
        dir.path(),
        &store,
        r#"{"schema_version":7,"locale":"en"}"#,
        r#"{"lock_mode":{"idle":120}}"#,
    )
    .await;
    // Tamper the sealed behavior slot (flip a byte in the AEAD tag/ciphertext).
    let mut sealed = std::fs::read(dir.path().join("app.json")).unwrap();
    if let Some(b) = sealed.last_mut() {
        *b ^= 0xff;
    }
    std::fs::write(dir.path().join("app.json"), &sealed).unwrap();
    let state = build_state(Arc::clone(&store), AppConfigStore::new(dir.path()).await);

    run_app_migrations(&state).await;

    // m0008 returned Err on the tampered slot ⇒ schema stayed at 7, no crash.
    assert_eq!(
        state.app_config.get_pref().schema_version,
        7,
        "a tampered behavior slot must not advance the schema (Err propagates)"
    );
    assert!(
        dir.path().join("pref.json").exists(),
        "pref.json survives a tampered-slot read"
    );
}
