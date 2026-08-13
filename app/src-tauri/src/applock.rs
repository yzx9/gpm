// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! App-launch biometric gate (RFC 0028) — an opt-in lock that gates the **age
//! identity** behind a biometric, so the secrets themselves stay unreadable
//! until the user authenticates on launch/resume.
//!
//! R064 split the at-rest seal into two keys, and this gate controls only one:
//! the **vault key** (biometric-gated when the gate is on), which seals the
//! `identity` and its passphrase slot (`app_id_pass`). The **auth-free master
//! key** (the other half of the split) stays permanently retrievable without a
//! prompt and keeps sealing `repo.json` + `app.json` — so the headless
//! background worker can read the git credential and pull-sync even while the
//! gate is locked. The gate therefore locks the *identity*, not the whole store.
//!
//! This is a **third**, UI/session-layer lock, deliberately independent of the
//! identity cache lock (`identity::`) and of the auth-free seal master key:
//! - Enabling mints a distinct vault key and re-keys the identity + passphrase
//!   slot under it (and re-keys them back to the auth-free master on disable).
//!   The vault alias's presence IS the toggle state — probed non-promptingly at
//!   startup.
//! - `app_unlock` retrieves the vault key via a biometric prompt and injects it
//!   into the `Store` (the auth-free master is loaded non-promptingly just
//!   before); when it re-locks, `app_lock` wipes the vault key (and the identity
//!   cache) so a locked app cannot read the identity even from memory — while the
//!   auth-free master stays keyed. (R058: a return within `gate_idle = After(N)`
//!   is a grace no-op — no wipe.)
//! - While the gate is active the frontend suppresses the identity overlay, so
//!   the two never race to show competing prompts.
//!
//! The identity-auto-unlock opt-in (one app-unlock also unlocks the identity)
//! layers on top: the identity passphrase is sealed under the vault key, so
//! `app_unlock` decrypts it with no second prompt when the opt-in is on.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use rustpass::Error;
use rustpass::error::ErrorCode;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_keystore::{BiometricState, KeystoreError, KeystoreExt, PromptText};
use zeroize::Zeroizing;

use crate::entry_cache::EntryCacheReason;
use crate::keystore::BiometricSlot;
use crate::migrations::run_app_migrations;
use crate::verbose::arm_verbose_timer;
use crate::{AppState, GateIdle, entry_cache, identity, keystore};

// ---------------------------------------------------------------------------
// Tauri-IPC types
// ---------------------------------------------------------------------------

/// App-lock error — serializes to `{ code, message }` (same shape as
/// `rustpass::Error` / `BiometricError`) so the frontend destructures all
/// uniformly. Carries the plugin's `KEYSTORE_*` codes and
/// maps `rustpass::Error` for the config writes.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AppLockError {
    code: String,
    message: String,
}

impl AppLockError {
    /// Build a generic `APP_LOCK_FAILED` error with a safe (no-secret) message.
    #[must_use]
    fn failed(message: &str) -> Self {
        Self {
            code: "APP_LOCK_FAILED".to_string(),
            message: message.to_string(),
        }
    }
}

