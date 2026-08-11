// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! GPM — age-only gopass password manager client built with Tauri v2.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use base64::Engine;
use rustpass::{LockMode, Store};
use tauri::{App, AppHandle, Emitter, Manager, Runtime, WebviewWindowBuilder, Wry};
use tauri_plugin_keystore::{Keystore, KeystoreExt};
use tokio::task::JoinHandle;

use crate::app_config::AppConfigStore;
// Re-exported so the workspace `codegen` crate can reach these IPC config
// enums (the `app_config` module is otherwise private). They cross the
// Rust↔TS boundary inside `AppConfig`; the codegen emits their TS mirrors (R085).
pub use crate::app_config::{BackgroundSyncCadence, GateIdle, SecureScreenMode};
use crate::keystore::KvKeystore;
use crate::setup::PendingIdentity;

mod app_config;
mod applock;
mod archive;
mod authenticity;
mod biometric;
mod clipboard;
mod config;
mod diagnostics_export;
mod entry_cache;
mod export_guard;
mod generator;
mod git;
mod identity;
mod jni_sync;
mod keystore;
mod logging;
mod migrations;
mod page;
mod read;
mod registry;
mod repo_export;
mod revisions;
mod setup;
mod update_check;
mod verbose;
mod write;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// Application state shared across all Tauri commands.
pub(crate) struct AppState {
    pub(crate) store: Arc<Store>,
    /// Multi-repository registry (R080): the ordered index of per-repository
    /// `Store` facades, keyed by [`registry::RepoId`]. Populated from
    /// `AppConfig.repositories` after the migration chain runs (so m0009's
    /// registered id is reflected). Repo operations resolve a facade via
    /// `state.registry.facade(&repo_id)`; `state.store` remains the device-level
    /// facade (rooted at `config_dir`, owns `app.json`) during the threading
    /// transition.
    pub(crate) registry: registry::RepoRegistry,
    /// Identity auto-lock idle timer — cancel-and-respawn with a generation-tagged
    /// self-disarm (see [`identity::IdleTimer`]). Drives the `Idle` auto-lock mode.
    pub(crate) lock_timer: identity::IdleTimer,
    /// Identity picked via the file picker, awaiting its passphrase before
    /// `complete_setup_from_file` saves it. Held only in memory (`Zeroizing` on
    /// drop); never persisted.
    pub(crate) pending_identity: Mutex<Option<PendingIdentity>>,
    /// Cached effective auto-lock mode (refreshed on unlock + the `set_*`
    /// config commands via `identity::refresh_security_cache`) so the read/write
    /// hot paths branch on a cheap mutex read instead of decrypting `repo.json`
    /// per operation.
    pub(crate) lock_mode: Mutex<LockMode>,
    /// Cached effective clipboard auto-clear seconds (same refresh contract).
    pub(crate) clipboard_clear_secs: Mutex<u64>,
    /// Clipboard auto-clear timer handle — cancel-and-respawn pattern, same
    /// shape as `lock_timer`. Holds the in-flight clear task so a fresh copy
    /// aborts the prior one (the copy-overlap fix). The manual tap-clear path
    /// doesn't abort this handle; instead the task self-skips by polling the
    /// plugin's manual-clear flag on wake (see `arm_clipboard_clear`).
    pub(crate) clipboard_clear_handle: Mutex<Option<JoinHandle<()>>>,
    /// Monotonic generation tag for the clipboard-clear timer; bumped on every
    /// (re)arm. The spawned task self-disarms if a newer arm happened while it
    /// slept.
    pub(crate) clipboard_clear_generation: Arc<AtomicU64>,
    /// Whether the app-launch biometric gate is enabled (the **vault key** —
    /// which seals the identity — is sealed in the biometric-gated Keystore).
    /// Probed at startup from the key's location and updated on enable/disable.
    /// Drives whether the frontend ever shows the app-lock overlay.
    pub(crate) app_lock_enabled: AtomicBool,
    /// Runtime app-lock state: `true` while the **vault key** is NOT in memory
    /// — cold start with the gate on, or after a background wipe. (The auth-free
    /// master key stays loaded while locked, so only the identity is gated.)
    /// Cleared by `applock::app_unlock`. Drives the frontend app-lock overlay
    /// (which suppresses the identity overlay while up, so the two never
    /// compete). `Arc` so the gate idle timer's spawned fire-task can flip it
    /// (a plain `AtomicBool` can't cross into a `'static` task).
    pub(crate) app_locked: Arc<AtomicBool>,
    /// Gate in-app idle timer (R057) — a second [`identity::IdleTimer`] that
    /// fires `applock::do_app_lock(Idle)` after the configured foreground-idle
    /// window. Armed on unlock/enable, disarmed on lock/disable; reset on
    /// activity (the same signal the identity timer consumes).
    pub(crate) gate_idle_timer: identity::IdleTimer,
    /// Last user-activity instant — a monotonic [`Instant`] (not wall-clock
    /// `SystemTime`, so an NTP/user-clock backward skew can't make the resume
    /// grace window read wider than it is). Updated at the single chokepoint
    /// [`identity::reset_gate_idle_timer`], so every secret op, `bump_idle_timer`,
    /// and unlock all flow through one update. The resume re-lock path
    /// (`applock::app_lock`) reads it to decide the R058 grace window.
    pub(crate) last_activity_at: Mutex<Instant>,
    /// Entry-view decrypted-content cache (R086): ONE in-view entry's decrypted
    /// `Secret`, held owned so it outlives the per-op identity wipe. `Arc` so the
    /// `'static` identity-idle / gate-idle timer fire tasks can wipe it on lock.
    /// None when no entry is cached (cold, or after a wipe).
    pub(crate) cached_entry: Arc<Mutex<Option<entry_cache::EntryCache>>>,
    /// View-clear idle timer for the entry cache (R086) — a third
    /// [`identity::IdleTimer`] that wipes the cached secret after `view_clear_secs`
    /// (the same value the frontend reveal timer uses). Armed on miss-populate and
    /// — Show only — on hit (the slide); disarmed on wipe / Never.
    pub(crate) entry_cache_timer: identity::IdleTimer,
    /// Cached `RepoConfig.unlock_identity_with_app`: when true the identity
    /// session has no independent auto-lock — its lifecycle follows the gate
    /// (R057 coupling). Refreshed in `refresh_security_cache` BEFORE
    /// `reset_lock_timer` reads it (the flag-before-timer ordering rule).
    pub(crate) identity_coupled: AtomicBool,
    /// One-shot state for the post-unlock legacy-envelope migrate
    /// (`0` = Pending, `1` = `InFlight`, `2` = Done). App Lock defers the master
    /// key until `app_unlock`, so the startup migrate soft-skips; the first
    /// unlock claims this and runs `migrate_seal` to convert `GPMATR1`
    /// envelopes to `GPMSEL1`. TODO: v1.0.x — remove with the legacy-magic path.
    pub(crate) seal_migrate_state: AtomicU8,
    /// One-shot state for the post-unlock storage-backend resolve
    /// (`0` = Pending, `1` = `InFlight`, `2` = Done). The backend type lives in
    /// sealed `repo.json`; the foreground defers loading the auth-free master
    /// key until `app_unlock`, so the resolve runs post-unlock — mirroring
    /// `seal_migrate_state`. On a hard failure the specific error is stashed in
    /// `Store` (not here) so `storage()` surfaces it.
    pub(crate) backend_resolve_state: AtomicU8,
    /// Cancel slot for the in-flight clone/pull/push (if any). Shared by-ref into
    /// the rustpass orchestrator so it arms UNDER `write_mu` (not before),
    /// eliminating the stomp where a queued op overwrote the running op's
    /// token. `cancel_git` `take`s/sets it.
    pub(crate) active_cancel_slot: rustpass::CancelSlot,
    /// Verbose deadline timer handle — cancel-and-respawn pattern, same shape
    /// as `clipboard_clear_handle`. Holds the in-flight revert task so a fresh
    /// arm (or a manual Off) aborts the prior one.
    pub(crate) verbose_timer: Mutex<Option<JoinHandle<()>>>,
    /// Monotonic generation tag for the verbose timer; bumped on every (re)arm
    /// and on disarm. The spawned task captures its generation and self-disarms
    /// if a newer arm happened while it slept.
    pub(crate) verbose_generation: Arc<AtomicU64>,
    /// App-shell (non-repo) preferences — screen-capture toggle, locale, and the
    /// behavior prefs (lock mode, clear timers, autosync, app-lock flag) moved
    /// here from `RepoConfig`. Persists at `app.json`; survives `reset_config`.
    /// `Arc` so the fire-and-forget update-check probe can clone a handle into
    /// its spawned task (mirrors `store: Arc<Store>`).
    pub(crate) app_config: Arc<AppConfigStore>,
    /// The Tauri app handle, so a migration that needs the Android Keystore
    /// (m0007 vault-key relocate) can reach `keystore()` without a
    /// signature change to the whole migration engine. `Some` in the live app
    /// (`init_state`), `None` on desktop and in tests (the keystore is inert /
    /// absent there, so keystore-touching migrations no-op).
    pub(crate) app_handle: Option<AppHandle>,
}

