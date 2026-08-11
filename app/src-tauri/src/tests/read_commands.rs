// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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

/// `entry_probe` decrypts once and reports TOTP-presence + attachment metadata
/// for the detail view. A normal UTF-8 entry with no TOTP and no attachment
/// probes to `Some(EntryProbe { has_totp: false, attachment: None, edit_blocked: None })`.
#[tokio::test]
async fn entry_probe_returns_metadata_when_unlocked() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    let probe = read::entry_probe(
        app.state::<AppState>(),
        app.handle().clone(),
        "foo.age".into(),
    )
    .await
    .expect("entry_probe should succeed on an unlocked store")
    .expect("a normal entry should probe to Some");
    assert!(!probe.has_totp, "no TOTP seed in the fixture");
    assert!(probe.attachment.is_none(), "no attachment in the fixture");
    assert!(probe.edit_blocked.is_none(), "a UTF-8 password is editable");
}

/// `entry_probe` must NEVER raise an unlock prompt. The fixture's identity is
/// passphrase-encrypted, so wiping the cache makes `Store::get` fail
/// `IDENTITY_ENCRYPTED`; the probe surfaces that as `Ok(None)` ("unknown")
/// before touching any lock timer. A regression that prompted would fire an
/// unlock modal off a passive probe.
#[tokio::test]
async fn entry_probe_never_prompts_when_identity_locked() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    app.state::<AppState>().store.lock(); // wipe the cached, passphrase-encrypted identity
    let probe = read::entry_probe(
        app.state::<AppState>(),
        app.handle().clone(),
        "foo.age".into(),
    )
    .await
    .expect("a locked identity is Ok(None), not an error");
    assert!(
        probe.is_none(),
        "a locked identity must probe to None, never prompt"
    );
}

/// With no TOTP seed, `copy_totp` short-circuits BEFORE the clipboard write —
/// `copied == false`, no clear scheduled — so it's testable without the
/// clipboard plugin. Pins the no-seed branch + the `cleared_after_secs: 0`
/// contract.
#[tokio::test]
async fn copy_totp_skips_clipboard_when_no_totp_seed() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    let result = read::copy_totp(
        app.state::<AppState>(),
        app.handle().clone(),
        "foo.age".into(),
        None,
    )
    .await
    .expect("copy_totp should succeed on a no-seed entry");
    assert!(!result.copied, "no TOTP seed ⇒ copied == false");
    assert_eq!(result.cleared_after_secs, 0);
    assert_eq!(result.entry_name, "foo");
}