impl From<Error> for AppLockError {
    fn from(e: Error) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

impl From<KeystoreError> for AppLockError {
    fn from(e: KeystoreError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

impl fmt::Display for AppLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Why the gate locked — drives the frontend's auto-prompt rule (the gate
/// mirror of identity's `LockEventReason`). `Idle` (the in-app idle timer
/// fired; the user is present but idle) suppresses the auto-prompt so the mask
/// shows and they tap; `Return` (a foreground-return re-lock) keeps it. `None`
/// (cold start, no transition yet) → the frontend treats null as "prompt."
/// R058 reuses `Return` for the resume-past-timeout re-lock (no new variant): the
/// resume path (`app_lock`) re-emits `Return` for a lock found at the return instant.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AppLockReason {
    Return,
    Idle,
}

/// Snapshot of the app-lock state, emitted as `app-lock-state` on every
/// transition and returned by `get_app_lock_state`.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct AppLockState {
    /// Whether the gate is enabled (master key lives in the biometric-gated
    /// store).
    enabled: bool,
    /// Whether the app is currently locked (master key not in memory).
    locked: bool,
    /// Why the gate most recently locked (`None` until a lock transition, incl.
    /// cold start). See [`AppLockReason`].
    reason: Option<AppLockReason>,
}

/// Emit the current app-lock state so the frontend mirrors it. `reason` is the
/// lock transition cause (`None` for an unlock emit or the cold-start snapshot).
fn emit_app_lock_state<R: Runtime>(
    app: &AppHandle<R>,
    enabled: bool,
    locked: bool,
    reason: Option<AppLockReason>,
) {
    let _ = app.emit(
        "app-lock-state",
        AppLockState {
            enabled,
            locked,
            reason,
        },
    );
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Whether the app-launch biometric gate is usable on this device (API 30+ with
/// a STRONG biometric). `false` on desktop / Android <11 / no/too-weak biometric.
/// Gates the Settings toggle. The probe itself is quad-state
/// ([`tauri_plugin_keystore::BiometricState`]); this command keeps a bool
/// boundary so the app-lock frontend is unchanged.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn is_app_lock_available(app: AppHandle) -> Result<bool, AppLockError> {
    Ok(app.keystore().is_biometric_available().await? == BiometricState::Available)
}

/// Current app-lock state, for the frontend's initial render.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn get_app_lock_state(state: State<'_, AppState>) -> AppLockState {
    AppLockState {
        enabled: state.app_lock_enabled.load(Ordering::SeqCst),
        locked: state.app_locked.load(Ordering::SeqCst),
        // Cold start: no lock transition yet — the frontend treats null as
        // "prompt," matching today's cold-start auto-prompt.
        reason: None,
    }
}

/// Enable the app-launch biometric gate (R064 model B): mint a **distinct**
/// vault key, seal it behind biometric (ENCRYPT prompt), and re-key `identity`
/// + `app_id_pass` from the auth-free master to the vault key. The auth-free
/// master stays permanent (never deleted) so the headless worker can sync under
/// lock — only the identity moves under the gate. The ENCRYPT prompt runs before
/// the re-key, so a cancel leaves the store untouched; the vault alias is
/// created before the identity moves, so a mid-enable crash is self-healed by
/// the `app_unlock` resume.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn enable_biometric_app_lock(
    state: State<'_, AppState>,
    app: AppHandle,
    prompt_text: Option<PromptText>,
) -> Result<(), AppLockError> {
    log::info!("app-lock: enable");
    let ks = app.keystore();
    if ks.is_biometric_available().await? != BiometricState::Available {
        return Err(AppLockError::from(KeystoreError::unavailable()));
    }
    // Already enabled (a biometric key already exists) — nothing to do.
    if keystore::has_app_lock_enabled(ks).await {
        state.app_lock_enabled.store(true, Ordering::SeqCst);
        return Ok(());
    }

    // Read the auth-free master key (non-prompting). R064 keeps it auth-free and
    // permanent — it is NOT migrated away; we use it to re-key identity.
    let master = keystore::retrieve_master(ks)
        .await?
        .ok_or_else(|| AppLockError::failed("No auth-free master key to gate"))?;

    // Mint a vault key DISTINCT from the master. The forensic property of the
    // split depends on this: the auth-free master must not decrypt identity.
    let vault = rustpass::seal::generate_master_key()
        .map_err(|e| AppLockError::failed(&format!("vault key generation failed: {e}")))?;

    // Create the vault alias FIRST (ENCRYPT prompt). Crash-safe: the vault key
    // is recoverable BEFORE identity moves under it.
    keystore::store_slot(ks, &vault, BiometricSlot::Vault, prompt_text.as_ref()).await?;
    // Re-key identity + app_id_pass master→vault. These are repo-scoped (the
    // identity + passphrase slot live under the active repo facade), so route
    // them through the active repo facade (rooted at the
    // per-repo subdir). The auth-free master is twin-injected onto it so its
    // master_seal can read identity under the master; both seals are keyed now
    // (master_seal reads, the just-injected vault_seal writes).
    let active = state.active_repo()?;
    state.store.set_master_key(Some(master));
    active.set_master_key(Some(master));
    active.set_vault_key(Some(vault));
    active.rekey_identity_to_vault().await?;
    // The auth-free master stays (permanent) — do NOT delete it. Persist the
    // flag last so a crash before this leaves the resume to finish the enable.
    state.app_config.set_biometric_app_lock(true).await?;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    // The gate is now on with the app unlocked — arm the in-app idle timer
    // (R057). No-op when gate-idle is Off.
    identity::reset_gate_idle_timer(&state, &app, &active);
    Ok(())
}

