// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests for the one-shot post-unlock legacy-envelope migrate
//! (`applock::run_seal_migrate_once`).
//!
//! The legacy-conversion mechanics themselves are covered in `rustpass` (config
//! tests). These cover the app-layer glue those tests can't reach: the helper
//! actually drives `Store::migrate_seal`, the CAS state machine transitions
//! Pending → Done, and a second call is a no-op. They build a keyed `AppState`
//! directly (no keystore mock needed).

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rustpass::Store;

use crate::AppState;
use crate::app_config::AppConfigStore;
use crate::applock::run_seal_migrate_once;
use crate::identity::IdleTimer;

/// Build a minimal `AppState` backed by a keyed `Store` in a temp config dir,
/// with `seal_migrate_state` Pending.
async fn keyed_state(dir: &Path) -> AppState {
    let key = rustpass::seal::generate_master_key().unwrap();
    let store = Arc::new(Store::new(dir.to_path_buf(), Some(key)));
    AppState {
        store,
        registry: crate::registry::RepoRegistry::empty(),
        app_config: Arc::new(AppConfigStore::new(dir).await),
        app_handle: None,
        lock_timer: IdleTimer::new(),
        pending_identity: Mutex::new(None),
        lock_mode: Mutex::new(rustpass::LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        clipboard_clear_handle: Mutex::new(None),
        clipboard_clear_generation: Arc::new(AtomicU64::new(0)),
        app_lock_enabled: AtomicBool::new(false),
        app_locked: Arc::new(AtomicBool::new(false)),
        gate_idle_timer: IdleTimer::new(),
        last_activity_at: Mutex::new(std::time::Instant::now()),
        cached_entry: Arc::new(Mutex::new(None)),
        entry_cache_timer: IdleTimer::new(),
        identity_coupled: AtomicBool::new(false),
        seal_migrate_state: AtomicU8::new(0),
        backend_resolve_state: AtomicU8::new(0),
        active_cancel_slot: Arc::new(Mutex::new(None)),
        verbose_timer: Mutex::new(None),
        verbose_generation: Arc::new(AtomicU64::new(0)),
    }
}

#[tokio::test]
async fn run_seal_migrate_once_wraps_plaintext_and_marks_done() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    let state = keyed_state(dir.path()).await;

    // Plant a plaintext repo.json (pre-seal shape).
    let plaintext = br#"{"url":"https://x/repo","pat":"secret"}"#;
    let repo_json = dir.path().join("repo.json");
    std::fs::write(&repo_json, plaintext).unwrap();
    assert!(
        !rustpass::seal::is_envelope(&std::fs::read(&repo_json).unwrap()),
        "precondition: plaintext"
    );

    // First call: claims Pending → drives migrate_seal → plaintext wrapped → Done.
    run_seal_migrate_once(&state).await;
    assert_eq!(
        state.seal_migrate_state.load(Ordering::SeqCst),
        2,
        "state should be Done after a successful migrate"
    );
    let raw = std::fs::read(&repo_json).unwrap();
    assert!(
        rustpass::seal::is_envelope(&raw),
        "plaintext should now be sealed"
    );
    assert!(
        raw.starts_with(b"GPMSEL1"),
        "new seals use the current magic"
    );

    // Second call: Done ⇒ CAS fails ⇒ no-op (idempotent regardless).
    run_seal_migrate_once(&state).await;
    assert_eq!(state.seal_migrate_state.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn run_seal_migrate_once_marks_done_on_empty_dir() {
    // No files ⇒ migrate_seal Ok on every (missing) file ⇒ Done. Proves the
    // helper invokes migrate_seal and transitions even with nothing to convert.
    let dir = tempfile::tempdir().unwrap();
    let state = keyed_state(dir.path()).await;

    run_seal_migrate_once(&state).await;
    assert_eq!(state.seal_migrate_state.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn run_seal_migrate_once_snaps_back_to_pending_on_failure() {
    // A corrupt, too-short legacy-magic blob: is_envelope is true (GPMATR1
    // prefix) but unseal returns SealTampered (header < MAGIC + key_id + nonce),
    // which is NOT the soft-skipped SEAL_KEY_UNAVAILABLE — it propagates out of
    // migrate_seal into the Err(_) arm. The state must snap to Pending so the
    // next app_unlock retries, and the file must be left untouched (a failed
    // re-wrap never overwrites prior bytes). Pins the recovery contract.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    let state = keyed_state(dir.path()).await;

    let corrupt_legacy = b"GPMATR1\xff\x00\x00"; // magic + 3 bytes < 20-byte header
    let repo_json = dir.path().join("repo.json");
    std::fs::write(&repo_json, corrupt_legacy).unwrap();
    assert!(
        rustpass::seal::is_envelope(corrupt_legacy),
        "precondition: recognized as an envelope"
    );

    run_seal_migrate_once(&state).await;
    assert_eq!(
        state.seal_migrate_state.load(Ordering::SeqCst),
        0,
        "Err must snap state to Pending so the next unlock retries"
    );
    assert_eq!(
        std::fs::read(&repo_json).unwrap(),
        corrupt_legacy,
        "failed re-wrap leaves the file untouched"
    );
}
