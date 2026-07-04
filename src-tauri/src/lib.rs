// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! GPM — age-only gopass password manager client built with Tauri v2.

#![warn(
    trivial_casts,
    trivial_numeric_casts,
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications,
    clippy::dbg_macro,
    clippy::indexing_slicing,
    clippy::pedantic
)]

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use base64::Engine;
use rustpass::Store;
use tauri::Manager;
use tauri_plugin_secure_keystore::SecureKeystoreExt;
use tokio::task::JoinHandle;

mod app_config;
mod applock;
mod authenticity;
mod biometric;
mod clipboard;
mod config;
mod generator;
mod git;
mod identity;
mod read;
mod setup;
mod write;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// Application state shared across all Tauri commands.
pub(crate) struct AppState {
    pub(crate) store: Arc<Store>,
    /// Auto-lock timer handle (cancel-and-respawn pattern).
    pub(crate) lock_timer: Mutex<Option<JoinHandle<()>>>,
    /// Monotonic generation tag for the auto-lock timer. Bumped on every (re)arm; the spawned
    /// task captures its generation and self-disarms if a newer arm happened while it slept.
    /// Kills the spurious re-lock race where a stale timer wakes right after a fresh unlock
    /// — the modal auto-prompts, so such a re-lock would visibly re-show the overlay.
    pub(crate) lock_generation: Arc<AtomicU64>,
    /// Identity picked via the file picker, awaiting its passphrase before
    /// `complete_setup_from_file` saves it. Held only in memory (`Zeroizing` on
    /// drop); never persisted.
    pub(crate) pending_identity: Mutex<Option<setup::PendingIdentity>>,
    /// Cached effective auto-lock mode (refreshed on unlock + the `set_*`
    /// config commands via `identity::refresh_security_cache`) so the read/write
    /// hot paths branch on a cheap mutex read instead of decrypting `repo.json`
    /// per operation.
    pub(crate) lock_mode: Mutex<rustpass::LockMode>,
    /// Cached effective clipboard auto-clear seconds (same refresh contract).
    pub(crate) clipboard_clear_secs: Mutex<u64>,
    /// Whether the app-launch biometric gate is enabled (the at-rest master key
    /// is sealed in the biometric-gated Keystore). Probed at startup from the
    /// key's location and updated on enable/disable. Drives whether the frontend
    /// ever shows the app-lock overlay.
    pub(crate) app_lock_enabled: AtomicBool,
    /// Runtime app-lock state: `true` while the master key is NOT in memory —
    /// cold start with the gate on, or after a background wipe. Cleared by
    /// `applock::app_unlock`. Drives the frontend app-lock overlay (which
    /// suppresses the identity overlay while up, so the two never compete).
    pub(crate) app_locked: AtomicBool,
    /// Cancel token for the in-flight clone/pull (if any). Set by the
    /// `cancel_git` command; cleared by the owning command once the operation
    /// settles. `None` outside a user-initiated clone/pull.
    pub(crate) active_cancel_token: Mutex<Option<rustpass::CancelToken>>,
    /// App-shell (non-repo) preferences — primarily the screen-capture master
    /// toggle. Persists at `app.json`; survives `reset_config` (device pref).
    pub(crate) app_config: app_config::AppConfigStore,
}

// ---------------------------------------------------------------------------
// At-rest master key (Android Keystore)
// ---------------------------------------------------------------------------

/// Base64 engine for the master key crossing the Rust ↔ Android-plugin IPC.
pub(crate) const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// Decode a Base64 master key to 32 bytes, or `None` if malformed/wrong length.
pub(crate) fn decode_master_key(b64: &str) -> Option<[u8; 32]> {
    let bytes: Vec<u8> = B64.decode(b64).ok()?;
    bytes.try_into().ok()
}

