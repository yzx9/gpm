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

use std::fmt;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rustpass::ssh;
use rustpass::{Error, ErrorCode, Store};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_biometric_keystore::KeystoreExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_clipboard_notify::ClipboardNotifyExt;

use crate::AppState;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `generate_ssh_key` — contains both keys for setup form.
#[derive(Clone, Serialize)]
pub(crate) struct SshKeyPairResult {
    public_key: String,
    private_key: String,
}

/// Redacts `private_key` — mirrors `crate::read::SensitiveContent` so `Debug`
/// never leaks the PEM. `Serialize` is unaffected (the private key still crosses
/// IPC for the setup form).
impl fmt::Debug for SshKeyPairResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshKeyPairResult")
            .field("public_key", &self.public_key)
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

/// Returned by `get_ssh_public_key` — public key only, safe to display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshPublicKeyResult {
    public_key: String,
}

/// Returned by `export_ssh_private_key` — secret, strict Vue lifecycle required.
#[derive(Clone, Serialize)]
pub(crate) struct SshPrivateKeyResult {
    private_key: String,
}

/// Redacts `private_key` — mirrors `crate::read::SensitiveContent` so `Debug`
/// never leaks it. `Serialize` is unaffected (the key still crosses IPC).
impl fmt::Debug for SshPrivateKeyResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshPrivateKeyResult")
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod redaction_tests {
    //! Redaction regressions for the secret-bearing IPC result types — assert
    //! `Debug` redacts the private key while `Serialize` still carries it (so IPC
    //! keeps working). Mirrors `debug_redacts_password` in `rustpass::Secret` and
    //! the serialize-transparent pattern in `read.rs`.
    use super::*;

    #[test]
    fn ssh_key_pair_result_debug_redacts_but_serializes() {
        let result = SshKeyPairResult {
            public_key: "ssh-ed25519 AAAApublic".to_string(),
            private_key: "-----BEGIN OPENSSH PRIVATE KEY-----".to_string(),
        };
        let dbg = format!("{result:?}");
        assert!(dbg.contains("[REDACTED]"), "Debug redacts: {dbg}");
        assert!(
            !dbg.contains("BEGIN OPENSSH PRIVATE KEY"),
            "private key leaked into Debug: {dbg}"
        );
        assert!(dbg.contains("AAAApublic"), "public key is safe: {dbg}");
        // Serialize still carries the private key across IPC.
        let json = serde_json::to_string(&result).expect("serializes");
        assert!(
            json.contains("BEGIN OPENSSH PRIVATE KEY"),
            "Serialize must still carry the key for IPC: {json}"
        );
    }

    #[test]
    fn ssh_private_key_result_debug_redacts_but_serializes() {
        let result = SshPrivateKeyResult {
            private_key: "-----BEGIN OPENSSH PRIVATE KEY-----".to_string(),
        };
        let dbg = format!("{result:?}");
        assert!(dbg.contains("[REDACTED]"), "Debug redacts: {dbg}");
        assert!(
            !dbg.contains("BEGIN OPENSSH PRIVATE KEY"),
            "private key leaked into Debug: {dbg}"
        );
        let json = serde_json::to_string(&result).expect("serializes");
        assert!(
            json.contains("BEGIN OPENSSH PRIVATE KEY"),
            "Serialize must still carry the key: {json}"
        );
    }
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

/// Best-effort idle-timer bump fired by frontend user activity (tap / scroll /
/// key). Thin IPC wrapper over [`reset_lock_timer`]: the frontend throttles
/// (~1 per few seconds while active) and filters on its cached `LockMode` +
/// `identityCached`, so this only runs under `Idle` while cached and active.
/// Does NOT call [`refresh_security_cache`] — no config changed (matches the
/// per-op resets in `read`/`write`, which also call `reset_lock_timer` without
/// re-refreshing). The backend timer stays authoritative; see [`reset_lock_timer`]
/// for `Immediate`/`Never`/`Idle` branching.
///
/// The server side does NOT re-check `LockMode`/`identityCached` — the filter is
/// frontend-only, so this is a private IPC whose only caller is the activity
/// bumper (never reused). A bump landing just after a hard-lock re-arms a timer
/// against an already-locked store: benign dead work (`store.lock()` is
/// idempotent; the re-emitted hard-lock event is a no-op for the frontend).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn bump_idle_timer(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), Error> {
    reset_lock_timer(&state, &app);
    Ok(())
}

