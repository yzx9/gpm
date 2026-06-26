// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tests for the git cancel/progress bridge ([`crate::git`]):
//! [`crate::git::cancel_git`], the arm/disarm slot, and the progress drain's
//! exit-on-drop contract that the cancellable commands depend on.

use std::sync::atomic::Ordering;

use tauri::Manager;

use crate::AppState;
use crate::tests::{make_unlocked_state, mock_app};

/// `cancel_git` flips the armed token AND takes it out of the slot, so a second
/// cancel (or a cancel after the op settled) is a no-op rather than double-fire.
#[tokio::test]
async fn cancel_git_flips_armed_token_and_clears_slot() {
    let (state, _store) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    let token = crate::git::fresh_cancel_token();
    crate::git::arm_cancel(&app_state, token.clone());
    assert!(
        !token.load(Ordering::Relaxed),
        "token must start unset when armed"
    );

    crate::git::cancel_git(app_state).expect("cancel_git always returns Ok(())");

    assert!(
        token.load(Ordering::Relaxed),
        "cancel_git must flip the armed token so the in-flight op aborts"
    );
    assert!(
        app.state::<AppState>()
            .active_cancel_token
            .lock()
            .unwrap()
            .is_none(),
        "cancel_git must take the token out of the slot, so a second cancel is a no-op"
    );
}

/// With nothing armed, `cancel_git` succeeds and leaves the slot empty — the
/// "user taps Cancel after the op already finished" path.
#[tokio::test]
async fn cancel_git_is_noop_when_no_op_armed() {
    let (state, _store) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    crate::git::cancel_git(app_state).expect("cancel_git is Ok(()) with nothing armed");
    assert!(
        app.state::<AppState>()
            .active_cancel_token
            .lock()
            .unwrap()
            .is_none(),
        "slot stays empty when nothing was armed"
    );
}

/// The progress drain terminates once the sender is dropped — the contract the
/// cancellable commands rely on via `let _ = drain.await` to flush final events.
/// If this ever stopped holding, those commands would hang.
#[tokio::test]
async fn progress_drain_exits_when_sender_drops() {
    let (state, _store) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let (tx, drain) = crate::git::spawn_progress_drain(app.handle().clone());
    // Push one event so the drain is mid-recv when we drop the sender.
    let _ = tx.send(rustpass::GitProgress::default());
    drop(tx); // close the channel → drain's recv() returns Err → task exits

    tokio::time::timeout(std::time::Duration::from_secs(2), drain)
        .await
        .expect("drain must terminate within 2s of the sender dropping")
        .expect("drain task must not panic");
}
