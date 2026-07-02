// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Identity & access — the age/SSH identity, its locked runtime session, and
//! the auto-lock model.
//!
//! Owns unlock/lock, passphrase management, SSH key material, and the shared
//! lock-state plumbing (`reset_lock_timer` / `emit_lock_state` / `soft_wipe`)
//! that `read`, `write`, `config`, and `setup` reuse.
//!
//! ## Two wipe paths
//!
//! The lock transition is split into two paths so the no-cache (`Immediate`)
//! mode can wipe the identity after each secret access without also dismissing
//! a secret the user is still viewing:
//! - A **hard** lock (`do_lock`, or the idle timer firing under `Idle` mode)
//!   wipes the identity, raises the unlock overlay, and clears revealed secrets
//!   — `emit_lock_state(_, _, false)`.
//! - A **soft** wipe (`soft_wipe`, the `Immediate` no-cache mode's post-op
//!   step) wipes the identity *only* and emits `emit_lock_state(_, _, true)` —
//!   the overlay stays down and a just-revealed secret stays on screen until
//!   its own view-clear timer. `maybe_soft_wipe` is the gated wrapper the
//!   read/write commands call after each op.

use std::sync::atomic::Ordering;
use std::time::Duration;

use rustpass::ssh;
use rustpass::{Error, ErrorCode, Store};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_biometric_keystore::KeystoreExt;

use crate::{AppState, write};

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `generate_ssh_key` — contains both keys for setup form.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshKeyPairResult {
    public_key: String,
    private_key: String,
}

/// Returned by `get_ssh_public_key` — public key only, safe to display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshPublicKeyResult {
    public_key: String,
}

/// Returned by `export_ssh_private_key` — secret, strict Vue lifecycle required.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshPrivateKeyResult {
    private_key: String,
}

// ---------------------------------------------------------------------------
// Lock-state plumbing
// ---------------------------------------------------------------------------

/// Snapshot of the identity lock state, emitted on every lock/unlock transition.
///
/// The frontend's `locked` ref is a pure mirror of this — it must never decide
/// lock state on its own (it used to, after its own `unlock` call, which desynced
/// from the backend on reset and on setup of an encrypted identity).
///
/// `soft` distinguishes the two wipe paths: a _hard_ lock (`soft == false`,
/// manual/idle) raises the unlock overlay and clears revealed secrets — today's
/// behavior. A _soft_ wipe (`soft == true`, the no-cache mode's post-op step)
/// only reports that the identity is no longer cached; the frontend leaves the
/// overlay down and any revealed secret on screen (it clears on its own
/// view-clear timer).
#[derive(Debug, Clone, Copy, Serialize)]
struct LockState {
    locked: bool,
    soft: bool,
}

/// Compute the current lock state from the store and emit it as
/// `identity-lock-state`, so the frontend mirrors the backend. `soft` marks a
/// soft wipe (no-cache mode) — see [`LockState`].
///
/// Runtime-generic so tests can drive it with the mock runtime; production
/// always calls with the default (`Wry`) runtime.
pub(crate) async fn emit_lock_state<R: Runtime>(app: &AppHandle<R>, store: &Store, soft: bool) {
    let locked = store.is_identity_encrypted().await && !store.is_unlocked();
    let _ = app.emit("identity-lock-state", LockState { locked, soft });
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Unlock a passphrase-encrypted identity (async — scrypt is slow).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
) -> Result<(), Error> {
    unlock_and_arm(&state, &app, &passphrase).await
}

/// Lock the store: clear cached identity and cancel auto-lock timer.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn lock(state: State<'_, AppState>, app: AppHandle) -> Result<(), Error> {
    do_lock(&state, &app).await;
    Ok(())
}

/// Core lock logic, shared by the [`lock`] command and the auto-lock timer's
/// fire path. Runtime-generic so tests can drive it with the mock runtime.
///
/// Cancels the auto-lock timer, disarms any racing in-flight timer task, wipes
/// the cached identity, drops any stashed conflict plaintext (it would be
/// undecryptable behind the wiped identity), and emits the new lock state.
pub(crate) async fn do_lock<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    // Cancel the armed timer + bump the generation so any in-flight timer task
    // self-disarms (shared with the soft-wipe / reset paths).
    disarm_lock(state);
    state.store.lock();
    // A conflict left pending would be undecryptable behind the wiped identity.
    write::clear_pending(&state.pending_write);
    // Emit the current lock state — same path the auto-lock timer takes.
    emit_lock_state(app, &state.store, false).await;
}