/// Disable the app-launch biometric gate (R064 model B): one biometric DECRYPT
/// retrieves the vault key, `identity` + `app_id_pass` re-key vault→master, the
/// vault alias is dropped, and `vault_seal` collapses back onto the permanent
/// auth-free master. If the biometric key is dead (all biometrics removed), the
/// vault key is unrecoverable and this fails — re-setup is the only path.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn disable_biometric_app_lock(
    state: State<'_, AppState>,
    app: AppHandle,
    prompt_text: Option<PromptText>,
) -> Result<(), AppLockError> {
    log::info!("app-lock: disable");
    // Disarm the gate idle timer FIRST, before any await (the retrieve_biometric
    // prompt below awaits user input). A gate-idle fire during the disable
    // sequence would wipe the master key mid-disable → lockout with no biometric
    // key to recover (R057). The generation bump also self-disarms any task that
    // hasn't passed its stale-check yet.
    identity::disarm_gate_idle(&state);
    let ks = app.keystore();
    // Retrieve the vault key (DECRYPT prompt) — the biometric key post-R064.
    let vault = keystore::retrieve_slot(ks, BiometricSlot::Vault, prompt_text.as_ref())
        .await?
        .ok_or_else(|| AppLockError::failed("No vault key to migrate back"))?;
    // The identity + passphrase slot are repo-scoped — route their reads/writes
    // through the active repo facade (== `state.store` under the single-repo
    // invariant).
    let active = state.active_repo()?;
    active.set_vault_key(Some(vault)); // key vault_seal to read identity
    // Load the auth-free master (permanent, non-prompting) to write identity
    // back under it. The in-memory master may have been wiped by a prior
    // `app_lock` (disable can run while locked), so re-inject it BEFORE the
    // re-key — rekey_identity_to_master writes via master_seal. Twin-inject:
    // the device facade (seals app.json) and the active repo facade (seals
    // repo.json/identity) each need the master keyed.
    let master = keystore::retrieve_master(ks)
        .await?
        .ok_or_else(|| AppLockError::failed("No auth-free master to disable into"))?;
    state.store.set_master_key(Some(master));
    active.set_master_key(Some(master));
    // Re-key identity + app_id_pass vault→master, drop the vault alias, then
    // collapse vault_seal onto the master (post-disable there is no separate
    // vault — identity lives under the master).
    active.rekey_identity_to_master().await?;
    keystore::delete_slot(ks, BiometricSlot::Vault).await?;
    active.set_vault_key(Some(master));
    state.app_config.set_biometric_app_lock(false).await?;
    // The identity-auto-unlock opt-in is meaningless without the gate (app_unlock
    // is never called when app_lock_enabled is false). Clear its flag + sealed
    // passphrase slot so re-enabling the gate later starts clean — otherwise the
    // persisted flag would silently re-activate auto-unlock with the old sealed
    // passphrase, and the Settings UI (which hides the opt-in while the gate is
    // off) would offer no way to clear it.
    if let Err(e) = active.clear_app_identity_pass().await {
        log::warn!("app-lock: clear-app-identity-pass cleanup failed: {e}");
    }
    if let Err(e) = active.set_unlock_identity_with_app(false).await {
        log::warn!("app-lock: set-unlock-identity-with-app cleanup failed: {e}");
    }

    state.app_lock_enabled.store(false, Ordering::SeqCst);
    state.app_locked.store(false, Ordering::SeqCst);
    emit_app_lock_state(&app, false, false, None);
    Ok(())
}

/// One-shot legacy-envelope migrate, CAS-guarded against concurrent callers.
///
/// Called by `app_unlock` after the master key is injected: under App Lock the
/// key is absent at cold start, so the startup migrate soft-skipped and the key
/// exists only now. `Store::migrate_seal` converts `GPMATR1` envelopes to
/// `GPMSEL1`. `pub(crate)` so the in-crate tests can drive it without a
/// keystore mock.
///
/// State: `0` = Pending, `1` = `InFlight`, `2` = Done. The claiming call sets Done
/// on Ok, Pending on Err (next unlock retries). A re-lock wiping the key
/// mid-flight makes the legacy branch soft-skip → Ok → Done; that envelope
/// stays legacy, kept readable by dual-read until the v1.0.x forced migrate.
// TODO: v1.0.x — remove with the legacy-magic compat path.
/// Whether the migration chain halted over a configured repo: schema still
/// below current AND `repo.json` still at the config root (m0009/m0010 not
/// both done — files not relocated, registry not derivable). The discriminator
/// [`app_unlock`] fails loudly on; extracted for unit tests.
pub(crate) async fn chain_left_repo_at_root(state: &AppState) -> bool {
    if !matches!(
        state.app_config.peek_schema_version().await,
        crate::app_config::PeekOutcome::Version(v)
            if v < crate::migrations::APP_CONFIG_SCHEMA_VERSION
    ) {
        return false;
    }
    crate::exists_or_err(&state.store.config_dir().join("repo.json"))
        .await
        .unwrap_or(false)
}

pub(crate) async fn run_seal_migrate_once(state: &AppState) {
    const SM_PENDING: u8 = 0;
    const SM_INFLIGHT: u8 = 1;
    const SM_DONE: u8 = 2;
    // The identity/repo.json/app_id_pass envelope migrate is repo-scoped — run
    // it on the active repo facade (rooted at the per-repo subdir). No active
    // repo ⇒ nothing to migrate; return without claiming the one-shot so a
    // later unlock retries.
    let Some(active) = state.registry.active_facade() else {
        return;
    };
    if state
        .seal_migrate_state
        .compare_exchange(SM_PENDING, SM_INFLIGHT, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        match active.migrate_seal().await {
            Ok(()) => {
                state.seal_migrate_state.store(SM_DONE, Ordering::Release);
            }
            Err(e) => {
                log::warn!("seal migrate failed, will retry: {e}");
                state
                    .seal_migrate_state
                    .store(SM_PENDING, Ordering::Release);
            }
        }
    }
}