impl AppState {
    /// Resolve a repository id to its `Store` facade, or a clear not-found error.
    /// The single funnel every repo-touching command threads through:
    /// `let store = state.repo(&repo_id)?;`. The error carries no secret — only
    /// the opaque id that did not resolve (never a path, credential, or entry).
    pub(crate) fn repo(&self, id: &registry::RepoId) -> Result<Arc<Store>, rustpass::Error> {
        self.registry.facade(id).ok_or_else(|| {
            rustpass::Error::new(
                rustpass::ErrorCode::ConfigError,
                format!("unknown repository: {id}"),
            )
        })
    }
}

// ---------------------------------------------------------------------------
// At-rest master key (Android Keystore)
// ---------------------------------------------------------------------------

/// Base64 engine for the master key crossing the Rust ↔ Android-plugin IPC.
pub(crate) const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// Decode a Base64 master key (a JNI-bridge base64 string) to 32 bytes, or
/// `None` if malformed/wrong length. Scope is now the headless-sync JNI bridge
/// (`jni_sync::run_headless_sync` receives the key as a base64 string from
/// Kotlin); the live keystore read path interprets raw bytes via
/// [`interpret_key_bytes`] instead.
pub(crate) fn decode_master_key(b64: &str) -> Option<[u8; 32]> {
    let bytes: Vec<u8> = B64.decode(b64).ok()?;
    bytes.try_into().ok()
}