/// Set a passphrase on an existing plaintext identity.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_passphrase(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
) -> Result<(), Error> {
    state.store.set_passphrase(&passphrase).await?;
    // The sealed biometric passphrase (if any) is now stale — invalidate it.
    let _ = app.keystore().delete();
    // Setting a passphrase locks the store (forces re-auth with the new
    // passphrase); emit the real state so the frontend shows the overlay.
    emit_lock_state(&app, &state.store, false).await;
    Ok(())
}

/// Change the passphrase on an encrypted identity.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn change_passphrase(
    state: State<'_, AppState>,
    app: AppHandle,
    old_passphrase: String,
    new_passphrase: String,
) -> Result<(), Error> {
    state
        .store
        .change_passphrase(&old_passphrase, &new_passphrase)
        .await?;
    // The sealed biometric passphrase (if any) is now stale — invalidate it.
    let _ = app.keystore().delete();
    // Changing the passphrase locks the store; emit the real state.
    emit_lock_state(&app, &state.store, false).await;
    Ok(())
}

/// Generate a new ed25519 SSH keypair for setup.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn generate_ssh_key(passphrase: Option<String>) -> Result<SshKeyPairResult, Error> {
    let pair = ssh::generate_keypair(passphrase.as_deref())?;
    Ok(SshKeyPairResult {
        public_key: pair.public_key,
        private_key: pair.private_key.to_string(),
    })
}

/// Get the public key derived from the stored SSH private key.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_ssh_public_key(
    state: State<'_, AppState>,
) -> Result<SshPublicKeyResult, Error> {
    let config = state.store.config().await?;
    let private_key = config
        .ssh_key
        .ok_or_else(|| Error::new(ErrorCode::SshKeyInvalid, "No SSH key configured"))?;
    let public_key = ssh::get_public_key(&private_key)?;
    Ok(SshPublicKeyResult { public_key })
}

/// Export the stored SSH private key (secret — requires confirmation in UI).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn export_ssh_private_key(
    state: State<'_, AppState>,
) -> Result<SshPrivateKeyResult, Error> {
    let config = state.store.config().await?;
    let private_key_pem = config
        .ssh_key
        .ok_or_else(|| Error::new(ErrorCode::SshKeyInvalid, "No SSH key configured"))?;
    let private_key = ssh::export_private_key(&private_key_pem)?;
    Ok(SshPrivateKeyResult {
        private_key: private_key.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Auto-lock timer (shared: activity in read/write defers it)
// ---------------------------------------------------------------------------

/// Unlock the store with `passphrase` and (re)arm the auto-lock timer.
///
/// Shared by the password UI ([`unlock`]) and the biometric path
/// (`biometric::biometric_unlock`): both must produce the same post-unlock state
/// — identity cached, timer armed per the configured mode, lock state emitted —
/// so whichever unlock method the user used, the app is in an identical state.
pub(crate) async fn unlock_and_arm<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    passphrase: &str,
) -> Result<(), Error> {
    state.store.unlock(passphrase).await?;
    // Refresh the cached effective lock_mode so reset_lock_timer branches on the
    // user's actual setting (config may have changed since the last refresh).
    refresh_security_cache(state).await;
    reset_lock_timer(state, app);
    // The backend is the single source of truth for lock state; tell the frontend.
    emit_lock_state(app, &state.store, false).await;
    Ok(())
}

/// Load the repo config into the [`AppState`] security cache (`lock_mode`,
/// `clipboard_clear_secs`), so the read/write hot paths branch on a cheap mutex
/// read instead of decrypting `repo.json` per operation. Called on unlock and on
/// the `set_*` config commands — never on the copy/show hot path. A load failure
/// (e.g. mid-setup) leaves the defaults in place (fail-safe).
pub(crate) async fn refresh_security_cache(state: &State<'_, AppState>) {
    if let Ok(rc) = state.store.config().await {
        if let Ok(mut mode) = state.lock_mode.lock() {
            *mode = rc.lock_mode;
        }
        if let Ok(mut secs) = state.clipboard_clear_secs.lock() {
            *secs = rc.clipboard_clear_secs_effective();
        }
    }
}

/// Reset the auto-lock timer per the cached effective [`LockMode`]:
/// `Idle(n)` arms an idle timer for `n`; `Never` and `Immediate` arm no idle
/// timer at all (the no-cache mode wipes per operation instead; `Never` keeps
/// the session until a manual lock). Both also disarm any timer left over from a
/// prior `Idle` setting. Reads the [`AppState`] cache, so this stays sync (no
/// per-op config decrypt). On a cache miss (poisoned) it fails safe to the
/// default idle timer.
pub(crate) fn reset_lock_timer<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    let mode = state.lock_mode.lock().map_or_else(
        |_| rustpass::LockMode::Idle(rustpass::store::DEFAULT_LOCK_TIMEOUT_SECS),
        |m| *m,
    );
    match mode {
        rustpass::LockMode::Idle(secs) => arm_lock(state, app, secs),
        // No idle timer: Never keeps the session, Immediate wipes per-op. Either
        // way, disarm any idle timer armed under a prior Idle setting so it can't
        // fire and surprise-lock right after the mode switch.
        rustpass::LockMode::Never | rustpass::LockMode::Immediate => disarm_lock(state),
    }
}

/// Cancel any armed auto-lock timer and bump the generation so an in-flight
/// timer task self-disarms. Does NOT wipe the identity or emit — the timer-fire
/// path and the hard lock do their own wipe. Used by [`reset_lock_timer`] for
/// `Never`/`Immediate`, and as the timer-cancel half of [`soft_wipe`].
pub(crate) fn disarm_lock(state: &State<'_, AppState>) {
    if let Ok(mut timer) = state.lock_timer.lock()
        && let Some(handle) = timer.take()
    {
        handle.abort();
    }
    state.lock_generation.fetch_add(1, Ordering::SeqCst);
}

/// Soft wipe — the no-cache mode's post-operation step. Wipes the cached
/// identity (and disarms any idle timer) and emits a _soft_ lock-state event so
/// the frontend knows the next op needs re-auth, but **without** raising the
/// unlock overlay or clearing a revealed secret. Only the hard lock (manual /
/// idle) does those; a soft wipe leaves the UI exactly as it is. The caller
/// ([`maybe_soft_wipe`]) guarantees no conflict is pending first — a stashed
/// conflict plaintext is replayed by `resolve_write_conflict`, which needs the
/// identity, so it must never be left behind a wiped cache.
pub(crate) async fn soft_wipe<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    disarm_lock(state);
    state.store.lock();
    emit_lock_state(app, &state.store, true).await;
}