/// One-shot backend resolve (storage + crypto), CAS-guarded against concurrent
/// callers. The shared CAS flag is `Done` only when BOTH resolve; on a hard
/// failure the next unlock retries both (see the body comment).
///
/// Called by `app_unlock` after the master key is injected (and after
/// [`run_seal_migrate_once`]): the backend type + root live in sealed
/// `repo.json`. (The master key is auth-free — R064 — but the foreground
/// deliberately defers loading it until `app_unlock`, so this resolves
/// post-unlock.) On a hard failure (unregistered `ext:`,
/// tampered config) the specific error is stashed in `Store` so `storage()`
/// surfaces it; the CAS resets to `Pending` so the next unlock retries (mirrors
/// `run_seal_migrate_once`). `pub(crate)` so in-crate tests can drive it.
pub(crate) async fn run_backend_resolve_once(state: &AppState) {
    const BR_PENDING: u8 = 0;
    const BR_INFLIGHT: u8 = 1;
    const BR_DONE: u8 = 2;
    // The backend type + root live in the repo-scoped `repo.json` — resolve
    // them on the active repo facade (rooted at the per-repo subdir). No active
    // repo ⇒ nothing to resolve; leave the one-shot pending so a later unlock
    // retries.
    let Some(active) = state.registry.active_facade() else {
        return;
    };
    if state
        .backend_resolve_state
        .compare_exchange(BR_PENDING, BR_INFLIGHT, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        // Shared one-shot flag: Done only when BOTH storage and crypto resolve.
        // They read the same sealed repo.json at the same unlock instant, so a
        // failure in either is a failure of the shared config read — retry both
        // on the next unlock.
        let storage_ok = active
            .resolve_storage()
            .await
            .inspect_err(|e| log::warn!("storage resolve failed: {e}"))
            .is_ok();
        let crypto_ok = active
            .resolve_crypto()
            .await
            .inspect_err(|e| log::warn!("crypto resolve failed: {e}"))
            .is_ok();
        state.backend_resolve_state.store(
            if storage_ok && crypto_ok {
                BR_DONE
            } else {
                BR_PENDING
            },
            Ordering::Release,
        );
    }
}