/// Interpret retrieved keystore bytes as a 32-byte seal key, accepting BOTH the
/// v0.17.0 on-disk format (32 raw key bytes) and the transitional v0.17.1 format
/// (the UTF-8 bytes of a base64-encoded key). Returns `None` if neither yields a
/// 32-byte key. New writes are always 32 raw bytes (the v0.17.0 format); the
/// UTF-8-of-base64 branch is read-only compatibility for keys sealed by v0.17.1.
// TODO: v1.0.0 — remove the v0.17.1 UTF-8-of-base64 read compat (the fallback
// branch below); by then no in-the-wild key uses that format.
pub(crate) fn interpret_key_bytes(bytes: &[u8]) -> Option<[u8; 32]> {
    // v0.17.0 / current format: 32 raw key bytes. (A v0.17.1 key is the UTF-8 of
    // a 44-char base64 string = 44 bytes, so 32 is unambiguously the raw form.)
    if bytes.len() == 32 {
        let mut key = [0u8; 32];
        key.copy_from_slice(bytes);
        return Some(key);
    }
    // v0.17.1 compat: the bytes are the UTF-8 of a base64-encoded key.
    let s = std::str::from_utf8(bytes).ok()?;
    let decoded: Vec<u8> = B64.decode(s).ok()?;
    decoded.try_into().ok()
}

/// Fetch the sealed master key if present — **retrieve-only, never generates**.
///
/// Returns `None` on desktop (no Keystore), if the Keystore is unavailable, if no
/// key is sealed yet, OR if a sealed key is malformed (degrade, never mint).
/// Crucially this does NOT generate on absent, so it is safe to call on the
/// upgrader path (the auth-free alias is absent pre-m0007) without minting a new
/// master that would orphan every existing envelope. First-run provisioning is
/// [`provision_master`]'s job, called explicitly by [`startup_master_key`].
async fn retrieve_master_or_none<R: Runtime>(ks: &Keystore<R>) -> Option<[u8; 32]> {
    keystore::retrieve_master(ks).await.unwrap_or(None)
}

/// Generate + seal a fresh master key (first-run provisioning).
///
/// Returns `None` on desktop (no Keystore), if generation/sealing fails, OR if a
/// key slot is already present — an **absent** slot is the only mint path. A
/// present-but-malformed slot is NOT minted over (it may have envelopes sealed
/// under it, so minting would orphan them); the caller degrades to passthrough.
/// A key that cannot be sealed is discarded rather than used unpersisted, so it
/// can never orphan later envelopes behind a key the next run won't have.
pub(crate) async fn provision_master<K: KvKeystore>(ks: &K) -> Option<[u8; 32]> {
    // Mint ONLY on an absent slot. A present slot — even a malformed one — may
    // have envelopes sealed under it; minting over it would orphan them.
    match keystore::retrieve_master(ks).await {
        Ok(None) => {}
        Ok(Some(_)) | Err(_) => return None,
    }
    let key = rustpass::seal::generate_master_key().ok()?;
    // Seal before adopting — an unpersisted key would orphan future envelopes.
    keystore::store_master(ks, &key).await.ok()?;
    Some(key)
}

