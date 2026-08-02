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

use std::sync::atomic::{self, AtomicBool, AtomicU8, AtomicU64};
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
mod diagnostics_export;
mod export_guard;
mod generator;
mod git;
mod identity;
mod jni_sync;
mod logging;
mod migrations;
mod page;
mod read;
mod revisions;
mod setup;
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
    /// Identity auto-lock idle timer — cancel-and-respawn with a generation-tagged
    /// self-disarm (see [`identity::IdleTimer`]). Drives the `Idle` auto-lock mode.
    pub(crate) lock_timer: identity::IdleTimer,
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
    /// Whether the app-launch biometric gate is enabled (the seal master key
    /// is sealed in the biometric-gated Keystore). Probed at startup from the
    /// key's location and updated on enable/disable. Drives whether the frontend
    /// ever shows the app-lock overlay.
    pub(crate) app_lock_enabled: AtomicBool,
    /// Runtime app-lock state: `true` while the master key is NOT in memory —
    /// cold start with the gate on, or after a background wipe. Cleared by
    /// `applock::app_unlock`. Drives the frontend app-lock overlay (which
    /// suppresses the identity overlay while up, so the two never compete).
    /// `Arc` so the gate idle timer's spawned fire-task can flip it (a plain
    /// `AtomicBool` can't cross into a `'static` task).
    pub(crate) app_locked: Arc<AtomicBool>,
    /// Gate in-app idle timer (R057) — a second [`identity::IdleTimer`] that
    /// fires `applock::do_app_lock(Idle)` after the configured foreground-idle
    /// window. Armed on unlock/enable, disarmed on lock/disable; reset on
    /// activity (the same signal the identity timer consumes).
    pub(crate) gate_idle_timer: identity::IdleTimer,
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
    /// sealed `repo.json` (unreadable until app unlock), so the resolve runs
    /// post-unlock — mirroring `seal_migrate_state`. On a hard failure the
    /// specific error is stashed in `Store` (not here) so `storage()` surfaces it.
    pub(crate) backend_resolve_state: AtomicU8,
    /// Cancel slot for the in-flight clone/pull/push (if any). Shared by-ref into
    /// the rustpass orchestrator so it arms UNDER `write_mu` (not before),
    /// eliminating the pre-R032 stomp where a queued op overwrote the running
    /// op's token. `cancel_git` `take`s/sets it.
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
    pub(crate) app_config: app_config::AppConfigStore,
    /// The Tauri app handle, so a migration that needs the Android Keystore
    /// (m0007 vault-key relocate) can reach `secure_keystore()` without a
    /// signature change to the whole migration engine. `Some` in the live app
    /// (`init_state`), `None` on desktop and in tests (the keystore is inert /
    /// absent there, so keystore-touching migrations no-op).
    pub(crate) app_handle: Option<tauri::AppHandle>,
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

/// Fetch the sealed master key if present — **retrieve-only, never generates**.
///
/// Returns `None` on desktop (no Keystore), if the Keystore is unavailable, or if no
/// key is sealed yet. Crucially this does NOT generate on absent, so it is safe to call
/// on the upgrader path (the auth-free alias is absent pre-m0007) without minting a new
/// master that would orphan every existing envelope. First-run provisioning is
/// [`provision_master`]'s job, called explicitly by [`startup_master_key`].
async fn retrieve_master_or_none<R: tauri::Runtime>(
    ks: &tauri_plugin_secure_keystore::SecureKeystore<R>,
) -> Option<[u8; 32]> {
    if !ks.is_available().await.unwrap_or(false) {
        return None;
    }
    let b64 = ks.retrieve().await.unwrap_or(None)?;
    decode_master_key(&b64)
}

/// Generate + seal a fresh master key (first-run provisioning).
///
/// Returns `None` on desktop (no Keystore) or if generation/sealing fails. A key that
/// cannot be sealed is discarded rather than used unpersisted, so it can never orphan
/// later envelopes behind a key the next run won't have.
async fn provision_master<R: tauri::Runtime>(
    ks: &tauri_plugin_secure_keystore::SecureKeystore<R>,
) -> Option<[u8; 32]> {
    if !ks.is_available().await.unwrap_or(false) {
        return None;
    }
    // Never overwrite an existing entry: a present entry (even a malformed one)
    // may have envelopes sealed under it, so minting a fresh key would orphan
    // them. Degrade to passthrough instead — this restores the pre-split self-heal
    // (a garbled decode used to return None without touching the entry).
    if ks.retrieve().await.unwrap_or(None).is_some() {
        return None;
    }
    let key = rustpass::seal::generate_master_key().ok()?;
    // Seal before adopting — an unpersisted key would orphan future envelopes.
    ks.store(&B64.encode(key)).await.ok()?;
    Some(key)
}