/// Unlock the app: retrieve the **vault key** via a biometric prompt and inject
/// it into the `Store` (the auth-free master key is loaded non-promptingly just
/// above). The identity cache is left wiped (re-established lazily by
/// per-operation auth, or by the identity-auto-unlock opt-in); a soft
/// identity-lock event tells the frontend the next identity-needing op will
/// re-authenticate WITHOUT raising the identity overlay over the just-unlocked
/// app.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn app_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    prompt_text: Option<PromptText>,
) -> Result<(), AppLockError> {
    log::info!("app-lock: unlock");
    // Idempotent: if already unlocked (or app-lock is off), skip the biometric
    // prompt entirely. Guards against a double-call re-prompting.
    if !state.app_locked.load(Ordering::SeqCst) {
        return Ok(());
    }
    // Order matters: the migration chain resumes HERE (below) — a deferred
    // chain (an app-lock upgrader below schema 7) has not yet run m0009/m0010,
    // so the registry may still be empty and the repo's files still sit at the
    // config root. Every repo-scoped step therefore resolves the active facade
    // AFTER the chain (via the device-facade fallback, which is also the
    // correct target while the files are unrelocated), and the keys retrieved
    // below are injected into the DEVICE facade first — m0005/m0006/m0007 gate
    // on and operate on it.
    // Disarm the gate-idle timer for the duration of the unlock awaits (the
    // DECRYPT prompt, m0007's ENCRYPT, run_app_migrations). A fire in that
    // window would call do_app_lock mid-unlock, wiping vault_seal before
    // app_locked clears — leaving the app rendered unlocked but unable to read
    // identity (SealKeyUnavailable) until the next lock/unlock. reset_gate_idle
    // _timer at the end of this fn re-arms it on success. Mirrors disable.
    identity::disarm_gate_idle(&state);
    let ks = app.keystore();
    // Load the auth-free master if present (post-m0007: permanent; upgrader
    // pre-m0007: absent — its master lives in the legacy biometric alias).
    // Non-prompting; a failure (incl. a malformed key) degrades to None (the
    // deadlock-fix below or m0007 supplies the master on the upgrader path).
    let mut retrieved_master = None;
    if let Ok(Some(master)) = keystore::retrieve_master(ks).await {
        // Device facade first (seals app.json; also the pre-relocate home of
        // repo.json). The repo facade gets it after the chain resolves below.
        state.store.set_master_key(Some(master));
        retrieved_master = Some(master);
    }
    // Retrieve the biometric key: the vault (post-m0007), or — when the vault
    // alias is absent (`None`, non-prompting) — the legacy master (upgrader
    // pre-m0007, until m0007 relocates it). Either way one DECRYPT. A present-
    // but-malformed slot surfaces as `Err(KEYSTORE_MALFORMED)` (the read-side
    // interpreter rejected the bytes) rather than a decode failure here.
    let (key, from_vault) =
        match keystore::retrieve_slot(ks, BiometricSlot::Vault, prompt_text.as_ref()).await {
            Ok(Some(key)) => (key, true),
            Ok(None) => {
                // Vault absent → upgrader: the master is trapped in the legacy alias.
                let key = keystore::retrieve_slot(ks, BiometricSlot::Legacy, prompt_text.as_ref())
                    .await
                    .map_err(|err| {
                        let ae: AppLockError = err.into();
                        log::warn!("app-lock: unlock (legacy) failed: {ae}");
                        ae
                    })?
                    .ok_or_else(|| AppLockError::failed("No biometric master key stored"))?;
                // R064: relocate the legacy master to the auth-free alias NOW (the
                // bytes are in hand here) so it becomes the permanent auth-free
                // master; m0007 then only has to mint the vault + re-key identity.
                // Idempotent on retry (overwrites the same value). A failure aborts
                // the unlock so the user retries rather than landing half-relocated.
                keystore::store_master(ks, &key).await?;
                // Inject BOTH seals of the DEVICE facade with this master so
                // identity (still under master pre-m0007) stays readable if
                // m0007's ENCRYPT cancels, so m0005/m0006 un-defer, and so
                // m0007's `has_vault_key` deferral gate passes (it reads the
                // device facade — the repo facade did not exist yet when the
                // chain was deferred). m0007 mints the distinct vault + re-keys.
                state.store.set_master_key(Some(key));
                state.store.set_vault_key(Some(key));
                (key, false)
            }
            Err(e) => {
                let ae: AppLockError = e.into();
                log::warn!("app-lock: unlock failed: {ae}");
                return Err(ae);
            }
        };
    log::info!("app-lock: biometric key retrieved");
    // Copy the app-scoped behavior prefs out of a pre-split repo.json into
    // app.json BEFORE anything reads them — the first unlock, and the cache
    // refresh inside try_identity_auto_unlock, must see the migrated values, not
    // the defaults. The master key is now in memory, so the sealed read succeeds.
    run_app_migrations(state.inner()).await;
    // A chain that HALTED mid-way (m0007 returning `Pending` when the user
    // cancels its one-time ENCRYPT prompt, or any migration error) leaves the
    // schema below current with the repo files still at the config root: the
    // registry cannot populate (no relocated subdir), so every repo-scoped
    // identity path dies — `unlock` funnels through the registry and the
    // WebView cannot even resolve a repoId, while `get_auth_state`'s
    // device-facade fallback reports configured=true, locking the user into a
    // session whose unlock overlay can never succeed. Fail loudly instead so
    // the user retries — the chain re-runs and re-prompts.
    if chain_left_repo_at_root(state.inner()).await {
        return Err(AppLockError::from(Error::new(
            ErrorCode::ConfigError,
            "app config migration incomplete — lock and unlock again to retry".to_string(),
        )));
    }
    // Re-apply the runtime log gate after migrations: under app-lock, the
    // verbose carry-over (m0004_verbose_from_debug) runs here, not in init_state.
    // Without this an upgrading user previously pinned to "debug" would spend
    // this first session at Info — the data lands on disk (verbose_until set),
    // but the runtime gate was last set at cold start (Info) and nothing
    // re-applied it. Mirrors the post-migration block in init_state.
    let _ = state
        .app_config
        .clear_expired_verbose()
        .await
        .map_err(|e| log::warn!("app-config: clear_expired_verbose failed: {e}"));
    log::set_max_level(state.app_config.effective_log_filter());
    // Re-arm the mid-session revert timer if a verbose window is still live.
    arm_verbose_timer(state.inner(), &app);
    // Reload the sealed config. R074/D: the auth-free key is loaded at
    // `.setup()`, so the merged config is already in the caches — this is a
    // defensive post-migration refresh (a just-run m0008 collapsed pref.json
    // into the sealed merged app.json; in the new world the refresh covers the
    // WHOLE cache, so a just-run m0009 registry write is visible below). Runs
    // before `app_locked` is cleared so the frontend sees real values after the
    // emit.
    state.app_config.reload_behavior().await.ok();
    // A chain that completed m0009/m0010 in THIS call (the deferred-upgrader
    // path) rewrote the registry on disk after init_state populated the
    // in-memory one — re-derive it (no-op when already populated). The seeding
    // inside also transfers the device facade's keys onto any freshly built
    // facade, including the vault key m0007 just minted.
    crate::repopulate_registry_if_empty(state.inner()).await;
    // Resolve the active facade AFTER the chain: post-relocate this is the
    // registry facade (rooted at `repositories/<id>/`); the device-facade
    // fallback covers the no-repo window and a chain that halted mid-way
    // (files still at the config root — where the device facade reads them).
    let active = crate::setup::active_or_device_facade(state.inner());
    if let Some(master) = retrieved_master {
        active.set_master_key(Some(master));
    }
    if from_vault {
        // Post-m0007: inject the vault key onto the active repo facade (the
        // master twin-inject above covered the device facade pre-chain).
        active.set_vault_key(Some(key));
        // Crash-safety resume: a half-finished enable/disable left identity
        // under the master while the vault alias is present — finish moving it
        // under the vault. An interrupted disable is undone here (identity stays
        // readable; the user re-disables) — no data loss either way.
        if active.is_identity_under_master().await {
            active.rekey_identity_to_vault().await?;
            if let Err(e) = state.app_config.set_biometric_app_lock(true).await {
                log::warn!("app-lock: resume set_biometric_app_lock persist failed: {e}");
            }
            log::info!("app-lock: resumed unfinished master→vault re-key");
        }
    } else {
        // Legacy upgrader: post-chain the identity lives under the vault
        // m0007 minted (injected into the device facade pre-chain and
        // transferred onto the repo facade by the re-populate seeding).
        active.set_master_key(Some(key));
        active.set_vault_key(state.store.vault_key());
    }
    active.set_autosync(state.app_config.get_behavior().autosync);
    // One-shot legacy-envelope migrate, BEFORE the unlock emit so the app isn't
    // interactive while repo.json is re-wrapped (no race with a settings write).
    // Under App Lock the key is absent at cold start, so convert it now.
    // TODO: v1.0.x — remove with the legacy-magic compat path.
    run_seal_migrate_once(&state).await;
    // One-shot storage-backend resolve: the backend type lives in sealed
    // repo.json, now readable. Runs before the unlock emit so content ops see a
    // resolved backend (not BackendNotAvailable). Mirrors the seal-migrate
    // one-shot; on a hard failure the error is stashed in Store for storage().
    run_backend_resolve_once(&state).await;
    // Identity-auto-unlock opt-in FIRST, before announcing the app is unlocked.
    // unlock_and_arm emits identity-lock-state{locked:false}, but the frontend's
    // UnlockModal is suppressed while `appLocked` is still true (its v-if gates
    // on `!appLocked`). Emitting app-unlocked BEFORE this runs opens an "app
    // unlocked / identity still locked" window where the frontend mounts
    // UnlockModal and, with identity biometric on, auto-fires a DUPLICATE
    // biometric unlock — a second BiometricPrompt right after the master-key
    // prompt plus a second scrypt (the "resume unlock spins forever" symptom).
    // Run the auto-unlock first so that by the time the app-unlock event lands,
    // the identity is already unlocked and UnlockModal never mounts.
    let auto_unlocked = try_identity_auto_unlock(&state, &app).await;
    log::info!(
        "app-lock: identity auto-unlock {}",
        if auto_unlocked { "done" } else { "skipped" }
    );
    state.app_locked.store(false, Ordering::SeqCst);
    let enabled = state.app_lock_enabled.load(Ordering::SeqCst);
    emit_app_lock_state(&app, enabled, false, None);
    // The app is unlocked with the master key resident — arm the in-app idle
    // timer (R057). No-op when gate-idle is Off.
    identity::reset_gate_idle_timer(&state, &app, &active);
    // Auto-unlock was off / no sealed passphrase / failed: for a passphrase-
    // encrypted identity, a SOFT identity event tells the frontend to use per-op
    // auth (no overlay over the just-unlocked app). A plaintext identity is
    // always readable straight from disk, so it must NOT receive a soft event —
    // that would force runWithAuth to raise an unusable UnlockModal (no
    // passphrase to enter) on every copy/show.
    if !auto_unlocked && active.is_identity_encrypted().await {
        identity::emit_lock_state(&app, &active, true, identity::LockEventReason::SoftWipe).await;
    }
    Ok(())
}

