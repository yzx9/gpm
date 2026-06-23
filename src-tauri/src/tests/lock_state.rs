// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Lock state machine & auto-lock timer — the security-critical glue that
//! `rustpass` can't test (it stops at `Store::lock`).
//!
//! These run on a headless [`MockRuntime`][tauri::test::MockRuntime] app and
//! drive the runtime-generic command cores (`do_lock`, `arm_lock`) directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rustpass::LockMode;
use tauri::{Listener, Manager};

use crate::AppState;
use crate::identity;
use crate::tests::{make_unlocked_state, mock_app};
use crate::write;

/// `do_lock` wipes the identity cache and drops any stashed conflict plaintext
/// so a modal left open can't survive behind a wiped identity.
#[tokio::test]
async fn do_lock_wipes_cache_and_clears_pending() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    assert!(app_state.store.is_unlocked(), "precondition: unlocked");
    write::stash_pending(&app_state.pending_write, "foo", b"hunter2".to_vec());
    assert!(
        app_state.pending_write.lock().unwrap().is_some(),
        "precondition: plaintext stashed"
    );

    identity::do_lock(&app_state, app.handle()).await;

    assert!(
        !app_state.store.is_unlocked(),
        "lock must wipe the identity cache"
    );
    assert!(
        app_state.pending_write.lock().unwrap().is_none(),
        "lock must clear a stashed conflict plaintext"
    );
}

/// `do_lock` emits `identity-lock-state` so the frontend mirrors the backend
/// (the frontend must never decide lock state on its own).
#[tokio::test]
async fn do_lock_emits_locked_state() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"x\n")]).await;
    let app = mock_app(state);

    let fired = Arc::new(AtomicBool::new(false));
    let fired_clone = fired.clone();
    app.listen("identity-lock-state", move |_| {
        fired_clone.store(true, Ordering::SeqCst);
    });

    let app_state = app.state::<AppState>();
    identity::do_lock(&app_state, app.handle()).await;

    assert!(
        fired.load(Ordering::SeqCst),
        "lock must emit identity-lock-state"
    );
}

/// The auto-lock timer fires after its timeout: locks the store and clears a
/// stashed conflict plaintext. Uses a 0s timeout (production uses 5 min).
#[tokio::test]
async fn auto_lock_timer_locks_and_clears_pending() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    assert!(app_state.store.is_unlocked(), "precondition: unlocked");
    write::stash_pending(&app_state.pending_write, "foo", b"secret".to_vec());

    identity::arm_lock(&app_state, app.handle(), 0);
    // Current-thread runtime: the spawned task runs while we await.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(!app_state.store.is_unlocked(), "timer must lock the store");
    assert!(
        app_state.pending_write.lock().unwrap().is_none(),
        "timer must clear a stashed plaintext"
    );
}

/// A stale timer (an older `arm` whose generation has since been bumped) must
/// self-disarm instead of locking right after a fresh unlock — the subtle race
/// `abort` alone doesn't prevent. Deterministic on the current-thread runtime:
/// task A is parked until the test awaits, by which point generation has moved.
#[tokio::test]
async fn stale_timer_self_disarms_after_rearm() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    write::stash_pending(&app_state.pending_write, "foo", b"secret".to_vec());

    // Task A captures generation G; the current-thread runtime parks it.
    identity::arm_lock(&app_state, app.handle(), 0);
    // Simulate a newer arm racing ahead (bumps generation past A's captured G).
    app_state.lock_generation.fetch_add(1, Ordering::SeqCst);
    // Let A wake — it must self-disarm.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        app_state.store.is_unlocked(),
        "a stale timer must not lock the store"
    );
    assert!(
        app_state.pending_write.lock().unwrap().is_some(),
        "a stale timer must not clear a stashed plaintext"
    );
}

// ── no-cache (Immediate) mode: soft wipe ─────────────────────────────────

/// Helper: set the cached lock mode on a managed `AppState`.
fn set_lock_mode(app: &tauri::App<tauri::test::MockRuntime>, mode: LockMode) {
    let app_state = app.state::<AppState>();
    *app_state.lock_mode.lock().unwrap() = mode;
}

/// `soft_wipe` empties the identity cache but, unlike a hard lock, leaves a
/// stashed conflict plaintext in place (it must stay behind a live identity for
/// resolve — only the hard lock / timer clear it).
#[tokio::test]
async fn soft_wipe_empties_cache_but_keeps_pending() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"x\n")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    write::stash_pending(&app_state.pending_write, "foo", b"secret".to_vec());

    identity::soft_wipe(&app_state, app.handle()).await;

    assert!(
        !app_state.store.is_unlocked(),
        "soft wipe must empty the identity cache"
    );
    assert!(
        app_state.pending_write.lock().unwrap().is_some(),
        "soft wipe must NOT clear a stashed plaintext (only a hard lock does)"
    );
}

/// `maybe_soft_wipe` under Immediate wipes the identity after an op.
#[tokio::test]
async fn maybe_soft_wipe_wipes_under_immediate() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    set_lock_mode(&app, LockMode::Immediate);
    let app_state = app.state::<AppState>();

    assert!(app_state.store.is_unlocked(), "precondition: unlocked");
    identity::maybe_soft_wipe(&app_state, app.handle()).await;
    assert!(
        !app_state.store.is_unlocked(),
        "Immediate + no pending conflict must wipe the identity"
    );
}

/// While a write conflict is pending, `maybe_soft_wipe` must NOT wipe the
/// identity — the stashed plaintext is replayed by `resolve_write_conflict`,
/// which needs the identity. A regression here leaves an undecryptable stash
/// behind a wiped cache (or breaks resolve). This is the security-critical
/// invariant of the conflict-stash design.
#[tokio::test]
async fn maybe_soft_wipe_skips_while_conflict_pending() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    set_lock_mode(&app, LockMode::Immediate);
    let app_state = app.state::<AppState>();

    write::stash_pending(&app_state.pending_write, "foo", b"secret".to_vec());
    assert!(app_state.store.is_unlocked(), "precondition: unlocked");

    identity::maybe_soft_wipe(&app_state, app.handle()).await;

    assert!(
        app_state.store.is_unlocked(),
        "must NOT wipe the identity while a conflict is pending"
    );
    assert!(
        app_state.pending_write.lock().unwrap().is_some(),
        "stash must remain for resolve"
    );
}

/// `maybe_soft_wipe` is a no-op under Idle (the session stays cached).
#[tokio::test]
async fn maybe_soft_wipe_noop_under_idle() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    set_lock_mode(&app, LockMode::Idle(300));
    let app_state = app.state::<AppState>();

    identity::maybe_soft_wipe(&app_state, app.handle()).await;
    assert!(
        app_state.store.is_unlocked(),
        "Idle mode must keep the identity cached"
    );
}

/// `reset_lock_timer` reads the cached mode: Immediate and Never disarm (no idle
/// timer armed); Idle arms one.
#[tokio::test]
async fn reset_lock_timer_branches_on_mode() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    for mode in [LockMode::Immediate, LockMode::Never] {
        set_lock_mode(&app, mode);
        identity::reset_lock_timer(&app_state, app.handle());
        assert!(
            app_state.lock_timer.lock().unwrap().is_none(),
            "{mode:?} must not arm an idle timer"
        );
    }

    set_lock_mode(&app, LockMode::Idle(60));
    identity::reset_lock_timer(&app_state, app.handle());
    assert!(
        app_state.lock_timer.lock().unwrap().is_some(),
        "Idle must arm an idle timer"
    );
}
