// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret writes & sync — the write/sync side of the store.
//!
//! These wrap [`rustpass::Store`]'s gopass-style create / template / conflict
//! APIs ([`Store::create`], [`Store::preview_create`], and
//! [`Store::resolve_write_conflict`]) plus [`Store::sync`] (pull), and expose
//! them to the `WebView`.
//!
//! ## Conflict stash
//!
//! When a create collides with a newer remote copy ([`WriteOutcome::Conflict`]),
//! the backend rolls the local store back to the pre-write state and the caller
//! must decide how to resolve it. Re-resolving needs the *plaintext we tried to
//! write* — but we never want to round-trip that secret across IPC a second time.
//! So on conflict we stash `(name, plaintext)` in [`AppState::pending_write`]
//! (Rust heap, [`Zeroizing`]) and [`resolve_write_conflict`] replays it from
//! there. The stash is cleared on resolve (success *or* failure), on cancel,
//! and on lock (see [`clear_pending`]) — so a plaintext is never left behind a
//! wiped identity cache.
//!
//! [`Store::create`]: rustpass::Store::create
//! [`Store::preview_create`]: rustpass::Store::preview_create
//! [`Store::resolve_write_conflict`]: rustpass::Store::resolve_write_conflict
//! [`WriteOutcome::Conflict`]: rustpass::WriteOutcome::Conflict

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rustpass::template::{self, CreatePreset};
use rustpass::{ConflictChoice, Error, ErrorCode, SyncResult, WriteOutcome, WriteResult};
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::reset_lock_timer;

/// A write that collided with a newer remote copy and is awaiting the user's
/// resolution. Held only in memory; `plaintext` is [`Zeroizing`] and the struct
/// intentionally does not derive `Debug`.
pub(crate) struct PendingWrite {
    /// The entry name that collided (passed back to `resolve_write_conflict`).
    name: String,
    /// The plaintext we tried to write, replayed when the user picks
    /// `keep_mine` / `keep_mine_force`.
    plaintext: Zeroizing<Vec<u8>>,
}

/// Drop any stashed pending write.
///
/// Called on lock so a conflict modal left open across the 5-minute auto-lock
/// can't leave a plaintext behind a wiped identity cache (which would also make
/// a later resolve fail confusingly).
pub(crate) fn clear_pending(pending: &Arc<Mutex<Option<PendingWrite>>>) {
    if let Ok(mut pw) = pending.lock() {
        pw.take();
    }
}

/// Create a secret (applying a matching `.pass-template`), stash the plaintext
/// on conflict, and reset the auto-lock timer. Shared by the two create entry
/// points so both stash identically.
async fn do_create(
    state: &State<'_, AppState>,
    app: &AppHandle,
    name: &str,
    body: Vec<u8>,
) -> Result<WriteOutcome, Error> {
    let outcome = state.store.create(name, &body).await?;

    if matches!(outcome, WriteOutcome::Conflict(_)) {
        // Stash the plaintext so resolve_write_conflict can replay it without
        // the frontend re-sending the secret across IPC.
        let mut pw = state
            .pending_write
            .lock()
            .expect("pending_write mutex poisoned");
        *pw = Some(PendingWrite {
            name: name.to_string(),
            plaintext: Zeroizing::new(body),
        });
    }

    reset_lock_timer(state, app);
    Ok(outcome)
}

/// List the built-in secret-creation presets (Website login, PIN code) — the
/// "create from a few options" set the wizard offers.
#[tauri::command]
pub(crate) async fn list_create_presets() -> Vec<CreatePreset> {
    template::builtin_presets().to_vec()
}

/// Look up the `.pass-template` that would apply to `name`, if any. Used by the
/// wizard to hint that a template will shape the new secret.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn lookup_template(
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<String>, Error> {
    state.store.lookup_template(&name).await
}

/// Preview what [`rustpass::Store::create`] would store for `name` + `content`:
/// the rendered template body, or `None` when no template applies. Writes
/// nothing.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn preview_create(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<Option<String>, Error> {
    state.store.preview_create(&name, content.as_bytes()).await
}

/// Create a secret at an explicit path from its raw content (first line is the
/// password). A matching `.pass-template` is applied automatically.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn create_secret(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
    content: String,
) -> Result<WriteOutcome, Error> {
    do_create(&state, &app, &name, content.into_bytes()).await
}

/// Create a secret from one of the built-in presets, generating it at the
/// preset's fixed path from a few field values (Website → `websites/…`,
/// PIN → `pin/…`).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn create_from_preset_secret(
    state: State<'_, AppState>,
    app: AppHandle,
    preset_id: String,
    fields: HashMap<String, String>,
) -> Result<WriteOutcome, Error> {
    let preset = template::find_preset(&preset_id).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidEntryName,
            format!("unknown create preset: {preset_id:?}"),
        )
    })?;
    // Tauri hands us HashMap<String, String>; the template helpers key off the
    // preset's `&'static str` field keys, so rebuild as HashMap<&str, String>.
    let fields_ref: HashMap<&str, String> = fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    let name = template::preset_name(preset, &fields_ref)?;
    let body = template::preset_body(preset, &fields_ref)?;
    do_create(&state, &app, &name, body).await
}

/// Resolve a write conflict ([`WriteOutcome::Conflict`]) per the user's
/// `choice`. Replays the stashed plaintext for `keep_mine` / `keep_mine_force`;
/// `keep_remote` fast-forwards to the remote, `cancel` leaves the pre-write
/// state. The stash is always consumed (cleared) on return — the frontend
/// re-runs `create_secret` / `create_from_preset_secret` to re-stash on retry.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn resolve_write_conflict(
    state: State<'_, AppState>,
    app: AppHandle,
    choice: ConflictChoice,
) -> Result<Option<WriteResult>, Error> {
    let pending = {
        let mut pw = state
            .pending_write
            .lock()
            .expect("pending_write mutex poisoned");
        pw.take()
    };
    let Some(pending) = pending else {
        return Err(Error::new(
            ErrorCode::StoreError,
            "no pending write to resolve",
        ));
    };
    let result = state
        .store
        .resolve_write_conflict(&pending.name, &pending.plaintext, choice)
        .await;
    reset_lock_timer(&state, &app);
    result
}

/// Pull latest changes from the remote (fast-forward only).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn pull_repo(state: State<'_, AppState>) -> Result<SyncResult, Error> {
    state.store.sync().await
}

#[cfg(test)]
mod tests {
    //! The lock-clearing invariant is the security-critical piece of the conflict
    //! stash: a plaintext must not survive a lock. The create/resolve data flow
    //! itself is covered end-to-end by the `rustpass` integration tests.

    use super::*;

    fn stashed(name: &str) -> Arc<Mutex<Option<PendingWrite>>> {
        Arc::new(Mutex::new(Some(PendingWrite {
            name: name.to_string(),
            plaintext: Zeroizing::new(b"s3kr3t".to_vec()),
        })))
    }

    #[test]
    fn clear_pending_drops_a_stashed_write() {
        let pending = stashed("sites/foo");
        clear_pending(&pending);
        assert!(pending.lock().unwrap().is_none());
    }

    #[test]
    fn clear_pending_is_a_noop_when_empty() {
        let pending: Arc<Mutex<Option<PendingWrite>>> = Arc::new(Mutex::new(None));
        clear_pending(&pending);
        assert!(pending.lock().unwrap().is_none());
    }
}