/// Attempt the identity-auto-unlock opt-in after the master key is in memory.
/// Returns `true` if the identity session is now unlocked. Cheaply skips when
/// the opt-in is off or the identity isn't passphrase-encrypted; on a missing
/// slot returns `false` (per-op auth). On a stale sealed passphrase
/// (`WRONG_PASSPHRASE` — the user changed it), self-heals by clearing the slot +
/// the opt-in so it stops auto-attempting, and returns `false`.
async fn try_identity_auto_unlock<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
) -> bool {
    // The config/identity/passphrase-slot reads below are repo-scoped —
    // operate on the active repo facade (rooted at the per-repo subdir). No
    // active repo ⇒ no identity to auto-unlock.
    let Some(active) = state.registry.active_facade() else {
        return false;
    };
    let Ok(rc) = active
        .config()
        .await
        .inspect_err(|e| log::debug!("auto-unlock: config read failed, skipping: {e}"))
    else {
        return false;
    };
    if !rc.unlock_identity_with_app {
        return false;
    }
    if !active.is_identity_encrypted().await {
        return false;
    }
    let Ok(pass_bytes) = active
        .load_app_identity_pass()
        .await
        .inspect_err(|e| log::debug!("auto-unlock: slot read failed, skipping: {e}"))
    else {
        return false; // slot absent, or the master key is somehow unavailable
    };
    // age passphrases are UTF-8; an invalid sequence means a corrupt slot.
    let Ok(s) = str::from_utf8(pass_bytes.as_slice()) else {
        log::debug!("auto-unlock: corrupt slot UTF-8, skipping");
        return false;
    };
    let pass = Zeroizing::new(s.to_owned());
    match identity::unlock_and_arm(state, app, &active, pass.as_str()).await {
        Ok(()) => true,
        Err(e) => {
            if e.code == "WRONG_PASSPHRASE" {
                log::warn!("auto-unlock: stale sealed passphrase, clearing slot");
                if let Err(cleanup) = active.clear_app_identity_pass().await {
                    log::warn!("auto-unlock: clear-app-identity-pass cleanup failed: {cleanup}");
                }
                if let Err(cleanup) = active.set_unlock_identity_with_app(false).await {
                    log::warn!(
                        "auto-unlock: set-unlock-identity-with-app cleanup failed: {cleanup}"
                    );
                }
            }
            false
        }
    }
}

