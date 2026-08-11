// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R058 resume-timeout — the grace-aware foreground-return re-lock
//! ([`apply_resume_relock`]) and the `last_activity_at` chokepoint in
//! [`identity::reset_gate_idle_timer`].
//!
//! These run on a headless [`MockRuntime`][tauri::test::MockRuntime] app and
//! drive `apply_resume_relock` directly (the `app_lock` command is a thin Wry
//! wrapper over it). Elapsed time is simulated by writing a past/now value into
//! `last_activity_at` rather than waiting, since `clamp_gate_idle` floors `After`
//! at 300s.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::test::MockRuntime;
use tauri::{App, Listener, Manager};

use crate::app_config::GateIdle;
use crate::identity;
use crate::tests::{make_unlocked_state, mock_app};
use crate::{AppState, applock};

/// Capture the last `app-lock-state` payload (serialized JSON) so a test can
/// assert the `reason` tag. Mirrors `tests/gate_idle::last_app_lock_payload`.
fn last_app_lock_payload(app: &App<MockRuntime>) -> Arc<Mutex<String>> {
    let payload = Arc::new(Mutex::new(String::new()));
    let payload_clone = payload.clone();
    app.listen("app-lock-state", move |e| {
        if let Ok(mut p) = payload_clone.lock() {
            *p = e.payload().to_string();
        }
    });
    payload
}

/// `gate_idle = After(N)` and a return within N of the last activity stays
/// unlocked (grace). The idle timer is NOT disarmed — total-disuse semantics.
#[tokio::test]
async fn resume_within_window_stays_unlocked() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    app_state
        .app_config
        .set_gate_idle(GateIdle::After(300))
        .await
        .unwrap();
    // Arms the timer AND stamps last_activity_at = now (the chokepoint).
    identity::reset_gate_idle_timer(&app_state, app.handle());
    assert!(app_state.gate_idle_timer.is_armed());

    applock::apply_resume_relock(&app_state, app.handle());

    assert!(
        !app_state.app_locked.load(Ordering::SeqCst),
        "within the grace window the app must stay unlocked"
    );
    assert!(
        app_state.gate_idle_timer.is_armed(),
        "grace must NOT disarm the idle timer (total-disuse continues)"
    );
}

/// `gate_idle = After(N)` and a return past N re-locks with `Return` (auto-prompt)
/// and disarms the idle timer so it can't fire `Idle` afterwards.
#[tokio::test]
async fn resume_past_window_relocks_with_return() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    let payload = last_app_lock_payload(&app);

    app_state
        .app_config
        .set_gate_idle(GateIdle::After(300))
        .await
        .unwrap();
    identity::reset_gate_idle_timer(&app_state, app.handle());
    // Simulate "last activity 400s ago" (N=300) — past the grace window.
    *app_state.last_activity_at.lock().unwrap() = Instant::now()
        .checked_sub(Duration::from_secs(400))
        .unwrap();

    applock::apply_resume_relock(&app_state, app.handle());

    assert!(
        app_state.app_locked.load(Ordering::SeqCst),
        "past the grace window the app must re-lock"
    );
    let p = payload.lock().unwrap().clone();
    assert!(
        p.contains("\"reason\":\"return\""),
        "past-window re-lock tags reason return (auto-prompt): {p}"
    );
    assert!(
        !app_state.gate_idle_timer.is_armed(),
        "re-lock must disarm the idle timer (no late Idle emit)"
    );
}

/// `gate_idle = Off` keeps today's every-resume behavior: always re-lock.
#[tokio::test]
async fn resume_off_always_relocks() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    let payload = last_app_lock_payload(&app);

    app_state
        .app_config
        .set_gate_idle(GateIdle::Off)
        .await
        .unwrap();
    // last_activity_at = now (so a grace branch, if it wrongly applied to Off,
    // would keep it unlocked — asserting it locks regardless pins Off ≠ grace).
    *app_state.last_activity_at.lock().unwrap() = Instant::now();

    applock::apply_resume_relock(&app_state, app.handle());

    assert!(
        app_state.app_locked.load(Ordering::SeqCst),
        "Off must re-lock on every resume (today's behavior, m0006 users)"
    );
    let p = payload.lock().unwrap().clone();
    assert!(
        p.contains("\"reason\":\"return\""),
        "Off re-lock tags reason return: {p}"
    );
}