/// Resolve the seal master key + app-lock state at startup.
///
/// **R074 (decision D):** the auth-free master key is loaded **always**,
/// including under App Lock — it seals the merged app config (`repo.json` +
/// `app.json`), which must be readable at `.setup()` so the pinned locale/theme
/// bake into the first-paint init scripts. This is safe because the auth-free
/// key is **not** what App Lock protects: App Lock gates the **vault key** (the
/// identity, retrieved via biometric at `app_unlock`). The auth-free key is the
/// git-credential tier — already loaded by the headless worker while locked
/// under R064 — and a process/memory attacker is an explicit non-goal. So
/// loading it at `.setup()` vs `app_unlock` is security-irrelevant.
///
/// `app_lock_enabled` is still probed (`keystore::has_app_lock_enabled`) so the frontend
/// knows whether to show the app-lock overlay; only the vault key stays deferred
/// to `app_unlock`. Returns `(master_key, app_lock_enabled)`.
async fn startup_master_key<R: Runtime>(ks: &Keystore<R>) -> (Option<[u8; 32]>, bool) {
    let app_lock_enabled = keystore::has_app_lock_enabled(ks).await;
    // Always retrieve the auth-free master (D). retrieve_master_or_none is safe on
    // the upgrader path (no generate-on-absent); provision_master is the explicit
    // first-run generate+store.
    let key = match retrieve_master_or_none(ks).await {
        Some(k) => Some(k),
        None => {
            // Provision ONLY when app-lock is off. A pre-m0007 upgrader under app
            // lock has its master in the legacy biometric alias (relocated to the
            // auth-free alias by m0007 at app_unlock), so the auth-free alias is
            // legitimately absent here — never mint over it. Such an upgrader is at
            // schema < 8, so pref.json still exists and the first-paint read is
            // unaffected; the next cold start (post-m0007) finds the auth-free key.
            if app_lock_enabled {
                None
            } else {
                provision_master(ks).await
            }
        }
    };
    (key, app_lock_enabled)
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Build the initial [`AppState`] during Tauri setup: run the one-time
/// plaintext→envelope + config migrations and assemble the state. Extracted
/// from [`run`] so the entry point stays a thin builder.
///
/// R074 (decision D): `store` (with the auth-free master key already loaded) and
/// `app_config` (already bound to the store + reloaded from the sealed config)
/// are built in `run`'s setup closure, so the pinned locale/theme bake into the
/// window's init script before it is created, and the migration engine sees a
/// keyed store from the start.
///
/// # Panics
///
/// Panics if the config directory cannot be determined (the `.expect` lives in
/// the setup closure, before this is called).
fn init_state(
    app: &App<Wry>,
    store: Arc<Store>,
    app_config: AppConfigStore,
    app_lock_enabled: bool,
) -> AppState {
    // Apply the persisted log level NOW (the sealed config is already loaded by
    // the setup closure's reload). The plugin is capped at Debug (see `run()`),
    // so this `set_max_level` is the runtime gate — a live `verbose_until` ⇒
    // Debug, else Info. Applied twice on purpose: here (the common Info case,
    // early so startup stays quiet) and again after `run_app_migrations` below,
    // so an upgrading `m0004` debug user gets Debug continuity on the first launch.
    log::set_max_level(app_config.effective_log_filter());
    // Bind the Store (idempotent — the setup closure already bound it; safe to
    // repeat so `run_app_migrations` and the post-migration reload flow through
    // the Seal regardless of call order).
    app_config.set_store(Arc::clone(&store));
    // One-time migration of any pre-existing plaintext files into the seal
    // envelope (no-op on desktop / already-wrapped). Each file is wrapped
    // atomically with a roundtrip check, so a failure leaves plaintext intact —
    // logged, non-fatal. R074/D: the auth-free master key is always loaded, so
    // this runs even under app-lock (it touches only master_seal files).
    if let Err(e) = tauri::async_runtime::block_on(store.migrate_seal()) {
        log::warn!("seal migration failed: {e}");
    }
    // Resolve the storage backend from sealed repo.json (no-op pre-setup or
    // under app-lock — the post-unlock one-shot finishes it). Best-effort, like
    // migrate_seal: a hard failure is stashed in Store for storage() to surface.
    if let Err(e) = tauri::async_runtime::block_on(store.resolve_storage()) {
        log::warn!("storage backend resolve failed: {e}");
    }
    // Resolve the crypto backend from the same sealed repo.json (no-op
    // pre-setup or under app-lock — the post-unlock one-shot finishes it).
    // Best-effort, like resolve_storage: a hard failure is stashed in Store
    // for crypto() to surface.
    if let Err(e) = tauri::async_runtime::block_on(store.resolve_crypto()) {
        log::warn!("crypto backend resolve failed: {e}");
    }

    let app_state = AppState {
        store,
        // Empty until the migration chain runs (m0009 registers the existing
        // repo's id); populated just below, after `run_app_migrations`.
        registry: registry::RepoRegistry::empty(),
        app_config: Arc::new(app_config),
        // `Some` so m0007 (vault-key relocate) can reach the Keystore. Concrete
        // `Wry` (not generic `<R>`) because `app.handle()` is `AppHandle<R>` and
        // AppState is non-generic — gpm only ever runs the default Wry runtime.
        app_handle: Some(app.handle().clone()),
        lock_timer: identity::IdleTimer::new(),
        pending_identity: Mutex::new(None),
        // Defaults until the first unlock/set refreshes them from config;
        // pre-setup no op reads them.
        lock_mode: Mutex::new(LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        clipboard_clear_handle: Mutex::new(None),
        clipboard_clear_generation: Arc::new(AtomicU64::new(0)),
        app_lock_enabled: AtomicBool::new(app_lock_enabled),
        // Locked at startup iff the gate is on (master key not yet injected).
        app_locked: Arc::new(AtomicBool::new(app_lock_enabled)),
        gate_idle_timer: identity::IdleTimer::new(),
        // `now` so the grace window is never spuriously huge before the first
        // `reset_gate_idle_timer` (unlock/activity) lands.
        last_activity_at: Mutex::new(Instant::now()),
        // R086: entry-view cache starts cold; populated on the first decrypt in
        // view, wiped on leave/lock/timer.
        cached_entry: Arc::new(Mutex::new(None)),
        entry_cache_timer: identity::IdleTimer::new(),
        // Refreshed on the first unlock/set_* (the gate is off / no identity here).
        identity_coupled: AtomicBool::new(false),
        // Legacy-envelope migrate pending; only consumed on the App-Lock path
        // (first app_unlock). Stays Pending on non-app-lock/desktop (no unlock).
        seal_migrate_state: AtomicU8::new(0),
        backend_resolve_state: AtomicU8::new(0),
        active_cancel_slot: Arc::new(Mutex::new(None)),
        verbose_timer: Mutex::new(None),
        verbose_generation: Arc::new(AtomicU64::new(0)),
    };
    // Copy the app-scoped behavior prefs out of a pre-split repo.json into
    // app.json (no-op once migrated; soft-skips under app-lock — retried on
    // app_unlock). Safe at startup when the master key is available (no app-lock).
    tauri::async_runtime::block_on(migrations::run_app_migrations(&app_state));
    // Re-apply the log level after migrations: `m0004_verbose_from_debug` may
    // have just carried a pinned "debug" into `verbose_until` (so the upgrading
    // user gets Debug on this launch, not Info), and an already-expired deadline
    // is cleared off disk. Best-effort — the early apply above already lowered
    // the gate for the common case; this corrects it post-migration.
    let _ = tauri::async_runtime::block_on(app_state.app_config.clear_expired_verbose())
        .map_err(|e| log::warn!("app-config: clear_expired_verbose failed: {e}"));
    log::set_max_level(app_state.app_config.effective_log_filter());
    // Arm the mid-session revert timer if a verbose window is still live (a
    // relaunch inside the window keeps capturing at Debug, then auto-reverts).
    verbose::arm_verbose_timer(&app_state, app.handle());
    // Reload the sealed config + reseed the Store's injected `autosync`
    // so a cold start sees the persisted values. R074/D: the auth-free master key
    // is always loaded (even under app-lock), so the sealed merged config is
    // readable here unconditionally — no app-lock guard. (The setup closure's
    // reload already populated the caches; this is a defensive re-seed covering
    // the no-migration cold start + post-migration refresh.)
    tauri::async_runtime::block_on(app_state.app_config.reload_behavior()).ok();
    app_state
        .store
        .set_autosync(app_state.app_config.get_behavior().autosync);
    // Populate the multi-repository registry from the (now-migrated) `AppConfig`:
    // m0009 has registered the existing repo's id into `repositories`/`last_active`.
    // Until the relocate migration lands, the facade root is still `config_dir`
    // (the single repo's historical location), so every entry shares the device
    // store. One repo ⇒ behavior identical to today.
    {
        let device_store = Arc::clone(&app_state.store);
        let cfg = app_state.app_config.get();
        let ids = cfg
            .repositories
            .iter()
            .map(|s| registry::RepoId::from(s.clone()))
            .collect::<Vec<_>>();
        let last_active = cfg.last_active.map(registry::RepoId::from);
        app_state
            .registry
            .populate(ids, last_active, move |_id| Arc::clone(&device_store));
    }
    // Self-heal the setup half-state: if `register_first_repo` failed mid-setup
    // (after identity/config were persisted but before the registry entry + id
    // landed in `app.json`), the registry is empty here. Fresh installs (no
    // repo.json yet) skip — only a configured store with an empty registry is
    // the recoverable half-state. Same path setup uses; idempotent once
    // `repositories` is non-empty (populate then fills the registry normally).
    if app_state.registry.is_empty() {
        let reconciled = tauri::async_runtime::block_on(async {
            // No repo.json ⇒ fresh install / pre-setup, nothing to register.
            if app_state.store.config().await.is_err() {
                return Ok(());
            }
            setup::register_first_repo(&app_state).await
        });
        if let Err(e) = reconciled {
            log::warn!("startup repo reconciliation failed: {e}");
        }
    }
    app_state
}

/// The app-owned headless worker FQN the worker-agnostic scheduler
/// instantiates by name (R077). Lives in the app's Android source set
/// (`xyz.yzx9.gpm.SyncWorker`), not the plugin.
#[cfg(target_os = "android")]
const SYNC_WORKER_CLASS: &str = "xyz.yzx9.gpm.SyncWorker";

/// The WorkManager unique-work name for the periodic background sync. Kept as a
/// stable literal for update continuity: the post-update
/// `enqueueUniquePeriodicWork(REPLACE)` matches the previously scheduled work
/// under this exact name and rewrites its spec, rather than orphaning it. R077
/// made the plugin worker- and name-agnostic; this is the one app-specific
/// value the app passes in.
#[cfg(target_os = "android")]
const SYNC_WORK_NAME: &str = "gpm_background_sync";

/// Re-apply the periodic background-sync schedule from `cadence`. Called
/// on app setup (once the cadence is loaded) and whenever the cadence changes
/// (the `set_background_sync` command). On Android: enqueues/replaces the
/// `WorkManager` periodic work (or cancels it when `Off`), passing `config_dir`
/// through as `InputData` so the Worker never reconstructs the path. On
/// other targets: a no-op (the foreground sync covers desktop). Best-effort —
/// errors are swallowed (a missed reschedule keeps the previous cadence).
#[allow(clippy::unused_async)] // the Android branch awaits; the desktop branch is a no-op.
pub(crate) async fn reschedule_background_sync<R: Runtime>(
    app: &AppHandle<R>,
    cadence: BackgroundSyncCadence,
) {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_background_work::BackgroundWorkExt;
        let sched = app.background_work_sched();
        match cadence.hours() {
            Some(hours) => match app.path().app_config_dir() {
                Ok(config_dir) => {
                    sched
                        .schedule(
                            hours,
                            config_dir.to_string_lossy().into_owned(),
                            SYNC_WORKER_CLASS.to_string(),
                            SYNC_WORK_NAME.to_string(),
                        )
                        .await;
                }
                Err(e) => log::warn!("bg-sync: config dir unavailable; not rescheduling: {e}"),
            },
            None => sched.cancel(SYNC_WORK_NAME.to_string()).await,
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        // Desktop: no WorkManager; the foreground sync covers convergence.
        let _ = (app, cadence);
    }
}

/// Cancel the periodic background-sync schedule (called when `AutoSync`
/// is turned off, since background sync is linked to `AutoSync`). No-op off-Android.
#[allow(clippy::unused_async)] // the Android branch awaits; desktop is a no-op.
pub(crate) async fn cancel_background_sync<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_background_work::BackgroundWorkExt;
        app.background_work_sched()
            .cancel(SYNC_WORK_NAME.to_string())
            .await;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
    }
}

