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

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use base64::Engine;
use rustpass::Store;
use tauri::Manager;
use tauri_plugin_secure_keystore::SecureKeystoreExt;
use tokio::task::JoinHandle;

mod authenticity;
mod biometric;
mod config;
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
    /// A write that collided with a newer remote copy, awaiting the user's
    /// resolution. Wrapped in `Arc` so the auto-lock timer closure can clear it.
    /// See `write::PendingWrite` / `write::clear_pending`.
    pub(crate) pending_write: Arc<Mutex<Option<write::PendingWrite>>>,
}

// ---------------------------------------------------------------------------
// At-rest master key (Android Keystore)
// ---------------------------------------------------------------------------

/// Base64 engine for the master key crossing the Rust ↔ Android-plugin IPC.
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// Decode a Base64 master key to 32 bytes, or `None` if malformed/wrong length.
fn decode_master_key(b64: &str) -> Option<[u8; 32]> {
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

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Application entry point.
///
/// # Panics
///
/// Panics if the config directory cannot be determined or if the Tauri
/// runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_safe_area::init())
        .plugin(tauri_plugin_biometric_keystore::init())
        .plugin(tauri_plugin_secure_keystore::init())
        .plugin(tauri_plugin_file_picker::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("Cannot determine app config directory");

            // At-rest master key: from the Android Keystore on Android; `None`
            // (plaintext passthrough) on desktop.
            let master_key = master_key_from(app.secure_keystore());
            let store = Arc::new(Store::new(config_dir, master_key));
            // One-time migration of any pre-existing plaintext files into the
            // at-rest envelope (no-op on desktop / already-wrapped). Each file
            // is wrapped atomically with a roundtrip check, so a failure leaves
            // plaintext intact — logged, non-fatal.
            if let Err(e) = tauri::async_runtime::block_on(store.migrate_at_rest()) {
                eprintln!("[gpm] at-rest migration failed: {e}");
            }

            app.manage(AppState {
                store,
                lock_timer: Mutex::new(None),
                lock_generation: Arc::new(AtomicU64::new(0)),
                pending_identity: Mutex::new(None),
                pending_write: Arc::new(Mutex::new(None)),
            });
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
            read::show_remote_secret,
            // write / sync
            write::pull_repo,
            write::push_repo,
            write::resolve_sync_divergence,
            write::list_create_presets,
            write::lookup_template,
            write::preview_create,
            write::create_secret,
            write::create_from_preset_secret,
            write::delete_secret,
            write::edit_secret,
            write::resolve_write_conflict,
            // config
            config::get_config,
            config::set_commit_identity,
            config::get_commit_identity_default,
            config::reset_config,
            // biometric
            biometric::is_biometric_available,
            biometric::is_biometric_unlock_enabled,
            biometric::enable_biometric_unlock,
            biometric::biometric_unlock,
            biometric::disable_biometric_unlock,
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