/// A warm resume into an already-locked app (the idle timer fired while away) is a
/// no-op: the existing overlay stays as-is, the stale `Idle` reason is preserved
/// (the user taps). The earlier "re-emit Return" promotion was dropped — it caused
/// a spurious cold-start ping + a re-lock-after-unlock race (R058 review).
#[tokio::test]
async fn resume_already_locked_is_noop() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    let payload = last_app_lock_payload(&app);

    app_state
        .app_config
        .set_gate_idle(GateIdle::After(300))
        .await
        .unwrap();
    // Fire the idle timer (0s) → do_app_lock(Idle) → locked, reason idle.
    identity::arm_gate_idle(&app_state, app.handle(), 0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(app_state.app_locked.load(Ordering::SeqCst));
    assert!(
        payload.lock().unwrap().contains("\"reason\":\"idle\""),
        "precondition: the idle timer fired Idle"
    );

    // The return into an already-locked app: no-op (no re-emit, no state change).
    applock::apply_resume_relock(&app_state, app.handle());

    assert!(
        app_state.app_locked.load(Ordering::SeqCst),
        "still locked (no-op)"
    );
    let p = payload.lock().unwrap().clone();
    assert!(
        p.contains("\"reason\":\"idle\""),
        "already-locked resume does NOT re-emit Return (stale Idle preserved): {p}"
    );
}

/// The `last_activity_at` chokepoint: `reset_gate_idle_timer` (called directly by
/// every secret op, bypassing `bump_idle_timer`) updates the timestamp — so the
/// resume grace check stays in lockstep with the timer. Pins R058 P1-2.
#[tokio::test]
async fn reset_gate_idle_timer_stamps_last_activity_chokepoint() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    app_state
        .app_config
        .set_gate_idle(GateIdle::After(300))
        .await
        .unwrap();
    // Simulate a stale timestamp (as if no activity had been recorded yet).
    *app_state.last_activity_at.lock().unwrap() = Instant::now()
        .checked_sub(Duration::from_secs(400))
        .unwrap();

    // A secret op resets the timer directly through this chokepoint.
    identity::reset_gate_idle_timer(&app_state, app.handle());

    let stamped = *app_state.last_activity_at.lock().unwrap();
    let drift = Instant::now().saturating_duration_since(stamped).as_secs();
    assert!(
        drift < 5,
        "reset_gate_idle_timer must stamp last_activity_at (~now, within 5s): drift={drift}s"
    );
}

/// The grace boundary is a strict `<`: elapsed == N re-locks, elapsed N-1 graces.
/// Pins the off-by-one so a future `<=` widening or truncation change can't slip by.
#[tokio::test]
async fn resume_grace_boundary_is_strict_less_than() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    app_state
        .app_config
        .set_gate_idle(GateIdle::After(300))
        .await
        .unwrap();

    // elapsed == N (as_secs truncates sub-second jitter) → NOT < N → re-lock.
    *app_state.last_activity_at.lock().unwrap() =
        Instant::now().checked_sub(Duration::from_mins(5)).unwrap();
    applock::apply_resume_relock(&app_state, app.handle());
    assert!(
        app_state.app_locked.load(Ordering::SeqCst),
        "elapsed==N must re-lock (strict <)"
    );

    // Reset and try N-1 → < N → grace.
    app_state.app_locked.store(false, Ordering::SeqCst);
    *app_state.last_activity_at.lock().unwrap() = Instant::now()
        .checked_sub(Duration::from_secs(299))
        .unwrap();
    applock::apply_resume_relock(&app_state, app.handle());
    assert!(
        !app_state.app_locked.load(Ordering::SeqCst),
        "elapsed N-1 must grace"
    );
}

/// The grace branch must NOT stamp `last_activity_at` — total-disuse semantics
/// (R058: app-switching can't reset the window; only a real secret op through the
/// chokepoint can). A regression that wrongly re-stamped here would re-enable
/// lock-evasion by rapid app-switching and pass every other test.
#[tokio::test]
async fn resume_grace_does_not_reset_last_activity() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    app_state
        .app_config
        .set_gate_idle(GateIdle::After(300))
        .await
        .unwrap();
    let stale_at = Instant::now()
        .checked_sub(Duration::from_secs(100))
        .unwrap();
    *app_state.last_activity_at.lock().unwrap() = stale_at;

    applock::apply_resume_relock(&app_state, app.handle()); // grace (100 < 300)
    assert!(!app_state.app_locked.load(Ordering::SeqCst), "must grace");

    let after = *app_state.last_activity_at.lock().unwrap();
    let drift = stale_at.saturating_duration_since(after).as_secs()
        + after.saturating_duration_since(stale_at).as_secs();
    assert!(
        drift < 5,
        "grace must NOT reset last_activity_at (total-disuse): drift={drift}s"
    );
}

/// A future `last_activity_at` (a monotonic-clock anomaly — impossible in practice,
/// but fail-safe) must NOT grant grace: `last <= now` gates it, so it falls through
/// to re-lock. Pins the safe direction (a switch to saturating-without-the-guard
/// would silently fail open).
#[tokio::test]
async fn resume_future_last_activity_relocks_safe_direction() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    app_state
        .app_config
        .set_gate_idle(GateIdle::After(300))
        .await
        .unwrap();
    *app_state.last_activity_at.lock().unwrap() = Instant::now() + Duration::from_mins(1);

    applock::apply_resume_relock(&app_state, app.handle());
    assert!(
        app_state.app_locked.load(Ordering::SeqCst),
        "future last (clock anomaly) must re-lock, not grace — fail-safe direction"
    );
}