/// Application entry point.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::too_many_lines)] // length is the command-registration list, not logic
pub fn run() {
    // libgit2's connect/server timeouts are C globals that must be set before
    // any thread is spawned, so configure them first — before the Tauri/tokio
    // runtime starts. Bounds the git handshake/transfer hang (R034).
    rustpass::storage::git::init_server_timeouts();
    tauri::Builder::default()
        // Logger is registered first so every subsequent plugin/setup line can
        // emit to the rotated file + Android logcat. `Stdout` auto-routes to
        // logcat on Android (there is no separate Logcat target in v2); `LogDir`
        // writes a rotated file under app_log_dir(). Capped at Debug, not Trace:
        // gpm emits no trace-level records, so Trace only admits third-party
        // chatter — the `jni` crate logs every JNI method lookup/call at TRACE and
        // would flood the file (notably during the startup window, before
        // `init_state` lowers the runtime `log::set_max_level` gate to the
        // persisted level — Info by default). The Debug ceiling takes effect the
        // instant the plugin initializes, so trace is excluded at every phase.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .level(log::LevelFilter::Debug)
                .max_file_size(1_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseUtc)
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_safe_area::init())
        .plugin(tauri_plugin_keystore::init())
        .plugin(tauri_plugin_file_picker::init())
        .plugin(tauri_plugin_device_info::init())
        .plugin(tauri_plugin_file_save::init())
        .plugin(tauri_plugin_screen_secure::init())
        .plugin(tauri_plugin_clipboard_notify::init())
        .plugin(tauri_plugin_background_work::init())
        .plugin(tauri_plugin_opener::init())
        // OS notifications (tauri-plugin-notification). POST_NOTIFICATIONS is
        // already declared via the clipboard-notify manifest merge, so this
        // adds no new permission prompt. Used by the frontend for the unsolicited
        // verbose notices (boot-still-active, deadline-reverted).
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Cold-start banner FIRST: version (the thing bug reports most need
            // to pin), then build profile + target. Emitted before the keystore
            // await below so a hang there still leaves a breadcrumb.
            log::info!(
                "gpm {} starting ({} {}/{})",
                env!("CARGO_PKG_VERSION"),
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
                std::env::consts::OS,
                std::env::consts::ARCH,
            );
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("Cannot determine app config directory");
            // R074 (decision D): load the auth-free master key + build the Store
            // HERE, before reading the app config, so the sealed merged config is
            // readable at first paint. The auth-free key is NOT what App Lock
            // protects (it gates the vault key / identity); loading it here — even
            // under App Lock — is safe: it is the git-credential tier, already
            // worker-loaded while locked, and a process attacker is a non-goal.
            let (master_key, app_lock_enabled) =
                tauri::async_runtime::block_on(startup_master_key(app.keystore()));
            // Basic-state summary — config dir (where the rotated log + sealed
            // config live) and whether the app-launch biometric gate is armed.
            log::info!(
                "startup: config_dir={}, app_lock={}",
                config_dir.display(),
                app_lock_enabled,
            );
            let store = Arc::new(Store::new(config_dir.clone(), master_key));
            let app_config = tauri::async_runtime::block_on(AppConfigStore::new(&config_dir));
            app_config.set_store(Arc::clone(&store));
            // Load the sealed config into the caches so the init scripts below
            // bake the PINNED locale/theme (new world: the merged sealed app.json;
            // old world: pref.json + the behavior slot), not cold-start defaults.
            // This is what preserves the first-paint fix (e3c7df6/cfadbb5) without
            // any post-unlock re-apply: the config is readable at .setup().
            if let Err(e) = tauri::async_runtime::block_on(app_config.reload()) {
                log::warn!("app-config: startup reload failed: {e}");
            }
            let theme_script =
                app_config::theme_init_script(app_config.get_pref().theme_mode.as_deref());
            let locale_script = app_config::locale_init_script(&app_config.resolved_locale());
            // `create: false` in tauri.conf.json keeps Tauri from auto-creating
            // the main window; build it here with the per-window init scripts.
            // Both the locale (pinned-or-system, from `resolved_locale`) and the
            // theme script are registered per-window — composed from the sealed
            // config just above (unreadable at Tauri `Builder` time on Android).
            // Any future `WebviewWindowBuilder` must chain both. The "main" label
            // must match the capabilities scope in capabilities/{default,mobile}.json.
            let main_window = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .expect("main window config missing (tauri.conf.json)");
            WebviewWindowBuilder::from_config(app.handle(), main_window)?
                .initialization_script(theme_script)
                .initialization_script(locale_script)
                .build()?;
            let state = init_state(app, store, app_config, app_lock_enabled);
            // Apply the persisted background-sync cadence on launch
            // (enqueue/cancel the WorkManager periodic work).
            #[cfg(target_os = "android")]
            let cadence = state.app_config.background_sync();
            // RFC R090: clone the config-store handle + read the toggle before
            // `state` moves into manage; the probe is spawned fire-and-forget
            // below and writes its result back through the store's
            // `write_mu`-guarded RMW, so it can't race a concurrent ack/toggle.
            let app_config = Arc::clone(&state.app_config);
            let update_check_enabled = state.app_config.get().update_check_enabled;
            app.manage(state);
            // Best-effort: clear any attachment stage stranded by a hard-killed
            // prior export (StageGuard's Drop runs on panic/cancel, not SIGKILL).
            tauri::async_runtime::block_on(read::sweep_attachment_stage(app.handle()));
            // Likewise clear any stranded repository-export stages.
            tauri::async_runtime::block_on(repo_export::sweep_repo_export_stage(app.handle()));
            #[cfg(target_os = "android")]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::block_on(reschedule_background_sync(&handle, cadence));
            }
            // RFC R090: passively probe for a newer release (≤1/day, gated on
            // the pref, fire-and-forget). The dots read `app.json`; a slow probe
            // updates it for next time. Platform-agnostic — not Android-gated.
            if update_check_enabled {
                // Fire-and-forget: the handle is intentionally dropped (the task
                // runs to completion on the runtime). `_`-prefixed so it reads as
                // unused without `let _ =` (which clippy flags on a future).
                let _update_check = tauri::async_runtime::spawn(update_check::run_once(app_config));
            }
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
            setup::create_gpg_store,
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
            identity::bump_idle_timer,
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
            read::copy_totp,
            read::entry_probe,
            read::export_attachment,
            read::entry_oid,
            // entry-view cache (R086): frontend wipes on leave/switch.
            read::wipe_entry_cache,
            clipboard::copy_generated_password,
            clipboard::are_clipboard_notifications_enabled,
            clipboard::request_clipboard_notifications_permission,
            clipboard::open_clipboard_notification_settings,
            // revisions
            revisions::list_revisions,
            revisions::show_revision,
            revisions::copy_revision,
            // generator
            generator::generate_password,
            generator::generate_password_batch,
            // write / sync
            write::pull_repo,
            write::sync_repo,
            write::background_sync,
            git::cancel_git,
            write::push_repo,
            write::resolve_sync_divergence,
            write::resolve_entry_conflict,
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
            config::set_pat,
            config::clear_ssh_key,
            config::verify_git_auth,
            config::set_lock_mode,
            config::set_gate_idle,
            config::set_view_clear_secs,
            config::set_clipboard_clear_secs,
            config::set_autosync,
            config::set_background_sync,
            config::consume_sync_attention,
            config::get_commit_identity_default,
            config::reset_config,
            // app config: screen-capture master toggle + display language + theme + platform availability
            app_config::get_app_config,
            app_config::set_secure_screen_mode,
            app_config::set_locale_pref,
            app_config::resolved_locale,
            app_config::set_theme_mode,
            app_config::set_verbose,
            app_config::screen_secure_available,
            app_config::runtime_platform,
            // update check (RFC R090): passive release-availability detection.
            update_check::get_update_status,
            update_check::acknowledge_update,
            update_check::set_update_check,
            update_check::check_update_now,
            // logging: in-app diagnostics viewer + the verbose (Debug) toggle.
            // The level is applied at startup via effective_log_filter in
            // init_state; `set_verbose` re-applies it within a session.
            logging::read_log,
            logging::clear_log,
            logging::write_log,
            // diagnostics export bundle (full log + redacted config + device info).
            diagnostics_export::export_diagnostics,
            // repository export archive (R078): full-history git bundle + manifest + README.
            repo_export::export_repository,
            // biometric
            biometric::is_biometric_available,
            biometric::open_security_settings,
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
            authenticity::add_trusted_signing_key,
            authenticity::import_trusted_gpg_key_file,
            authenticity::remove_trusted_key,
            authenticity::remove_trusted_gpg_key,
            authenticity::get_gpg_key_parse_warnings,
            authenticity::trust_head_signer,
            authenticity::trust_commit_signer,
            authenticity::ignore_commit_issue,
            authenticity::list_commit_signatures,
            authenticity::get_commit_signature,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(on_run_event);
}

