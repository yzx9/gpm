// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret edit (`Store::update`) — **local-only**: overwrite an existing entry's
//! raw body via `Store::set` (no template re-applied) and commit locally. No
//! sync, no push. The autosync orchestrator (`Store::autosync_write`) wraps this
//! in pull → update → push.
//!
//! The `0026` base-version-aware clobber caveat (a stale edit silently
//! overwriting a teammate's newer same-name change) now applies at the
//! orchestrator's pre-write pull, not in bare `update` — it is pinned in
//! `sync_resolve.rs` against `autosync_write`. See
//! `.plans/0026-edit-base-version-aware.md`.

mod common;

use std::path::Path;

use common::*;
use rustpass::store::Store;

/// Configure a store against a fresh bare repo carrying a recipients file with
/// the store's own key. (Mirrors `write_conflict.rs`.)
async fn store_with_recipients() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Store,
    String,
    String,
) {
    let (identity, recipient) = generate_test_keypair();

    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![],
        vec![(".gopass-recipients", recipient.as_bytes())],
        &recipient,
    );

    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure");
    (bare_dir, config_dir, store, identity, recipient)
}

/// Edit overwrites an existing entry in place and commits locally — and does NOT
/// re-apply a `.pass-template` (templates shape new secrets, not mutations).
#[tokio::test]
async fn update_overwrites_existing_locally() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    // A template that WOULD apply on create to anything under `sites/`.
    std::fs::create_dir_all(Path::new(&repo_path).join("sites")).unwrap();
    std::fs::write(
        Path::new(&repo_path).join("sites/.pass-template"),
        "PREFIX:{{ .Content }}",
    )
    .unwrap();

    // create applies the template → the stored body is templated (local-only).
    store
        .create("sites/foo", b"secret")
        .await
        .expect("create ok");
    assert_eq!(
        store.get("sites/foo").await.expect("get").password(),
        "PREFIX:secret",
        "create applied the template"
    );

    // update stores the raw body verbatim — no template re-applied.
    let result = store
        .update("sites/foo", b"newsecret")
        .await
        .expect("update ok");
    assert!(
        !result.commit.is_empty(),
        "edit returns a commit hash"
    );
    assert_eq!(
        store.get("sites/foo").await.expect("get").password(),
        "newsecret",
        "edit stored the raw body, no template re-applied"
    );
}

/// Editing a missing entry fails the existence gate (before any write).
#[tokio::test]
async fn update_missing_entry_returns_entry_not_found() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;

    let err = store
        .update("nope/missing", b"anything")
        .await
        .expect_err("missing entry should fail the gate");
    assert_eq!(err.code, "ENTRY_NOT_FOUND");
}

/// update's existence gate is preceded by the same name-validation +
/// path-traversal guards `set` has: bad names fail fast with `InvalidEntryName`,
/// before any existence check or write.
#[tokio::test]
async fn update_rejects_invalid_and_traversal_names() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    for bad in ["", "/", "//", "a/..", "\\x", "../escape"] {
        let err = store
            .update(bad, b"x")
            .await
            .expect_err("bad name rejected");
        assert_eq!(err.code, "INVALID_ENTRY_NAME", "name {bad:?}");
    }
}

/// A local-only store (no `origin`) edits cleanly — and on any store, since
/// `update` no longer pushes (the autosync orchestrator does).
#[tokio::test]
async fn update_local_only_store_succeeds_without_origin() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    store.set("foo", b"v1").await.expect("set ok");

    // Drop the origin remote → local-only store.
    let repo = git2::Repository::open(&repo_path).expect("open repo");
    repo.remote_delete("origin").expect("remove origin");

    let result = store.update("foo", b"v2").await.expect("update ok");
    assert!(
        !result.commit.is_empty(),
        "local edit returns a commit hash"
    );

    assert_eq!(
        store.get("foo").await.expect("get").password(),
        "v2",
        "edit applied locally"
    );
    let head = repo.head().expect("head");
    let commit = repo
        .find_commit(head.target().expect("oid"))
        .expect("head commit");
    assert!(
        commit.message().unwrap_or("").contains("Save secret: foo"),
        "HEAD is the save commit"
    );
}
