// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests for the app-lock gate on credential-using git WRITE commands.
//!
//! App Lock wipes the vault key but leaves the auth-free master key resident,
//! so `repo.json` (the git credential) stays decryptable while locked. Without
//! a command-layer gate a locked app could still publish — `create`/`delete`
//! need no identity and `push` is a pure git op. Each gated command must refuse
//! with `APP_LOCKED` as its first action, before any store/git work; reads
//! (pull/clone/verify) stay allowed. These mirror `background_sync`'s locked
//! test but assert an `Err` (user-initiated ops) rather than `Ok(None)` (the
//! best-effort headless skip contract — see `require_unlocked`'s doc comment).

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use rustpass::{DivergenceChoice, EntryConflictChoice, ExpectedKind};
use tauri::test::MockRuntime;
use tauri::{App, Manager};

use crate::AppState;
use crate::tests::{make_unlocked_state, mock_app};
use crate::write::SecretParts;

/// Build an unlocked state, flip `app_locked` on, and wrap it in a mock app.
/// Each gated command refuses at its first statement, so no real remote or
/// identity is needed — the bare `make_unlocked_state` fixture suffices.
async fn locked_mock_app() -> App<MockRuntime> {
    let (state, _store) = make_unlocked_state(&[]).await;
    state.app_locked.store(true, Ordering::SeqCst);
    mock_app(state)
}

/// Build minimal structured edit/resolve parts via serde (the fields are
/// private to `write`, but the gate fires before they're touched).
fn empty_parts() -> SecretParts {
    serde_json::from_value(serde_json::json!({
        "password": "",
        "attributes": [],
        "body": "",
    }))
    .expect("SecretParts deserializes from empty parts")
}

#[tokio::test]
async fn create_secret_refused_while_app_locked() {
    let app = locked_mock_app().await;
    let err = crate::write::create_secret(
        app.state::<AppState>(),
        app.handle().clone(),
        "test/locked".into(),
        "body".into(),
    )
    .await
    .expect_err("create_secret must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn create_from_preset_secret_refused_while_app_locked() {
    // The gate is the first statement, before `find_preset`, so any preset id
    // (even an unknown one) is fine — the gate short-circuits.
    let app = locked_mock_app().await;
    let err = crate::write::create_from_preset_secret(
        app.state::<AppState>(),
        app.handle().clone(),
        "nonexistent-preset".into(),
        HashMap::new(),
    )
    .await
    .expect_err("create_from_preset_secret must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn edit_secret_refused_while_app_locked() {
    let app = locked_mock_app().await;
    let err = crate::write::edit_secret(
        app.state::<AppState>(),
        app.handle().clone(),
        "test/locked".into(),
        empty_parts(),
        None,
    )
    .await
    .expect_err("edit_secret must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn delete_secret_refused_while_app_locked() {
    // delete_secret bypasses do_save and calls autosync_write_command directly,
    // so a do_save-only gate would miss it — this pins the command-top gate.
    let app = locked_mock_app().await;
    let err = crate::write::delete_secret(
        app.state::<AppState>(),
        app.handle().clone(),
        "test/locked".into(),
        None,
    )
    .await
    .expect_err("delete_secret must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn sync_repo_refused_while_app_locked() {
    // sync_repo is user-initiated (the Sync button), so it must Err — NOT the
    // Ok(None) silent skip that background_sync (best-effort headless) uses.
    let app = locked_mock_app().await;
    let err = crate::write::sync_repo(app.state::<AppState>(), app.handle().clone())
        .await
        .expect_err("sync_repo must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn push_repo_refused_while_app_locked() {
    let app = locked_mock_app().await;
    let err = crate::write::push_repo(app.state::<AppState>())
        .await
        .expect_err("push_repo must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn resolve_sync_divergence_refused_while_app_locked() {
    let app = locked_mock_app().await;
    let err = crate::write::resolve_sync_divergence(
        app.state::<AppState>(),
        app.handle().clone(),
        "deadbeef".into(),
        DivergenceChoice::AdoptRemote,
    )
    .await
    .expect_err("resolve_sync_divergence must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn resolve_entry_conflict_refused_while_app_locked() {
    let app = locked_mock_app().await;
    let err = crate::write::resolve_entry_conflict(
        app.state::<AppState>(),
        app.handle().clone(),
        "test/locked".into(),
        None,
        "deadbeef".into(),
        ExpectedKind::Edit,
        EntryConflictChoice::KeepMine,
    )
    .await
    .expect_err("resolve_entry_conflict must Err while app-locked");
    assert_eq!(err.code, "APP_LOCKED");
}

#[tokio::test]
async fn discard_divergence_allowed_while_app_locked() {
    // Negative test: discard_divergence only runs a local identity wipe
    // (maybe_soft_wipe) — no remote contact — so it must NOT be gated.
    let app = locked_mock_app().await;
    crate::write::discard_divergence(app.state::<AppState>(), app.handle().clone())
        .await
        .expect("discard_divergence must Ok(()) while app-locked (no remote contact)");
}

#[tokio::test]
async fn gate_transparent_when_unlocked() {
    // With app_locked false the gate must not fire: create_secret proceeds past
    // it (it may then succeed or fail for fixture reasons, but never APP_LOCKED).
    let (state, _store) = make_unlocked_state(&[]).await;
    let app = mock_app(state);
    let result = crate::write::create_secret(
        app.state::<AppState>(),
        app.handle().clone(),
        "test/unlocked".into(),
        "body".into(),
    )
    .await;
    match result {
        Ok(_) => {}
        Err(e) => assert_ne!(e.code, "APP_LOCKED", "gate must not fire when unlocked"),
    }
}