/// The global event emitted to the frontend when the app returns to the
/// foreground. Sourced from [`tauri::RunEvent::Resumed`] — which tao documents
/// as "Android: triggered by `onResume` of the Activity," i.e. the platform-
/// guaranteed foreground transition, below the `WebView` (whose `visibilitychange`
/// is OEM-unreliable). The frontend's resume triggers — the app-lock re-lock,
/// the foreground sync, and the permissions re-probe — listen for this instead
/// of the DOM event (R029). MUST match the `"app-resumed"` literal the frontend
/// listens for in `app/src/api/appLifecycle.ts`.
const APP_RESUME_EVENT: &str = "app-resumed";

/// The frontend-facing event for a run event, if any. Only [`RunEvent::Resumed`]
/// is bridged to the `WebView` (the authoritative foreground signal); every other
/// run event stays Rust-side. Pure so it is host-testable without a Tauri
/// runtime — the emit itself is a trivial `app.emit` in [`on_run_event`].
fn frontend_resume_event(event: &tauri::RunEvent) -> Option<&'static str> {
    match event {
        tauri::RunEvent::Resumed => Some(APP_RESUME_EVENT),
        _ => None,
    }
}

/// Event-loop observer: logs app lifecycle transitions so a diagnostic trace
/// records when the app started, returned to the foreground, lost/regained
/// window focus (backgrounding, biometric prompts, system dialogs), and exited.
/// Also bridges the authoritative foreground signal to the frontend: on
/// [`tauri::RunEvent::Resumed`] it emits [`APP_RESUME_EVENT`], which the resume
/// triggers listen for instead of the OEM-unreliable `visibilitychange` (R029).
///
/// `Focused(false)` on Android also fires for in-activity system windows
/// (biometric prompt, permission dialog), so read it as "lost window focus," not
/// strictly "backgrounded"; the `Resumed` event is the reliable foreground signal.
#[allow(clippy::needless_pass_by_value)] // signature dictated by `App::run`'s callback contract
fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: tauri::RunEvent) {
    if let Some(name) = frontend_resume_event(&event) {
        let _ = app.emit(name, ());
    }
    match event {
        tauri::RunEvent::Resumed => log::info!("app: resumed (foreground)"),
        tauri::RunEvent::ExitRequested { code, .. } => {
            log::info!("app: exit requested (code: {code:?})");
        }
        tauri::RunEvent::Exit => log::info!("app: exited"),
        tauri::RunEvent::WindowEvent {
            event: tauri::WindowEvent::Focused(focused),
            ..
        } => log::info!(
            "app: window focus {}",
            if focused { "gained" } else { "lost" }
        ),
        _ => {}
    }
}

