// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Lock state machine & auto-lock timer — the security-critical glue that
//! `rustpass` can't test (it stops at `Store::lock`).
//!
//! These run on a headless [`MockRuntime`][tauri::test::MockRuntime] app and
//! drive the runtime-generic command cores (`do_lock`, `arm_lock`) directly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustpass::LockMode;
use tauri::test::MockRuntime;
use tauri::{App, Listener, Manager};

use crate::AppState;
use crate::identity;
use crate::tests::{make_unlocked_state, mock_app};

/// `do_lock` wipes the identity cache so a modal left open across an auto-lock
/// can't keep the identity alive.
#[tokio::test]
async fn do_lock_wipes_cache() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    assert!(app_state.store.is_unlocked(), "precondition: unlocked");

    identity::do_lock(&app_state, app.handle(), &app_state.store).await;

    assert!(
        !app_state.store.is_unlocked(),
        "lock must wipe the identity cache"
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
    identity::do_lock(&app_state, app.handle(), &app_state.store).await;

    assert!(
        fired.load(Ordering::SeqCst),
        "lock must emit identity-lock-state"
    );
}

/// The auto-lock timer fires after its timeout and locks the store. Uses a 0s
/// timeout (production uses 5 min).
#[tokio::test]
async fn auto_lock_timer_locks() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    assert!(app_state.store.is_unlocked(), "precondition: unlocked");

    identity::arm_lock(&app_state, app.handle(), 0, &app_state.store);
    // Current-thread runtime: the spawned task runs while we await.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(!app_state.store.is_unlocked(), "timer must lock the store");
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

    // Task A captures generation G; the current-thread runtime parks it.
    identity::arm_lock(&app_state, app.handle(), 0, &app_state.store);
    // Simulate a newer arm racing ahead (bumps generation past A's captured G).
    app_state
        .lock_timer
        .generation
        .fetch_add(1, Ordering::SeqCst);
    // Let A wake — it must self-disarm.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        app_state.store.is_unlocked(),
        "a stale timer must not lock the store"
    );
}

// ── no-cache (Immediate) mode: soft wipe ─────────────────────────────────

/// Helper: set the cached lock mode on a managed `AppState`.
fn set_lock_mode(app: &App<MockRuntime>, mode: LockMode) {
    let app_state = app.state::<AppState>();
    *app_state.lock_mode.lock().unwrap() = mode;
}

/// `soft_wipe` empties the identity cache but, unlike a hard lock, does not
/// raise the unlock overlay (the frontend leaves a just-revealed secret on
/// screen until its own view-clear timer).
#[tokio::test]
async fn soft_wipe_empties_cache() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"x\n")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    identity::soft_wipe(&app_state, app.handle(), &app_state.store).await;

    assert!(
        !app_state.store.is_unlocked(),
        "soft wipe must empty the identity cache"
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
    identity::maybe_soft_wipe(&app_state, app.handle(), &app_state.store).await;
    assert!(
        !app_state.store.is_unlocked(),
        "Immediate mode must wipe the identity after an op"
    );
}

/// `maybe_soft_wipe` is a no-op under Idle (the session stays cached).
#[tokio::test]
async fn maybe_soft_wipe_noop_under_idle() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    set_lock_mode(&app, LockMode::Idle(300));
    let app_state = app.state::<AppState>();

    identity::maybe_soft_wipe(&app_state, app.handle(), &app_state.store).await;
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
        identity::reset_lock_timer(&app_state, app.handle(), &app_state.store);
        assert!(
            !app_state.lock_timer.is_armed(),
            "{mode:?} must not arm an idle timer"
        );
    }

    set_lock_mode(&app, LockMode::Idle(60));
    identity::reset_lock_timer(&app_state, app.handle(), &app_state.store);
    assert!(
        app_state.lock_timer.is_armed(),
        "Idle must arm an idle timer"
    );
}

// ── lock reason (drives the frontend's auto-biometric-prompt decision) ────

/// Capture the last `identity-lock-state` payload (as serialized JSON) so a test
/// can assert the `reason` tag the frontend keys its auto-prompt off of.
fn last_lock_payload(app: &App<MockRuntime>) -> Arc<Mutex<String>> {
    let payload = Arc::new(Mutex::new(String::new()));
    let payload_clone = payload.clone();
    app.listen("identity-lock-state", move |e| {
        if let Ok(mut p) = payload_clone.lock() {
            *p = e.payload().to_string();
        }
    });
    payload
}

/// A manual lock tags its event `reason: "manual"` so the frontend keeps
/// auto-prompting biometric (the user is present and wants back in).
#[tokio::test]
async fn do_lock_tags_reason_manual() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"x\n")]).await;
    let app = mock_app(state);
    let payload = last_lock_payload(&app);

    let app_state = app.state::<AppState>();
    identity::do_lock(&app_state, app.handle(), &app_state.store).await;

    let p = payload.lock().unwrap().clone();
    assert!(p.contains("\"locked\":true"), "payload: {p}");
    assert!(
        p.contains("\"reason\":\"manual\""),
        "manual lock tags reason=manual: {p}"
    );
}

/// The idle timer tags its event `reason: "idle"` so the frontend suppresses the
/// auto biometric prompt — the user likely stepped away, so a pre-emptive prompt
/// would just expire before they return.
#[tokio::test]
async fn auto_lock_timer_tags_reason_idle() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let payload = last_lock_payload(&app);

    let app_state = app.state::<AppState>();
    identity::arm_lock(&app_state, app.handle(), 0, &app_state.store);
    // Current-thread runtime: the spawned task runs while we await.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let p = payload.lock().unwrap().clone();
    assert!(p.contains("\"locked\":true"), "payload: {p}");
    assert!(
        p.contains("\"reason\":\"idle\""),
        "idle timer tags reason=idle: {p}"
    );
}

/// A soft wipe tags its event `reason: "soft-wipe"`. The value is unused by the
/// frontend (the soft branch returns before reading it) but kept honest on the
/// wire — and this pins the kebab-case serialization of `SoftWipe`.
#[tokio::test]
async fn soft_wipe_tags_reason_soft_wipe() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let payload = last_lock_payload(&app);

    let app_state = app.state::<AppState>();
    identity::soft_wipe(&app_state, app.handle(), &app_state.store).await;

    let p = payload.lock().unwrap().clone();
    assert!(
        p.contains("\"reason\":\"soft-wipe\""),
        "soft wipe tags reason=soft-wipe: {p}"
    );
}