/// Core gate-lock logic shared by the [`app_lock`] command (foreground-return
/// re-lock) and the gate idle timer's fire path. Wipes the **vault key** (the
/// identity becomes unreadable; the auth-free master stays keyed so the headless
/// worker can still read `repo.json`/`app.json`) and the identity cache, marks
/// the gate locked, and emits the transition with `reason` so the frontend
/// decides whether to auto-prompt.
///
/// In-flight writes are intentionally allowed to finish: they hold only the
/// already-captured identity bytes (git ops never touch the seal master key),
/// and any seal read/write racing this wipe surfaces a clean `SealKeyUnavailable`
/// (never a silent plaintext downgrade — the `ever_keyed` latch guards `seal`).
/// Do not add a mutex here to "fix" that — it would deadlock the write path.
pub(crate) fn do_app_lock<R: Runtime>(
    store: &rustpass::Store,
    app: &AppHandle<R>,
    app_locked: &AtomicBool,
    enabled: bool,
    reason: AppLockReason,
) {
    log::info!("app-lock: lock ({reason:?})");
    // R064: wipe only the vault key (the identity gate). The auth-free master
    // stays keyed so the headless worker can still read `repo.json`/`app.json`
    // under lock — the whole point of the master/vault split. Identity (under
    // `vault_seal`) becomes unreadable until the next `app_unlock` re-injects
    // the vault key.
    store.set_vault_key(None);
    store.lock();
    app_locked.store(true, Ordering::SeqCst);
    emit_app_lock_state(app, enabled, true, Some(reason));
}

/// R058 resume-timeout: the grace-aware foreground-return re-lock, called by the
/// [`app_lock`] command on every resume. A no-op unless the gate is on AND the app
/// is currently unlocked — the frontend guards both, but this is defense-in-depth
/// (e.g. a stray cold-start resume ping). A warm resume into an already-locked app
/// (the idle timer fired while away) leaves the existing overlay as-is: no re-emit,
/// so there is no emit ordering to race the idle timer's `Idle` fire. (An earlier
/// "re-emit Return" promotion was dropped — it caused a spurious cold-start ping
/// and a re-lock-after-unlock race.) If `gate_idle = After(N)` and within `N` of
/// the last activity, grace (the idle timer is NOT disarmed — total-disuse
/// semantics). Otherwise (past `N`, or `gate_idle = Off`) disarm the idle timer and
/// call [`do_app_lock`] with `Return`. Generic over `Runtime` so the host tests
/// drive it with `MockRuntime` (the `app_lock` command itself is Wry-specific).
pub(crate) fn apply_resume_relock<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    active: &rustpass::Store,
) {
    let enabled = state.app_lock_enabled.load(Ordering::SeqCst);
    if !enabled || state.app_locked.load(Ordering::SeqCst) {
        return; // gate off, or already locked → leave the existing state as-is
    }
    // R058 grace: within N of the last activity, stay unlocked. `last <= now` gates
    // grace on a genuinely-past timestamp; a future `last` (impossible for monotonic
    // Instant, but fail-safe) falls through to re-lock — the safe direction for a
    // security gate. Do NOT disarm the idle timer here: total-disuse semantics (the
    // window keeps counting toward N across the backgrounding; only a real secret op
    // through the chokepoint resets `last_activity_at`, so app-switching can't
    // extend the window).
    if let GateIdle::After(secs) = state.app_config.get().gate_idle {
        let now = std::time::Instant::now();
        let last = *state
            .last_activity_at
            .lock()
            .expect("last_activity_at poisoned");
        if last <= now && now.duration_since(last).as_secs() < secs {
            return; // grace
        }
    }
    // Past the grace window, or `gate_idle = Off`: re-lock. Disarm the idle timer
    // first so it can't fire `Idle` after this `Return` emit (prompt determinism).
    identity::disarm_gate_idle(state);
    // Wipe the entry-view cache with the app lock — a decrypted entry must not outlive
    // the vault key being dropped. (Grace-window resumes return above, before this, so
    // a resumed session keeps its cache.)
    entry_cache::soft_wipe_entry_cache(state, app, EntryCacheReason::Lock);
    // The vault-key wipe is repo-scoped — lock the active repo facade (rooted at
    // `repositories/<id>/` post-relocate, NOT the device facade `state.store`).
    do_app_lock(
        active,
        app,
        &state.app_locked,
        enabled,
        AppLockReason::Return,
    );
}