#[cfg(test)]
mod resume_event_tests {
    use super::{APP_RESUME_EVENT, frontend_resume_event};

    /// `Resumed` is the one run event bridged to the frontend. Pins both the
    /// variant AND the event name so a Rust↔TS drift fails here, not as a silent
    /// "resume never re-locks" at runtime.
    #[test]
    fn resumed_bridges_to_app_resume_event() {
        assert_eq!(
            frontend_resume_event(&tauri::RunEvent::Resumed),
            Some(APP_RESUME_EVENT)
        );
        assert_eq!(APP_RESUME_EVENT, "app-resumed");
    }

    /// Non-resume run events stay Rust-side (no spurious frontend resume).
    #[test]
    fn non_resumed_events_are_not_bridged() {
        assert_eq!(frontend_resume_event(&tauri::RunEvent::Exit), None);
    }
}

#[cfg(test)]
mod decode_master_key_tests {
    use super::*;

    #[test]
    fn valid_32_byte_key_roundtrips() {
        let key = rustpass::seal::generate_master_key().unwrap();
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

#[cfg(test)]
mod interpret_key_bytes_tests {
    use super::*;

    #[test]
    fn thirty_two_raw_bytes_is_a_key() {
        // v0.17.0 / current format: the raw key bytes.
        let key = rustpass::seal::generate_master_key().unwrap();
        assert_eq!(interpret_key_bytes(&key), Some(key));
    }

    #[test]
    fn utf8_of_base64_is_a_key_v0171_compat() {
        // the v0.17.1 on-disk form — the UTF-8 bytes of a base64 key — is
        // read back via the fallback, not rejected.
        let key = rustpass::seal::generate_master_key().unwrap();
        let bytes = B64.encode(key).into_bytes();
        assert_eq!(interpret_key_bytes(&bytes), Some(key));
    }

    #[test]
    fn non_key_bytes_are_none() {
        // Neither 32 raw bytes nor a base64-of-32 ⇒ None (the caller maps this
        // to KEYSTORE_MALFORMED).
        assert_eq!(interpret_key_bytes(&[0u8; 7]), None); // too short; NUL isn't base64
        assert_eq!(interpret_key_bytes(&[]), None);
        // Valid base64 but decodes to != 32 bytes ("aaaa" → 3 bytes).
        assert_eq!(interpret_key_bytes(b"aaaa"), None);
        // Invalid UTF-8 ⇒ the from_utf8().ok()? None branch (a non-32-length slot).
        assert_eq!(interpret_key_bytes(&[0xff, 0xfe]), None);
    }
}