/// Fetch the sealed at-rest master key, generating + sealing one on first run.
///
/// Returns `None` on desktop (no Keystore) or if the Keystore is unavailable /
/// errors — in which case at-rest encryption degrades to plaintext passthrough
/// (logged, non-fatal). A freshly generated key that cannot be sealed is
/// discarded (`None`) rather than used unpersisted, so it can never orphan
/// later envelopes behind a key the next run won't have.
fn master_key_from<R: tauri::Runtime>(
    ks: &tauri_plugin_secure_keystore::SecureKeystore<R>,
) -> Option<[u8; 32]> {
    if !ks.is_available().unwrap_or(false) {
        return None;
    }
    if let Some(b64) = ks.retrieve().unwrap_or(None) {
        return decode_master_key(&b64);
    }
    // No sealed key yet: generate + seal a fresh master key.
    let key = rustpass::atrest::generate_master_key().ok()?;
    // Seal before adopting — an unpersisted key would orphan future envelopes
    // on the next run.
    ks.store(&B64.encode(key)).ok()?;
    Some(key)
}

/// Resolve the at-rest master key + app-lock state at startup.
///
/// When a biometric-gated master key exists (the app-launch gate is on), the
/// key is deliberately NOT loaded here — it is injected after the app-unlock
/// biometric prompt — so `repo.json` stays unreadable until the user
/// authenticates. Otherwise the auth-free master key loads silently (the
/// pre-app-lock path). Returns `(master_key, app_lock_enabled)`.
fn startup_master_key<R: tauri::Runtime>(
    ks: &tauri_plugin_secure_keystore::SecureKeystore<R>,
) -> (Option<[u8; 32]>, bool) {
    if ks.has_stored_biometric().unwrap_or(false) {
        (None, true)
    } else {
        (master_key_from(ks), false)
    }
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Build the initial [`AppState`] during Tauri setup: resolve the config dir,
/// load (or defer, when app-lock is on) the at-rest master key, run the one-time
/// plaintext→envelope migration, and assemble the state. Extracted from
/// [`run`] so the entry point stays a thin builder.
///
/// # Panics
///
/// Panics if the config directory cannot be determined.
fn init_state<R: tauri::Runtime>(app: &tauri::App<R>) -> AppState {
    let config_dir = app
        .path()
        .app_config_dir()
        .expect("Cannot determine app config directory");

    // At-rest master key + app-lock state. When the biometric-gated master key
    // exists (app-lock on), the key is NOT loaded here — it is injected after
    // the app-unlock biometric prompt — so `repo.json` stays unreadable until
    // the user authenticates on launch/resume. Otherwise the auth-free master
    // key loads silently (the pre-app-lock path).
    let (master_key, app_lock_enabled) = startup_master_key(app.secure_keystore());
    // App-shell (non-repo) preferences — primarily the screen-capture master
    // toggle. Borrows `config_dir` before it is moved into `Store` below.
    let app_config = app_config::AppConfigStore::new(&config_dir);
    let store = Arc::new(Store::new(config_dir, master_key));
    // One-time migration of any pre-existing plaintext files into the at-rest
    // envelope (no-op on desktop / already-wrapped). Each file is wrapped
    // atomically with a roundtrip check, so a failure leaves plaintext intact —
    // logged, non-fatal. With app-lock on the master key is absent here, so
    // this is a no-op over the existing envelopes.
    if let Err(e) = tauri::async_runtime::block_on(store.migrate_at_rest()) {
        eprintln!("[gpm] at-rest migration failed: {e}");
    }

    AppState {
        store,
        app_config,
        lock_timer: Mutex::new(None),
        lock_generation: Arc::new(AtomicU64::new(0)),
        pending_identity: Mutex::new(None),
        // Defaults until the first unlock/set refreshes them from config;
        // pre-setup no op reads them.
        lock_mode: Mutex::new(rustpass::LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        app_lock_enabled: AtomicBool::new(app_lock_enabled),
        // Locked at startup iff the gate is on (master key not yet injected).
        app_locked: AtomicBool::new(app_lock_enabled),
        active_cancel_token: Mutex::new(None),
    }
}

/// Application entry point.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_safe_area::init())
        .plugin(tauri_plugin_biometric_keystore::init())
        .plugin(tauri_plugin_secure_keystore::init())
        .plugin(tauri_plugin_file_picker::init())
        .plugin(tauri_plugin_screen_secure::init())
        .setup(|app| {
            app.manage(init_state(app));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // setup / identity setup
            setup::get_auth_state,
            setup::is_configured,
            setup::is_repo_ready,
            setup::clone_repo,
            setup::generate_identity,
            setup::create_store,
            setup::list_recipients,
            setup::validate_identity,
            setup::complete_setup,
            setup::pick_identity_file,
            setup::verify_picked_identity,
            setup::verify_pasted_identity,
            setup::complete_setup_from_file,
            setup::clear_pending_identity,
            setup::setup,
            // identity: session, passphrase, ssh key
            identity::unlock,
            identity::lock,
            identity::set_passphrase,
            identity::change_passphrase,
            identity::generate_ssh_key,
            identity::get_ssh_public_key,
            identity::export_ssh_private_key,
            // read
            read::list_entries,
            read::search_entries,
            read::copy_password,
            read::show_password,
            clipboard::copy_generated_password,
            // generator
            generator::generate_password,
            generator::generate_password_batch,
            // write / sync
            write::pull_repo,
            write::sync_repo,
            git::cancel_git,
            write::push_repo,
            write::resolve_sync_divergence,
            write::discard_divergence,
            write::list_create_presets,
            write::lookup_template,
            write::preview_create,
            write::create_secret,
            write::create_from_preset_secret,
            write::delete_secret,
            write::edit_secret,
            // config
            config::get_config,
            config::set_commit_identity,
            config::set_lock_mode,
            config::set_view_clear_secs,
            config::set_clipboard_clear_secs,
            config::set_autosync,
            config::get_commit_identity_default,
            config::reset_config,
            // app config: screen-capture master toggle + platform availability
            app_config::get_app_config,
            app_config::set_secure_screen,
            app_config::screen_secure_available,
            // biometric
            biometric::is_biometric_available,
            biometric::is_biometric_unlock_enabled,
            biometric::enable_biometric_unlock,
            biometric::biometric_unlock,
            biometric::disable_biometric_unlock,
            // app-launch biometric gate (RFC 0028)
            applock::is_app_lock_available,
            applock::get_app_lock_state,
            applock::enable_biometric_app_lock,
            applock::disable_biometric_app_lock,
            applock::app_unlock,
            applock::app_lock,
            applock::enable_identity_auto_unlock,
            applock::disable_identity_auto_unlock,
            // repository authenticity
            authenticity::get_authenticity_state,
            authenticity::set_verification_mode,
            authenticity::get_authenticity_config,
            authenticity::add_trusted_key,
            authenticity::remove_trusted_key,
            authenticity::trust_head_signer,
            authenticity::trust_commit_signer,
            authenticity::ignore_commit_issue,
            authenticity::list_commit_signatures,
            authenticity::get_commit_signature,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod decode_master_key_tests {
    use super::*;

    #[test]
    fn valid_32_byte_key_roundtrips() {
        let key = rustpass::atrest::generate_master_key().unwrap();
        let b64 = B64.encode(key);
        assert_eq!(decode_master_key(&b64), Some(key));
    }

    #[test]
    fn wrong_length_returns_none() {
        // A 16-byte decode is the right shape but wrong length — must reject.
        assert_eq!(decode_master_key(&B64.encode([0u8; 16])), None);
    }

    #[test]
    fn malformed_base64_returns_none() {
        // Non-base64 characters ⇒ decode fails ⇒ None, no panic.
        assert_eq!(decode_master_key("!!!not-base64!!!"), None);
    }
}