/// After a secret operation: under `Immediate` (no-cache) mode, soft-wipe the
/// identity so the next op re-authenticates. No-op for `Idle`/`Never` (the
/// session stays).
///
/// The `no_pending` guard once held back the wipe for the whole window a write
/// conflict was unresolved — the stashed plaintext was replayed by
/// `resolve_write_conflict`, which needs the identity, so wiping was suppressed
/// until resolve/cancel. The autosync write path never produces a `Conflict`, so
/// the stash is never populated and this guard is now always true; it stays as
/// defense-in-depth until the stash machinery is retired in `PR2c`. (The old
/// tradeoff — a lingering conflict caching the identity past `Immediate`'s
/// per-op intent — no longer applies.)
pub(crate) async fn maybe_soft_wipe<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    let immediate = state
        .lock_mode
        .lock()
        .is_ok_and(|m| matches!(*m, rustpass::LockMode::Immediate));
    let no_pending = state.pending_write.lock().is_ok_and(|p| p.is_none());
    if immediate && no_pending {
        soft_wipe(state, app).await;
    }
}

/// (Re)arm the auto-lock timer to fire after `secs`, replacing any in-flight
/// timer. Runtime-generic + duration-injected so tests can drive it with the
/// mock runtime and a sub-second timeout.
///
/// The spawned task captures its `generation` and self-disarms if a newer arm
/// happened while it slept — `abort` alone is not a generation check, so without
/// this a task already past its sleep could fire right after a fresh unlock.
pub(crate) fn arm_lock<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>, secs: u64) {
    let Ok(mut timer) = state.lock_timer.lock() else {
        return;
    };

    // Cancel existing timer
    if let Some(handle) = timer.take() {
        handle.abort();
    }

    // Bump the generation so any still-in-flight older task self-disarms on wake.
    let generation = state.lock_generation.fetch_add(1, Ordering::SeqCst) + 1;

    // Spawn new timer
    let app_handle = app.clone();
    let store = state.store.clone();
    let pending = state.pending_write.clone();
    let generation_cell = state.lock_generation.clone();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;

        // Stale-task guard: if a newer (re)arm happened while we slept, a fresher
        // unlock is in effect — do not lock/emit. `abort` is not a generation check,
        // so without this a task already past its sleep can fire right after an unlock.
        if generation_cell.load(Ordering::SeqCst) != generation {
            return;
        }

        // Clear any stashed conflict plaintext before wiping the identity —
        // otherwise it would be undecryptable and linger in memory.
        write::clear_pending(&pending);

        // Lock the real store (clears cached identity + passphrase)
        store.lock();

        // Emit the current lock state so the frontend shows the unlock overlay
        // + clears revealed secrets (a hard lock, not a soft wipe).
        emit_lock_state(&app_handle, &store, false).await;
    });

    *timer = Some(handle);
}