/// Resolve the seal master key + app-lock state at startup.
///
/// When a biometric-gated master key exists (the app-launch gate is on), the
/// key is deliberately NOT loaded here — it is injected after the app-unlock
/// biometric prompt — so `repo.json` stays unreadable until the user
/// authenticates. Otherwise the auth-free master key loads silently (the
/// pre-app-lock path). Returns `(master_key, app_lock_enabled)`.
async fn startup_master_key<R: tauri::Runtime>(
    ks: &tauri_plugin_secure_keystore::SecureKeystore<R>,
) -> (Option<[u8; 32]>, bool) {
    if ks.has_stored_biometric().await.unwrap_or(false) {
        (None, true)
    } else {
        // Auth-free path: retrieve the sealed master, provisioning on first run.
        // retrieve_master_or_none is safe on the upgrader path (no generate-on-absent);
        // provision_master is the explicit first-run generate+store (None on desktop).
        let key = match retrieve_master_or_none(ks).await {
            Some(k) => Some(k),
            None => provision_master(ks).await,
        };
        (key, false)
    }
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Build the initial [`AppState`] during Tauri setup: resolve the config dir,
/// load (or defer, when app-lock is on) the seal master key, run the one-time
/// plaintext→envelope migration, and assemble the state. Extracted from
/// [`run`] so the entry point stays a thin builder.
///
/// # Panics
///
/// Panics if the config directory cannot be determined.
fn init_state(app: &tauri::App<tauri::Wry>) -> AppState {
    // Cold-start banner: version first (the thing bug reports most need to
    // pin), then build profile + target so a trace distinguishes dev vs
    // release and android vs desktop at a glance. Emitted BEFORE the keystore
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

    // At-rest master key + app-lock state. When the biometric-gated master key
    // exists (app-lock on), the key is NOT loaded here — it is injected after
    // the app-unlock biometric prompt — so `repo.json` stays unreadable until
    // the user authenticates on launch/resume. Otherwise the auth-free master
    // key loads silently (the pre-app-lock path).
    let (master_key, app_lock_enabled) =
        tauri::async_runtime::block_on(startup_master_key(app.secure_keystore()));
    // Basic-state summary — config dir (where the rotated log + sealed config
    // live) and whether the app-launch biometric gate is armed. Logged here,
    // before `config_dir` moves into `Store`, so a trace lands the paths once.
    log::info!(
        "startup: config_dir={}, app_lock={}",
        config_dir.display(),
        app_lock_enabled,
    );
    // App-shell (non-repo) preferences — primarily the screen-capture master
    // toggle. Borrows `config_dir` before it is moved into `Store` below.
    let app_config = app_config::AppConfigStore::new(&config_dir);
    // Apply the persisted log level NOW (right after app.json loads and the log
    // plugin has initialized). The plugin is capped at Debug (see `run()`), so
    // this `set_max_level` is the runtime gate — a live `verbose_until` ⇒ Debug,
    // else Info. Applied twice on purpose: here (the common Info case, early so
    // startup stays quiet) and again after `run_app_migrations` below, so an
    // upgrading `m0004` debug user gets Debug continuity on the first launch.
    log::set_max_level(app_config.effective_log_filter());
    let store = Arc::new(Store::new(config_dir, master_key));
    // Two-phase binding: the AppConfigStore setters/readers for the sealed
    // behavior slot need the Store ref, but the Store can't be constructed
    // until the config_dir is known. Bind it now (before the AppState move) so
    // `run_app_migrations` and the post-migration reload below can flow through
    // the Seal (sealed on Android, plaintext-passthrough on desktop).
    app_config.set_store(Arc::clone(&store));
    // One-time migration of any pre-existing plaintext files into the seal
    // envelope (no-op on desktop / already-wrapped). Each file is wrapped
    // atomically with a roundtrip check, so a failure leaves plaintext intact —
    // logged, non-fatal. With app-lock on the master key is absent here, so
    // this is a no-op over the existing envelopes.
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
        app_config,
        // `Some` so m0007 (vault-key relocate) can reach the Keystore. Concrete
        // `Wry` (not generic `<R>`) because `app.handle()` is `AppHandle<R>` and
        // AppState is non-generic — gpm only ever runs the default Wry runtime.
        app_handle: Some(app.handle().clone()),
        lock_timer: identity::IdleTimer::new(),
        pending_identity: Mutex::new(None),
        // Defaults until the first unlock/set refreshes them from config;
        // pre-setup no op reads them.
        lock_mode: Mutex::new(rustpass::LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        clipboard_clear_handle: Mutex::new(None),
        clipboard_clear_generation: Arc::new(AtomicU64::new(0)),
        app_lock_enabled: AtomicBool::new(app_lock_enabled),
        // Locked at startup iff the gate is on (master key not yet injected).
        app_locked: Arc::new(AtomicBool::new(app_lock_enabled)),
        gate_idle_timer: identity::IdleTimer::new(),
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
    // Reload the sealed behavior cache + reseed the Store's injected `autosync`
    // so a cold start (where the behavior cache started at defaults) sees the
    // persisted values. Skipped under app-lock (the load soft-fails to defaults;
    // the app_unlock path runs its own reload + reseed after biometric injects
    // the key). Best-effort.
    if !app_state.app_lock_enabled.load(atomic::Ordering::SeqCst) {
        tauri::async_runtime::block_on(app_state.app_config.reload_behavior()).ok();
        app_state
            .store
            .set_autosync(app_state.app_config.get_behavior().autosync);
    }
    app_state
}

/// Re-apply the periodic background-sync schedule from `cadence`. Called
/// on app setup (once the cadence is loaded) and whenever the cadence changes
/// (the `set_background_sync` command). On Android: enqueues/replaces the
/// `WorkManager` periodic work (or cancels it when `Off`), passing `config_dir`
/// through as `InputData` so the Worker never reconstructs the path. On
/// other targets: a no-op (the foreground sync covers desktop). Best-effort —
/// errors are swallowed (a missed reschedule keeps the previous cadence).
#[allow(clippy::unused_async)] // the Android branch awaits; the desktop branch is a no-op.
pub(crate) async fn reschedule_background_sync<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    cadence: app_config::BackgroundSyncCadence,
) {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_background_sync::BackgroundSyncExt;
        let sched = app.background_sync_sched();
        match cadence.hours() {
            Some(hours) => match app.path().app_config_dir() {
                Ok(config_dir) => {
                    sched
                        .schedule(hours, config_dir.to_string_lossy().into_owned())
                        .await;
                }
                Err(e) => log::warn!("bg-sync: config dir unavailable; not rescheduling: {e}"),
            },
            None => sched.cancel().await,
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
pub(crate) async fn cancel_background_sync<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_background_sync::BackgroundSyncExt;
        app.background_sync_sched().cancel().await;
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
        .plugin(tauri_plugin_biometric_keystore::init())
        .plugin(tauri_plugin_secure_keystore::init())
        .plugin(tauri_plugin_file_picker::init())
        .plugin(tauri_plugin_device_info::init())
        .plugin(tauri_plugin_file_save::init())
        .plugin(tauri_plugin_screen_secure::init())
        .plugin(tauri_plugin_clipboard_notify::init())
        .plugin(tauri_plugin_background_sync::init())
        .plugin(tauri_plugin_opener::init())
        // OS notifications (tauri-plugin-notification). POST_NOTIFICATIONS is
        // already declared via the clipboard-notify manifest merge, so this
        // adds no new permission prompt. Used by the frontend for the unsolicited
        // verbose notices (boot-still-active, deadline-reverted).
        .plugin(tauri_plugin_notification::init())
        // Best-effort display language baked in pre-paint; `resolved_locale` IPC reconciles a pinned preference after mount (see `app_config`).
        .append_invoke_initialization_script(app_config::locale_init_script())
        .setup(|app| {
            let state = init_state(app);
            // Apply the persisted background-sync cadence on launch
            // (enqueue/cancel the WorkManager periodic work).
            #[cfg(target_os = "android")]
            let cadence = state.app_config.background_sync();
            app.manage(state);
            // Best-effort: clear any attachment stage stranded by a hard-killed
            // prior export (StageGuard's Drop runs on panic/cancel, not SIGKILL).
            read::sweep_attachment_stage(app.handle());
            #[cfg(target_os = "android")]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::block_on(reschedule_background_sync(&handle, cadence));
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
            // logging: in-app diagnostics viewer + the verbose (Debug) toggle.
            // The level is applied at startup via effective_log_filter in
            // init_state; `set_verbose` re-applies it within a session.
            logging::read_log,
            logging::clear_log,
            logging::write_log,
            // diagnostics export bundle (full log + redacted config + device info).
            diagnostics_export::export_diagnostics,
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
        .run(log_run_event);
}

/// Event-loop observer: logs app lifecycle transitions so a diagnostic trace
/// records when the app started, returned to the foreground, lost/regained
/// window focus (backgrounding, biometric prompts, system dialogs), and exited.
/// Pure observation — never blocks or mutates state. `Focused(false)` on Android
/// also fires for in-activity system windows (biometric prompt, permission
/// dialog), so read it as "lost window focus," not strictly "backgrounded"; the
/// `Resumed` event is the reliable foreground signal.
#[allow(clippy::needless_pass_by_value)] // signature dictated by `App::run`'s callback contract
fn log_run_event<R: tauri::Runtime>(_app: &tauri::AppHandle<R>, event: tauri::RunEvent) {
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