/// Lock the app from the frontend (the foreground-return re-lock path). The gate
/// idle timer calls [`do_app_lock`] directly with [`AppLockReason::Idle`].
///
/// Thin Wry command wrapper over [`apply_resume_relock`] (R058 resume-timeout).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn app_lock(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppLockError> {
    // The vault-key wipe inside is repo-scoped — lock the active repo facade
    // (rooted at `repositories/<id>/` post-relocate, NOT the device facade).
    let active = state.active_repo()?;
    apply_resume_relock(&state, &app, &active);
    Ok(())
}

/// Enable the identity-auto-unlock opt-in: validate `passphrase`, then seal it
/// under the seal master key so a later `app_unlock` can unlock the identity
/// with no second prompt. Requires the gate to be enabled and the identity to be
/// passphrase-encrypted (the slot seals a passphrase, which a plaintext identity
/// has none of). The master key must be in memory (i.e. the app is unlocked) for
/// the seal to succeed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn enable_identity_auto_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
) -> Result<(), AppLockError> {
    log::info!("identity-auto-unlock: enable");
    if !state.app_lock_enabled.load(Ordering::SeqCst) {
        return Err(AppLockError::failed(
            "Enable the app lock before identity auto-unlock",
        ));
    }
    // The identity + passphrase slot are repo-scoped — operate on the active
    // repo facade (rooted at the per-repo subdir).
    let active = state.active_repo()?;
    if !active.is_identity_encrypted().await {
        return Err(AppLockError::from(Error::new(
            ErrorCode::IdentityNotEncrypted,
            "Identity auto-unlock requires a passphrase-encrypted identity",
        )));
    }
    let passphrase = Zeroizing::new(passphrase);
    // Validate before sealing (rejects a wrong passphrase before it is stored).
    active.validate_passphrase(passphrase.as_str()).await?;
    active.save_app_identity_pass(passphrase.as_str()).await?;
    active.set_unlock_identity_with_app(true).await?;
    // Refresh the coupling flag (now true) BEFORE re-applying the identity timer
    // — the flag-before-timer ordering rule (R057). Coupled → the identity timer
    // disarms; its lifecycle now follows the gate.
    identity::refresh_security_cache(&state, &active).await;
    identity::reset_lock_timer(&state, &app, &active);
    Ok(())
}

/// Disable the identity-auto-unlock opt-in: clear the sealed passphrase slot and
/// the flag. Never fails on a missing slot (best-effort clear).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn disable_identity_auto_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppLockError> {
    log::info!("identity-auto-unlock: disable");
    let active = state.active_repo()?;
    active.clear_app_identity_pass().await?;
    active.set_unlock_identity_with_app(false).await?;
    // Refresh the coupling flag (now false) BEFORE re-applying the identity
    // timer — the flag-before-timer ordering rule (R057). Uncoupled → the
    // identity timer re-arms per LockMode.
    identity::refresh_security_cache(&state, &active).await;
    identity::reset_lock_timer(&state, &app, &active);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_lock_state_serializes() {
        let s = AppLockState {
            enabled: true,
            locked: false,
            reason: Some(AppLockReason::Idle),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"locked\":false"));
        assert!(json.contains("\"reason\":\"idle\""));
    }

    #[test]
    fn app_lock_error_from_rustpass_preserves_code() {
        let err = AppLockError::from(Error::new(ErrorCode::StoreError, "boom"));
        assert_eq!(err.code, "STORE_ERROR");
        assert_eq!(err.message, "boom");
    }

    #[test]
    fn app_lock_error_from_keystore_preserves_code() {
        let err = AppLockError::from(KeystoreError::unavailable());
        assert_eq!(err.code, "KEYSTORE_UNAVAILABLE");
    }

    #[test]
    fn failed_error_uses_app_lock_failed_code() {
        let err = AppLockError::failed("no key");
        assert_eq!(err.code, "APP_LOCK_FAILED");
        assert_eq!(err.message, "no key");
    }
}
