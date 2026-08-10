// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Entry-view decrypted-content cache (R086).
//!
//! Caches ONE entry's decrypted [`Secret`] for the view window so a single
//! unlock opens the whole detail view (copy → show → copy-2FA) without
//! re-prompting. The identity is wiped after the one decrypt; only the
//! already-decrypted content lingers, scoped to the in-view entry and the
//! `view_clear_secs` window. See `docs/rfcs/R086-entry-view-decrypted-cache.md`.
//!
//! Mirrors identity's lock plumbing (`arm_lock` / `disarm_lock` /
//! `reset_lock_timer` / `soft_wipe` / `emit_lock_state`) but reuses
//! [`crate::identity::IdleTimer`] rather than introducing a shared generic
//! primitive (rule of three: two uses today). The cache lives in [`AppState`]
//! as `Arc<Mutex<Option<EntryCache>>>` + an [`crate::identity::IdleTimer`]; the
//! `Arc` is so the `'static` timer-fire closures (identity-idle, gate-idle)
//! can wipe it on lock.

use std::sync::{Arc, Mutex};

use rustpass::Secret;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use zeroize::Zeroizing;

use crate::AppState;

// ---------------------------------------------------------------------------
// Cache entry + state events
// ---------------------------------------------------------------------------

/// The cached decrypted content for the one entry currently in view.
///
/// [`Secret`] is NOT `Clone` (Zeroizing owned fields), so it is held owned here
/// and consumers borrow it under the cache mutex via `with_secret`
/// (`read::with_secret`) — projecting owned values out, never cloning.
pub(crate) struct EntryCache {
    /// The entry this cache is scoped to. A different entry ⇒ a MISS (the caller
    /// re-decrypts), so only the in-view entry is ever cached. `Zeroizing` so the
    /// path (which mirrors the secret's identity) is wiped with the rest.
    pub(crate) entry_path: Zeroizing<String>,
    /// The blob oid captured atomically with the decrypt — the freshness token
    /// `with_secret` re-checks on every hit: a background/manual sync that
    /// changed the entry invalidates the cache (mismatch ⇒ re-decrypt).
    pub(crate) oid: String,
    /// The decrypted secret. Owned (not borrowed) so it outlives the identity
    /// wipe that follows the decrypt. NOT wrapped in `Zeroizing` (`Secret` does
    /// not impl `Zeroize`) — but its fields are themselves `Zeroizing<Vec<u8>>`,
    /// so dropping the `Secret` on eviction zeros its password/body/attributes.
    pub(crate) secret: Secret,
}

/// Why the cache state just transitioned — surfaced so the frontend mirrors the
/// backend from both the warm (miss-populate) and wipe sides (timer/lock/leave).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EntryCacheReason {
    /// A miss just populated the cache (a decrypt happened, identity since wiped).
    Warmed,
    /// The view-clear timer expired.
    Timer,
    /// A hard lock (identity or app) wiped it.
    Lock,
    /// The user left or switched the entry.
    Leave,
}

/// Cache-state event payload. `cached == true` on warm, `false` on wipe.
#[derive(Debug, Clone, Copy, Serialize)]
struct EntryCacheState {
    cached: bool,
    reason: EntryCacheReason,
}

/// Emit the cache state as `entry-cache-warmed` (`cached == true`) or
/// `entry-cache-wiped` (`cached == false`) — two symmetric events so the
/// frontend mirrors cache state from both transitions (D9).
pub(crate) fn emit_entry_cache_state<R: Runtime>(
    app: &AppHandle<R>,
    cached: bool,
    reason: EntryCacheReason,
) {
    let event = if cached {
        "entry-cache-warmed"
    } else {
        "entry-cache-wiped"
    };
    let _ = app.emit(event, EntryCacheState { cached, reason });
}

// ---------------------------------------------------------------------------
// Timer plumbing (mirrors identity::arm_lock / disarm_lock / reset_lock_timer)
// ---------------------------------------------------------------------------

