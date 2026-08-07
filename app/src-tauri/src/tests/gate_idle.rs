// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Gate in-app idle timer (R057) — the in-app idle re-lock that wipes the
//! master key + identity cache and raises the mask WITHOUT auto-prompting, plus
//! the identity-coupling rule (auto-unlock on → the identity follows the gate).
//!
//! These run on a headless [`MockRuntime`][tauri::test::MockRuntime] app and
//! drive the timer cores (`arm_gate_idle`, `reset_gate_idle_timer`,
//! `reset_lock_timer`, `maybe_soft_wipe`) directly.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustpass::LockMode;
use tauri::test::MockRuntime;
use tauri::{App, Listener, Manager};

use crate::AppState;
use crate::app_config::GateIdle;
use crate::identity;
use crate::tests::{make_unlocked_state, mock_app};

/// Capture the last `app-lock-state` payload (serialized JSON) so a test can
/// assert the `reason` tag the frontend keys its auto-prompt off of.
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

/// The gate idle timer fires after its timeout and runs `do_app_lock(Idle)`:
/// the gate locks (`app_locked`), the identity cache wipes, and the transition
/// is tagged `reason: "idle"` (so the frontend suppresses the auto-prompt).
/// Uses a 0s timeout (production uses 5+ min).
#[tokio::test]
async fn gate_idle_timer_fires_idle_lock() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    let payload = last_app_lock_payload(&app);

    identity::arm_gate_idle(&app_state, app.handle(), 0);
    // Current-thread runtime: the spawned task runs while we await.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        app_state.app_locked.load(Ordering::SeqCst),
        "idle fire must lock the gate"
    );
    assert!(
        !app_state.store.is_unlocked(),
        "idle fire must wipe the identity cache"
    );
    let p = payload.lock().unwrap().clone();
    assert!(p.contains("\"locked\":true"), "emits locked:true: {p}");
    assert!(
        p.contains("\"reason\":\"idle\""),
        "tags reason idle (suppresses auto-prompt): {p}"
    );
}

/// `reset_gate_idle_timer` arms under `After(n)` and disarms under `Off`.
#[tokio::test]
async fn gate_idle_reset_arms_and_disarms() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    // clamp_gate_idle lifts any After(<300) to the 300s floor — still "armed."
    app_state
        .app_config
        .set_gate_idle(GateIdle::After(60))
        .await
        .unwrap();
    identity::reset_gate_idle_timer(&app_state, app.handle());
    assert!(
        app_state.gate_idle_timer.is_armed(),
        "After must arm the gate idle timer"
    );

    app_state
        .app_config
        .set_gate_idle(GateIdle::Off)
        .await
        .unwrap();
    identity::reset_gate_idle_timer(&app_state, app.handle());
    assert!(
        !app_state.gate_idle_timer.is_armed(),
        "Off must disarm the gate idle timer"
    );
}

/// The gate idle timer must NOT arm when the gate is disabled — even though the
/// default `gate_idle` is `After(300)`. The helper leaves `app_lock_enabled`
/// false and the `AppConfigStore` defaults `gate_idle` to `After(300)`, mirroring a
/// fresh install. Firing `do_app_lock` here would soft-brick a session with no
/// biometric master key to retrieve (R057 regression guard).
#[tokio::test]
async fn gate_idle_not_armed_when_gate_disabled() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    identity::reset_gate_idle_timer(&app_state, app.handle());

    assert!(
        !app_state.gate_idle_timer.is_armed(),
        "gate disabled → timer must NOT arm (would soft-brick: no biometric key)"
    );
}

/// R057 coupling: with `unlock_identity_with_app` on, the identity has no
/// independent auto-lock — even under `Idle` mode, `reset_lock_timer` disarms
/// instead of arming (the gate owns the identity lifecycle).
#[tokio::test]
async fn coupled_identity_timer_stays_disarmed_under_idle() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    *app_state.lock_mode.lock().unwrap() = LockMode::Idle(300);
    app_state.identity_coupled.store(true, Ordering::SeqCst);

    identity::reset_lock_timer(&app_state, app.handle());

    assert!(
        !app_state.lock_timer.is_armed(),
        "coupled → identity timer disarmed, gate owns the lifecycle"
    );
}

/// R057 coupling: with auto-unlock on, `Immediate`'s per-op wipe is suppressed
/// too — the identity persists until the gate locks.
#[tokio::test]
async fn coupled_skips_immediate_soft_wipe() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"x\n")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    *app_state.lock_mode.lock().unwrap() = LockMode::Immediate;
    app_state.identity_coupled.store(true, Ordering::SeqCst);

    identity::maybe_soft_wipe(&app_state, app.handle()).await;

    assert!(
        app_state.store.is_unlocked(),
        "coupled → Immediate per-op wipe suppressed"
    );
}