/// Core lock logic, shared by the [`lock`] command and the auto-lock timer's
/// fire path. Runtime-generic so tests can drive it with the mock runtime.
///
/// Cancels the auto-lock timer, disarms any racing in-flight timer task, wipes
/// the cached identity, and emits the new lock state.
pub(crate) async fn do_lock<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    log::info!("identity: locked");
    // Cancel the armed timer + bump the generation so any in-flight timer task
    // self-disarms (shared with the soft-wipe / reset paths).
    disarm_lock(state);
    state.store.lock();
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
    log::info!("identity: set-passphrase");
    state
        .store
        .set_passphrase(&passphrase)
        .await
        .inspect_err(|e| log::warn!("identity: set-passphrase failed: {e}"))?;
    // The sealed biometric passphrase (if any) is now stale — invalidate it.
    if let Err(e) = app.keystore().delete().await {
        log::warn!("identity: stale biometric slot delete failed: {e:?}");
    }
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
    log::info!("identity: change-passphrase");
    state
        .store
        .change_passphrase(&old_passphrase, &new_passphrase)
        .await
        .inspect_err(|e| log::warn!("identity: change-passphrase failed: {e}"))?;
    // The sealed biometric passphrase (if any) is now stale — invalidate it.
    if let Err(e) = app.keystore().delete().await {
        log::warn!("identity: stale biometric slot delete failed: {e:?}");
    }
    // Changing the passphrase locks the store; emit the real state.
    emit_lock_state(&app, &state.store, false).await;
    Ok(())
}

/// Generate a new ed25519 SSH keypair for setup.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn generate_ssh_key(passphrase: Option<String>) -> Result<SshKeyPairResult, Error> {
    log::info!("identity: generate-ssh-key");
    let pair = ssh::generate_keypair(passphrase.as_deref())
        .inspect_err(|e| log::warn!("identity: generate-ssh-key failed: {e}"))?;
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
    log::info!("identity: export-ssh-private-key");
    let config = state.store.config().await?;
    let private_key_pem = config
        .ssh_key
        .ok_or_else(|| Error::new(ErrorCode::SshKeyInvalid, "No SSH key configured"))?;
    let private_key = ssh::export_private_key(&private_key_pem)
        .inspect_err(|e| log::warn!("identity: export-ssh-private-key failed: {e}"))?;
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
    log::info!("identity: unlocking");
    state
        .store
        .unlock(passphrase)
        .await
        .inspect_err(|e| log::warn!("identity: unlock failed: {e}"))?;
    log::info!("identity: unlocked");
    // Refresh the cached effective lock_mode so reset_lock_timer branches on the
    // user's actual setting (config may have changed since the last refresh).
    refresh_security_cache(state).await;
    reset_lock_timer(state, app);
    // The backend is the single source of truth for lock state; tell the frontend.
    emit_lock_state(app, &state.store, false).await;
    Ok(())
}

/// Snapshot the app config into the [`AppState`] security cache (`lock_mode`,
/// `clipboard_clear_secs`), so the read/write hot paths branch on a cheap mutex
/// read instead of re-reading config per operation. These prefs live in
/// `app.json` (plaintext, always readable), so this never fails the way the old
/// sealed-`repo.json` read could.
pub(crate) fn apply_security_caches(state: &AppState) {
    let cfg = state.app_config.get();
    if let Ok(mut mode) = state.lock_mode.lock() {
        *mode = cfg.lock_mode;
    }
    if let Ok(mut secs) = state.clipboard_clear_secs.lock() {
        *secs = cfg.clipboard_clear_secs_effective();
    }
}

/// [`apply_security_caches`] wrapped for the Tauri `State` view. Called on
/// unlock, on the `set_*` config commands, and after the config-scope migration.
pub(crate) async fn refresh_security_cache(state: &State<'_, AppState>) {
    apply_security_caches(state.inner());
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
/// ([`maybe_soft_wipe`]) decides when to invoke this — a save that returned
/// [`rustpass::WriteOutcome::NeedsDivergenceResolve`] skips it so a keep-mine
/// resolve can reuse the cached identity, then performs the wipe itself once the
/// resolve settles.
pub(crate) async fn soft_wipe<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    disarm_lock(state);
    state.store.lock();
    emit_lock_state(app, &state.store, true).await;
}

/// After a secret operation: under `Immediate` (no-cache) mode, soft-wipe the
/// identity so the next op re-authenticates. No-op for `Idle`/`Never` (the
/// session stays).
///
/// Callers decide whether to invoke this on a given outcome — a save that
/// returned `NeedsDivergenceResolve` skips it (`resolve_sync_divergence` still
/// needs the identity for a keep-mine resolve) and `resolve_sync_divergence`
/// does the wipe after it settles.
pub(crate) async fn maybe_soft_wipe<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    let immediate = state
        .lock_mode
        .lock()
        .is_ok_and(|m| matches!(*m, rustpass::LockMode::Immediate));
    if immediate {
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
    let generation_cell = state.lock_generation.clone();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;

        // Stale-task guard: if a newer (re)arm happened while we slept, a fresher
        // unlock is in effect — do not lock/emit. `abort` is not a generation check,
        // so without this a task already past its sleep can fire right after an unlock.
        if generation_cell.load(Ordering::SeqCst) != generation {
            return;
        }

        log::info!("identity: locked (idle)");
        // Lock the real store (clears cached identity + passphrase)
        store.lock();

        // Emit the current lock state so the frontend shows the unlock overlay
        // + clears revealed secrets (a hard lock, not a soft wipe).
        emit_lock_state(&app_handle, &store, false).await;
    });

    *timer = Some(handle);
}

