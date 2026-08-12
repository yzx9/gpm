// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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

mod background_sync;
mod clipboard_clear;
mod gate_idle;
mod gate_resume;
mod git_commands;
mod lock_state;
mod locked_writes;
mod migrations;
mod read_commands;
mod seal_migrate;
mod setup_flow;

use std::io::Write;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};

use age::Encryptor;
use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};
use git2::build::RepoBuilder;
use git2::{IndexAddOption, Repository, Signature};
use rustpass::{GitAuth, LockMode, Store};
use tauri::App;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::AppState;
use crate::app_config::AppConfigStore;
use crate::identity::IdleTimer;
use crate::registry::RepoId;

/// 1-permit serializer guarding identity-crypto round-trips in this test binary.
///
/// Concurrent age-scrypt identity round-trips intermittently fail with
/// `WRONG_PASSPHRASE` on a correct passphrase — correct single-threaded,
/// intermittent under concurrency, with byte-identical input. That signature
/// (a data race / UB fingerprint, not a codegen miscompilation; root cause
/// unconfirmed) is documented on the authoritative gate in
/// `rustpass::test_crypto_gate`. A 1-permit serializer forces the
/// provably-correct single-threaded path. [`make_unlocked_state`] holds it for
/// its whole body, so every src-tauri crypto test routed through that helper is
/// serialized. One semaphore per binary — the failure is intra-binary. This is
/// a test-only stopgap; mirrors `rustpass::tests::common`.
static CRYPTO_SEM: Semaphore = Semaphore::const_new(1);

/// Acquire the per-binary crypto serializer. Hold the returned permit for the
/// whole operation (e.g. `let _crypto = crypto_permit().await;`) so identity
/// crypto round-trips never run concurrently. See [`CRYPTO_SEM`].
pub(super) async fn crypto_permit() -> SemaphorePermit<'static> {
    CRYPTO_SEM.acquire().await.expect("crypto semaphore closed")
}

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
    let encryptor = Encryptor::with_recipients(std::iter::once(recipient_dyn)).unwrap();
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

    let repo = Repository::init(work_dir.path()).unwrap();
    let sig = Signature::new("test", "test@test.com", &git2::Time::new(0, 0)).unwrap();

    for (path, content) in entries {
        let file_path = work_dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file_path, encrypt_to_recipient(content, recipient_str)).unwrap();
    }

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    let mut builder = RepoBuilder::new();
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

/// The fixed id [`make_unlocked_state`] registers the test store under (mirrors
/// the production single-repo invariant — registry facade `Arc::ptr_eq`
/// `state.store`). Threaded commands that resolve `state.repo(id)` take this in
/// in-crate tests. The value is a valid 32-hex form so it passes the funnel's
/// `is_valid_form` gate.
pub(super) fn test_repo_id() -> RepoId {
    // 32 ascii hex chars — the canonical form `RepoId::generate` produces, and
    // what `AppState::repo` validates at the funnel.
    RepoId::from("0123456789abcdef0123456789abcdef")
}

/// Configure + unlock an **encrypted-identity** store backed by a temp repo
/// seeded with `entries`. Returns the live [`AppState`] plus the [`TestStore`]
/// guard that must outlive it. Most tests start here (an unlocked store is the
/// precondition for observing lock transitions).
pub(super) async fn make_unlocked_state(entries: &[(&str, &[u8])]) -> (AppState, TestStore) {
    // Serialize the configure → set_passphrase → unlock crypto round-trip
    // against the age-scrypt concurrency failure (see CRYPTO_SEM). Held for the
    // whole body.
    let _crypto = crypto_permit().await;
    let (identity, recipient) = generate_test_keypair();
    let passphrase = "correct-horse-battery-staple".to_string();
    let bare_dir = create_bare_repo(entries, &recipient);
    let config_dir = tempfile::tempdir().unwrap();

    // No master key: tests use plaintext seal passthrough (desktop parity).
    let store = Arc::new(Store::new(config_dir.path().to_path_buf(), None));
    store
        .configure(
            bare_dir.path().to_str().unwrap(),
            &GitAuth::None,
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

    // Bind the AppConfigStore to the store (mirrors init_state) so gate-idle and
    // other behavior-config reads/writes flow through the seal in tests.
    let app_config = AppConfigStore::new(config_dir.path()).await;
    app_config.set_store(Arc::clone(&store));

    // Keep bare_dir alive (returned in TestStore) so the store's `origin` remote
    // stays valid for tests that drive real sync/push; `configure` already cloned
    // it into the config dir's repo.
    let state = AppState {
        store,
        registry: crate::registry::RepoRegistry::empty(),
        app_config: Arc::new(app_config),
        app_handle: None,
        lock_timer: IdleTimer::new(),
        pending_identity: Mutex::new(None),
        lock_mode: Mutex::new(LockMode::default()),
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
    };
    // Mirror the production single-repo invariant: register the test store under
    // [`test_repo_id`] so threaded commands resolving `state.repo(id)` work (the
    // registry facade is `Arc::ptr_eq` to `state.store`, exactly like a real
    // single-repo app after init).
    let store_for_registry = Arc::clone(&state.store);
    state.registry.populate(
        [test_repo_id()],
        Some(test_repo_id()),
        move |_| Arc::clone(&store_for_registry),
    );
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
pub(super) fn mock_app(state: AppState) -> App<MockRuntime> {
    mock_builder()
        // Register clipboard-notify so the armed clear task's `dismiss()` call
        // resolves against the desktop inert stub instead of panicking on a
        // missing managed state.
        .plugin(tauri_plugin_clipboard_notify::init())
        .manage(state)
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app")
}

/// `AppState::repo` is the single funnel every threaded command resolves
/// through. Pin its three outcomes: a well-formed but unregistered id yields
/// `UnknownRepository` (carrying only the opaque id); a malformed id (wrong
/// length/charset) yields `ConfigError "invalid repository id"` and does NOT
/// interpolate the value; the registered id resolves to the registry facade,
/// which (for one repo) is `Arc::ptr_eq` to `state.store`.
#[tokio::test]
async fn repo_funnel_classifies_unknown_and_malformed_ids() {
    let (state, _guard) = make_unlocked_state(&[]).await;

    // Well-formed (32 hex) but unregistered → UnknownRepository, opaque id only.
    let unknown = RepoId::from("ffffffffffffffffffffffffffffffff");
    let err = state.repo(&unknown).unwrap_err();
    assert_eq!(err.code, "UNKNOWN_REPOSITORY", "{err}");
    assert!(err.message.contains(&unknown.to_string()), "{err}");

    // Malformed (wrong length/charset) → ConfigError, bounded fixed message.
    let malformed = RepoId::from("not-a-real-id");
    let err = state.repo(&malformed).unwrap_err();
    assert_eq!(err.code, "CONFIG_ERROR", "{err}");
    assert_eq!(err.message, "invalid repository id");
    assert!(!err.message.contains("not-a-real-id"));

    // The registered id resolves to the registry facade == state.store today.
    let store = state.repo(&test_repo_id()).expect("registered id resolves");
    assert!(Arc::ptr_eq(&store, &state.store), "facade must be the device store");
}
