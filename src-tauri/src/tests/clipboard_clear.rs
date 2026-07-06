// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Clipboard-clear timer — the cancellable armed task that empties the
//! clipboard after the configured window. Mirrors the lock-timer tests in
//! `lock_state.rs`: the cancel-and-respawn pattern (`arm_clipboard_clear` /
//! `disarm_clipboard_clear`) is the load-bearing mechanism for both the
//! copy-overlap fix and the manual tap-clear bridge.

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::Manager;

use crate::AppState;
use crate::identity;
use crate::tests::{make_unlocked_state, mock_app};

/// `arm_clipboard_clear` stores a spawned task and bumps the generation.
#[tokio::test]
async fn arm_clear_sets_handle_and_bumps_generation() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    assert_eq!(
        app_state.clipboard_clear_generation.load(Ordering::SeqCst),
        0,
        "precondition: generation starts at 0"
    );
    assert!(
        app_state.clipboard_clear_handle.lock().unwrap().is_none(),
        "precondition: no armed handle"
    );

    identity::arm_clipboard_clear(&app_state, app.handle(), 60);

    assert!(
        app_state.clipboard_clear_handle.lock().unwrap().is_some(),
        "arm must store a spawned task"
    );
    assert!(
        app_state.clipboard_clear_generation.load(Ordering::SeqCst) > 0,
        "arm must bump the generation"
    );
}

/// Re-arming bumps the generation past the prior task's captured value, so a
/// stale earlier task self-disarms on wake. This is the copy-overlap fix:
/// copy-A's timer must not survive to clear copy-B's secret short of its full
/// timeout. (Belt-and-suspenders: `arm` also aborts the prior handle.)
#[tokio::test]
async fn re_arm_bumps_generation_past_prior_task() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    identity::arm_clipboard_clear(&app_state, app.handle(), 60);
    let gen_after_a = app_state.clipboard_clear_generation.load(Ordering::SeqCst);

    identity::arm_clipboard_clear(&app_state, app.handle(), 60);
    let gen_after_b = app_state.clipboard_clear_generation.load(Ordering::SeqCst);

    assert!(
        gen_after_b > gen_after_a,
        "re-arm must bump the generation so the prior (copy-A) task's captured \
         generation is stale and it self-disarms on wake — the overlap fix"
    );
    assert!(
        app_state.clipboard_clear_handle.lock().unwrap().is_some(),
        "re-arm must leave a single armed (copy-B) handle"
    );
}

/// `disarm_clipboard_clear` aborts the armed handle and bumps the generation.
/// Called on the Never path (timeout set to 0) so a stale timer from a prior
/// shorter setting can't fire and clear a clipboard the user asked to leave
/// alone.
#[tokio::test]
async fn disarm_clear_aborts_handle_and_bumps_generation() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    identity::arm_clipboard_clear(&app_state, app.handle(), 60);
    let gen_after_arm = app_state.clipboard_clear_generation.load(Ordering::SeqCst);

    identity::disarm_clipboard_clear(&app_state);

    assert!(
        app_state.clipboard_clear_handle.lock().unwrap().is_none(),
        "disarm must clear the armed handle"
    );
    assert!(
        app_state.clipboard_clear_generation.load(Ordering::SeqCst) > gen_after_arm,
        "disarm must bump the generation so any in-flight task self-disarms"
    );
}

/// A stale task (its captured generation no longer matches) self-disarms on
/// wake instead of clearing — the subtle race `abort` alone doesn't prevent.
/// Mirrors `stale_timer_self_disarms_after_rearm` in `lock_state.rs`.
#[tokio::test]
async fn stale_clipboard_task_self_disarms() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    // Task A captures generation G; the current-thread runtime parks it.
    identity::arm_clipboard_clear(&app_state, app.handle(), 0);
    let gen_a = app_state.clipboard_clear_generation.load(Ordering::SeqCst);
    // Simulate a newer arm or a manual tap-clear bumping the generation past A's
    // captured G.
    app_state
        .clipboard_clear_generation
        .fetch_add(1, Ordering::SeqCst);
    // Let A wake — it must self-disarm (gen mismatch) and leave state consistent.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        app_state.clipboard_clear_generation.load(Ordering::SeqCst),
        gen_a + 1,
        "a stale task must not bump the generation (it self-disarmed)"
    );
    // A re-arm after a stale wake still works — state isn't corrupted.
    identity::arm_clipboard_clear(&app_state, app.handle(), 60);
    assert!(
        app_state.clipboard_clear_generation.load(Ordering::SeqCst) > gen_a + 1,
        "re-arm after a stale wake must bump the generation normally"
    );
}