/// The post-sleep clipboard-clear decision + action. Extracted from
/// [`arm_clipboard_clear`] so the manual-clear decision is injectable: off-Android
/// the `clipboard_notify()` bridge is an inert `false` stub, so the load-bearing
/// self-skip branch is otherwise unreachable from `cargo test`. A host test
/// injects a `true`/`false` probe + a spy clear to prove skip-vs-fire without
/// the OS clipboard (unavailable on headless CI). Production wires the real
/// probe (the `clipboard_notify()` bridge) + the real clear (`clipboard()` write
/// + `dismiss`); the clear-and-dismiss is ONE action so the two cannot drift.
///
/// Generic over the probe/clear futures (mirrors the `FnOnce() -> Fut + Send`
/// pattern in `write.rs`) so callers pass plain `async` blocks — no boxing.
pub(crate) async fn run_clipboard_clear<P, C, PFut, CFut>(manual_clear: P, clear_and_dismiss: C)
where
    P: FnOnce() -> PFut + Send,
    PFut: Future<Output = bool> + Send,
    C: FnOnce() -> CFut + Send,
    CFut: Future<Output = ()> + Send,
{
    if manual_clear().await {
        return;
    }
    clear_and_dismiss().await;
}

/// (Re)arm the clipboard-clear timer to fire after `secs`, replacing any
/// in-flight task. Mirrors [`arm_lock`]: abort the existing handle, bump the
/// generation, and spawn a task that self-disarms if a newer arm happened
/// while it slept. Aborting the prior handle on each arm fixes the copy-overlap
/// bug (copy-A's earlier timer clearing copy-B's secret short of its full
/// timeout).
///
/// The spawned task carries the manual-clear bridge: on wake it consumes the
/// plugin's manual-clear flag (set by the notification-tap receiver) and, if
/// set, self-skips — so a manual tap-clear is not later undone by this timer
/// clobbering whatever the user copied next. The flag is reset in
/// `postClipboardNotification` (Kotlin) at post time, so the reset always
/// precedes any user tap (no race with the receiver). There is no Kotlin→Rust
/// event for this (Tauri's plugin `trigger` is plugin-scoped, unreachable from
/// a global Rust `listen` — see tauri issue #13027); the flag is polled via the
/// proven `run_mobile_plugin_async` direction.
pub(crate) fn arm_clipboard_clear<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    secs: u64,
) {
    let Ok(mut handle) = state.clipboard_clear_handle.lock() else {
        return;
    };

    // Cancel any in-flight clear (the copy-overlap fix: copy-A's earlier task
    // must not survive to clear copy-B's secret short of its full timeout).
    if let Some(h) = handle.take() {
        h.abort();
    }

    let generation = state
        .clipboard_clear_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let generation_cell = state.clipboard_clear_generation.clone();
    let app_handle = app.clone();

    let task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;

        // Stale-task guard: a fresh copy bumped the generation while we slept.
        if generation_cell.load(Ordering::SeqCst) != generation {
            return;
        }

        // Manual-clear bridge + auto-clear, factored into `run_clipboard_clear`
        // so the decision is injectable for host tests. Production wires the
        // real `clipboard_notify()` probe + the real clear-and-dismiss.
        let app_for_probe = app_handle.clone();
        let app_for_clear = app_handle.clone();
        run_clipboard_clear(
            move || async move {
                app_for_probe
                    .clipboard_notify()
                    .consume_manual_clear_flag()
                    .await
            },
            move || async move {
                // No manual clear — fire the auto-clear + dismiss.
                if let Err(e) = app_for_clear.clipboard().write_text(String::new()) {
                    log::warn!("clipboard: auto-clear write failed: {e}");
                }
                app_for_clear.clipboard_notify().dismiss().await;
            },
        )
        .await;
    });

    *handle = Some(task);
}

/// Cancel any armed clipboard-clear timer and bump the generation so an
/// in-flight task self-disarms. Called when the configured timeout is `Never`
/// (0) — a stale timer left over from a prior shorter setting must not fire and
/// clear the clipboard the user explicitly asked to leave alone.
pub(crate) fn disarm_clipboard_clear(state: &State<'_, AppState>) {
    if let Ok(mut handle) = state.clipboard_clear_handle.lock()
        && let Some(h) = handle.take()
    {
        h.abort();
    }
    state
        .clipboard_clear_generation
        .fetch_add(1, Ordering::SeqCst);
}
