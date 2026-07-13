// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! In-crate integration tests for the Tauri command layer.
//!
//! These live *inside* the crate (not `src-tauri/tests/`) on purpose: every
//! command and `AppState` is `pub(crate)`, so only an in-crate `#[cfg(test)]`
//! module can construct an `AppState` and call the command cores directly. We
//! exercise the real command glue — the lock state machine, the conflict stash,
//! the setup pending-identity flow — that `rustpass`'s own tests can't reach
//! (they stop at the `Store` facade).
//!
//! Tauri commands are driven directly as async functions; the few that need an
//! `AppHandle` run against a headless [`MockRuntime`] app
//! (`tauri::test::mock_builder`) rather than a real webview.

mod clipboard_clear;
mod git_commands;
mod lock_state;
mod migrate;
mod read_commands;
mod seal_migrate;
mod setup_flow;

use std::io::Write;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};

use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};
use rustpass::Store;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};

use crate::AppState;

/// Generate a random x25519 keypair: `(identity_str, recipient_str)`.
pub(super) fn generate_test_keypair() -> (String, String) {
    let sk = Identity::generate();
    let pk = sk.to_public();
    (sk.to_string().expose_secret().to_string(), pk.to_string())
}

/// Encrypt `plaintext` to `recipient_str`, returning ciphertext bytes.
fn encrypt_to_recipient(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
    let recipient = Recipient::from_str(recipient_str).unwrap();
    let recipient_dyn: &dyn age::Recipient = &recipient;
    let encryptor = age::Encryptor::with_recipients(std::iter::once(recipient_dyn)).unwrap();
    let mut encrypted = Vec::new();
    let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
    writer.write_all(plaintext).unwrap();
    writer.finish().unwrap();
    encrypted
}

/// Build a bare git repo (acts as the remote) seeded with `entries` encrypted to
/// the test recipient. Mirrors `rustpass`'s `create_test_git_repo` but we only
/// need the bare side — `Store::configure` clones it into the config dir.
fn create_bare_repo(entries: &[(&str, &[u8])], recipient_str: &str) -> tempfile::TempDir {
    let work_dir = tempfile::tempdir().unwrap();
    let bare_dir = tempfile::tempdir().unwrap();

    let repo = git2::Repository::init(work_dir.path()).unwrap();
    let sig = git2::Signature::new("test", "test@test.com", &git2::Time::new(0, 0)).unwrap();

    for (path, content) in entries {
        let file_path = work_dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file_path, encrypt_to_recipient(content, recipient_str)).unwrap();
    }

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    let mut builder = git2::build::RepoBuilder::new();
    builder.bare(true);
    builder
        .clone(work_dir.path().to_str().unwrap(), bare_dir.path())
        .unwrap();

    drop(tree);
    drop(index);
    drop(repo);
    drop(work_dir);
    bare_dir
}

/// Owns the temp config dir backing a test [`AppState`] — keep it alive for the
/// test's lifetime or the store's files vanish mid-test.
pub(super) struct TestStore {
    #[allow(dead_code)]
    pub(super) config_dir: tempfile::TempDir,
    /// Kept alive so the store's `origin` remote stays valid for tests that drive
    /// real sync/push (e.g. a divergence conflict). Harmless for tests that don't.
    #[allow(dead_code)]
    pub(super) bare_dir: tempfile::TempDir,
}

/// Configure + unlock an **encrypted-identity** store backed by a temp repo
/// seeded with `entries`. Returns the live [`AppState`] plus the [`TestStore`]
/// guard that must outlive it. Most tests start here (an unlocked store is the
/// precondition for observing lock transitions).
pub(super) async fn make_unlocked_state(entries: &[(&str, &[u8])]) -> (AppState, TestStore) {
    let (identity, recipient) = generate_test_keypair();
    let passphrase = "correct-horse-battery-staple".to_string();
    let bare_dir = create_bare_repo(entries, &recipient);
    let config_dir = tempfile::tempdir().unwrap();

    // No master key: tests use plaintext seal passthrough (desktop parity).
    let store = Arc::new(Store::new(config_dir.path().to_path_buf(), None));
    store
        .configure(
            bare_dir.path().to_str().unwrap(),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");
    store
        .set_passphrase(&passphrase)
        .await
        .expect("set_passphrase should succeed");
    store
        .unlock(&passphrase)
        .await
        .expect("unlock should succeed");

    // Keep bare_dir alive (returned in TestStore) so the store's `origin` remote
    // stays valid for tests that drive real sync/push; `configure` already cloned
    // it into the config dir's repo.
    let state = AppState {
        store,
        app_config: crate::app_config::AppConfigStore::new(config_dir.path()),
        lock_timer: Mutex::new(None),
        lock_generation: Arc::new(AtomicU64::new(0)),
        pending_identity: Mutex::new(None),
        lock_mode: Mutex::new(rustpass::LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        clipboard_clear_handle: Mutex::new(None),
        clipboard_clear_generation: Arc::new(AtomicU64::new(0)),
        app_lock_enabled: AtomicBool::new(false),
        app_locked: AtomicBool::new(false),
        seal_migrate_state: AtomicU8::new(0),
        backend_resolve_state: AtomicU8::new(0),
        active_cancel_token: Mutex::new(None),
    };
    (
        state,
        TestStore {
            config_dir,
            bare_dir,
        },
    )
}

/// Build a headless [`MockRuntime`] app managing `state`, returning it for the
/// test to keep alive. Pull `app.state::<AppState>()` and `app.handle()` to
/// drive commands that take an `AppHandle`.
pub(super) fn mock_app(state: AppState) -> tauri::App<MockRuntime> {
    mock_builder()
        // Register clipboard-notify so the armed clear task's `dismiss()` call
        // resolves against the desktop inert stub instead of panicking on a
        // missing managed state.
        .plugin(tauri_plugin_clipboard_notify::init())
        .manage(state)
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app")
}
