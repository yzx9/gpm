// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! read-command cores — the decrypt-and-show glue that needs a live `AppState`
//! and a runtime (so it can't live in `rustpass`). Drives `show_password_core`
//! against the mock app.

use rustpass::LockMode;
use tauri::Manager;

use crate::AppState;
use crate::read;
use crate::tests::{make_unlocked_state, mock_app};

/// Under Immediate, `show_password_core` returns the secret AND soft-wipes the
/// identity afterward — the decoded secret lives in the returned
/// `SensitiveContent`, independent of the identity cache. A regression that
/// drops the wipe would leave the identity cached past the op; one that wipes
/// before the read resolves would lose the secret. The wipe must also fire on
/// the error path (covered by the `maybe_soft_wipe` tests), so the success path
/// is the remaining gap this pins down.
#[tokio::test]
async fn show_password_core_returns_secret_then_soft_wipes_under_immediate() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\nbody line")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    *app_state.lock_mode.lock().unwrap() = LockMode::Immediate;

    assert!(app_state.store.is_unlocked(), "precondition: unlocked");

    let content = read::show_password_core(&app_state, app.handle(), "foo.age")
        .await
        .expect("show should succeed");
    // `password`/`notes` are `Zeroizing<String>` — deref to compare.
    assert_eq!(&*content.password, "hunter2");
    assert_eq!(&*content.notes, "body line");

    assert!(
        !app_state.store.is_unlocked(),
        "Immediate must soft-wipe the identity after show"
    );
}