/// Disarm the cache timer. Called on wipe so a stale timer can't fire into an
/// already-empty cache. Mirrors [`crate::identity::disarm_lock`].
pub(crate) fn disarm_entry_cache(state: &State<'_, AppState>) {
    state.entry_cache_timer.disarm();
}

/// (Re)arm the cache timer to fire after `secs`, replacing any in-flight timer.
/// On fire it wipes the cached secret (the view window elapsed) and emits
/// `entry-cache-wiped(Timer)`. Mirrors [`crate::identity::arm_lock`].
pub(crate) fn arm_entry_cache<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    secs: u64,
) {
    let cached_entry = Arc::clone(&state.cached_entry);
    let app_handle = app.clone();
    state.entry_cache_timer.arm(secs, move || async move {
        if wipe_inner(&cached_entry) {
            emit_entry_cache_state(&app_handle, false, EntryCacheReason::Timer);
        }
    });
}

/// Reset the cache timer per the cached `view_clear_secs`: `Some(n)` arms an
/// `n`-second timer; `None` (defaults to [`rustpass::config::DEFAULT_VIEW_CLEAR_SECS`])
/// arms that; `Some(0)` (Never) disarms — the cache then persists for the view
/// session until leave/lock, matching identity's `Never`. Mirrors
/// [`crate::identity::reset_lock_timer`]. Called on a miss-populate (initial arm)
/// and — only for Show — on a hit (the slide; D8).
pub(crate) fn reset_entry_cache_timer<R: Runtime>(state: &State<'_, AppState>, app: &AppHandle<R>) {
    let secs = state
        .app_config
        .get()
        .view_clear_secs
        .unwrap_or(rustpass::config::DEFAULT_VIEW_CLEAR_SECS);
    if secs == 0 {
        disarm_entry_cache(state);
    } else {
        arm_entry_cache(state, app, secs);
    }
}

// ---------------------------------------------------------------------------
// Wipe (mirrors identity::soft_wipe; runs on every identity/key-wipe path)
// ---------------------------------------------------------------------------

/// Take the cached secret (drop in place) and report whether anything was wiped.
/// Poison-recovery: a poisoned mutex is recovered into the guard so the wipe
/// still happens — a thread that panicked with the lock must not strand the
/// decrypted secret.
fn wipe_inner(cached_entry: &Arc<Mutex<Option<EntryCache>>>) -> bool {
    let mut guard = cached_entry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.take().is_some()
}

/// Wipe the cache from a context that holds the full [`AppState`]: disarm the
/// timer, take (drop) the cached secret, and emit `entry-cache-wiped(reason)`.
/// Called on the identity hard-lock ([`crate::identity::do_lock`]), the app-lock
/// command, and the frontend leave/switch IPC. Idempotent — emits nothing if the
/// cache was already empty.
pub(crate) fn soft_wipe_entry_cache<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    reason: EntryCacheReason,
) {
    disarm_entry_cache(state);
    if wipe_inner(&state.cached_entry) {
        emit_entry_cache_state(app, false, reason);
    }
}

/// Wipe the cache from a `'static` context (the identity-idle and gate-idle
/// timer fire tasks) that holds an `Arc` clone of the cache but not the full
/// [`AppState`] — so it cannot disarm the cache timer. The timer either
/// self-cancels on the next arm or fires harmlessly into an already-empty cache
/// (its own `on_fire` is a no-op wipe). Used alongside [`crate::applock::do_app_lock`]
/// and the identity-idle lock so a decrypted secret never outlives the key that
/// decrypted it.
pub(crate) fn wipe_entry_cache_arc<R: Runtime>(
    cached_entry: &Arc<Mutex<Option<EntryCache>>>,
    app: &AppHandle<R>,
    reason: EntryCacheReason,
) {
    if wipe_inner(cached_entry) {
        emit_entry_cache_state(app, false, reason);
    }
}
