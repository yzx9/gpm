// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tests for the `background_sync` command (RFC R060 Tier 1).

use std::sync::atomic::Ordering;

use tauri::Manager;

use crate::AppState;
use crate::tests::{make_unlocked_state, mock_app};

/// While the `AppLock` biometric launch-gate holds the master key
/// (`app_locked`), `background_sync` skips without touching the network AND
/// never arms the shared cancel slot (it bypasses `run_cancellable`), so the
/// user's pull-to-refresh cancel stays intact. That "never arms the shared
/// cancel slot" property is the load-bearing invariant vs the manual
/// `sync_repo`, which DOES arm it. The success/emit path and the underlying
/// pull+push are covered by `rustpass`'s `sync_resolve` tests + the frontend
/// composable tests; this pins the command-layer gate + cancel-slot invariant.
#[tokio::test]
async fn background_sync_skips_while_app_locked_and_never_arms_cancel_slot() {
    let (state, _store) = make_unlocked_state(&[]).await;
    state.app_locked.store(true, Ordering::SeqCst);
    let app = mock_app(state);

    let outcome = crate::write::background_sync(app.state::<AppState>(), app.handle().clone())
        .await
        .expect("background_sync returns Ok while app-locked");
    assert!(
        outcome.is_none(),
        "background_sync must skip (Ok(None)) while app-locked"
    );
    assert!(
        app.state::<AppState>()
            .active_cancel_token
            .lock()
            .unwrap()
            .is_none(),
        "background_sync must never arm the shared cancel slot (bypasses run_cancellable)"
    );
}
