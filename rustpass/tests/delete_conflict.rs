// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret deletion (`Store::delete`) — **local-only**: remove `<name>.age` and
//! commit the removal on the current branch. No sync, no push, no rollback. The
//! autosync orchestrator (`Store::autosync_write`) wraps this in pull → delete →
//! push and routes a rejected push to the sync-time divergence surface (covered
//! in `sync_resolve.rs`). See `.plans/0021-delete-secrets.md` and
//! `.plans/0028-decoupled-writes-autosync.md`.

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

/// The current HEAD oid of the working repo.
fn head_oid(repo_path: &str) -> git2::Oid {
    let repo = git2::Repository::open(repo_path).expect("open repo");
    repo.head().expect("head").target().expect("oid")
}

/// Delete removes the entry and commits locally. The remote is NOT pushed from
/// `delete` itself — publishing is the autosync orchestrator's job — so this
/// pins the local-only behavior (file gone, HEAD advanced by exactly one commit).
#[tokio::test]
async fn delete_removes_entry_and_commits_locally() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    // Create the entry we'll delete (local-only; not pushed).
    store.set("sites/foo", b"topsecret").await.expect("set ok");
    let head_before = head_oid(&repo_path);

    let result = store.delete("sites/foo").await.expect("delete ok");
    assert!(!result.commit.is_empty(), "delete returns a commit hash");

    // Gone from the worktree, HEAD advanced by the delete commit.
    assert!(
        !Path::new(&repo_path).join("sites/foo.age").exists(),
        "entry removed locally"
    );
    assert_ne!(
        head_oid(&repo_path),
        head_before,
        "delete commit landed on HEAD"
    );
}

/// Deleting a missing entry fails the existence gate.
#[tokio::test]
async fn delete_missing_entry_returns_entry_not_found() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;

    let err = store
        .delete("nope/missing")
        .await
        .expect_err("missing entry should fail");
    assert_eq!(err.code, "ENTRY_NOT_FOUND");
}

/// A local-only store (no `origin`) deletes cleanly. (Delete no longer pushes,
/// so this is now the default behavior; the test still pins the no-origin path.)
#[tokio::test]
async fn delete_local_only_store_succeeds_without_origin() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    store.set("foo", b"v1").await.expect("set ok");

    // Drop the origin remote → local-only store.
    let repo = git2::Repository::open(&repo_path).expect("open repo");
    repo.remote_delete("origin").expect("remove origin");

    let result = store.delete("foo").await.expect("delete ok");
    assert!(!result.commit.is_empty(), "local delete commit was created");

    // Entry is gone locally and the delete commit is on the local HEAD.
    assert!(
        !Path::new(&repo_path).join("foo.age").exists(),
        "entry removed locally"
    );
    let head = repo.head().expect("head");
    let commit = repo
        .find_commit(head.target().expect("oid"))
        .expect("head commit");
    assert!(
        commit
            .message()
            .unwrap_or("")
            .contains("Delete secret: foo"),
        "HEAD is the delete commit"
    );
}
