// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret deletion (`Store::delete`) — the simplified delete path that mirrors
//! `Store::set`'s happy path but defers all conflict handling to the sync flow.
//!
//! Delete does a best-effort sync, gates on existence, then remove→commit→push.
//! On a push REJECTION (remote diverged) it rolls back to the pre-delete state
//! and returns `PushRejected`; any OTHER push failure propagates with the local
//! delete commit left in place (an offline delete syncs later, like `set`).
//! See `.plans/0021-delete-secrets.md`.

mod common;

use std::path::Path;

use common::*;
use rustpass::store::{Store, WriteOutcome};

/// Make a local commit in `repo_path` WITHOUT pushing it — the "unpushed local
/// write" that creates divergence with the remote. (Mirrors `write_conflict.rs`.)
fn local_unpushed_commit(repo_path: &Path, rel_path: &str, content: &[u8], message: &str) {
    let repo = git2::Repository::open(repo_path).expect("open store repo");
    let file_path = repo_path.join(rel_path);
    if let Some(p) = file_path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(&file_path, content).unwrap();
    let mut index = repo.index().expect("index");
    index.add_path(Path::new(rel_path)).expect("add_path");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write_tree");
    let tree = repo.find_tree(tree_id).expect("find_tree");
    let head = repo.head().expect("head").target().expect("oid");
    let parent = repo.find_commit(head).expect("parent");
    let sig = git2::Signature::now("local", "local@local").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .expect("commit");
}

/// Whether `rel_path` exists in the bare repo's HEAD tree.
fn exists_in_bare(bare_path: &Path, rel_path: &str) -> bool {
    let Ok(repo) = git2::Repository::open(bare_path) else {
        return false;
    };
    let head = repo.head().expect("head");
    let commit = repo
        .find_commit(head.target().expect("oid"))
        .expect("commit");
    let tree = commit.tree().expect("tree");
    tree.get_path(Path::new(rel_path)).is_ok()
}

/// Configure a store against a fresh bare repo carrying a recipients file with
/// the store's own key. (Mirrors `write_conflict.rs`.)
async fn store_with_recipients() -> (tempfile::TempDir, tempfile::TempDir, Store, String, String) {
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

/// Unwrap a written outcome to its commit hash, panicking on Conflict.
fn written(outcome: WriteOutcome) -> String {
    match outcome {
        WriteOutcome::Written(w) => w.commit,
        other => panic!("expected Written, got {other:?}"),
    }
}

/// The current HEAD oid of the working repo.
fn head_oid(repo_path: &str) -> git2::Oid {
    let repo = git2::Repository::open(repo_path).expect("open repo");
    repo.head().expect("head").target().expect("oid")
}

/// Delete removes the entry, commits, and pushes to the remote.
#[tokio::test]
async fn delete_removes_entry_and_pushes() {
    let (bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    // Create the entry we'll delete (exercises the real create path).
    written(store.set("sites/foo", b"topsecret").await.expect("set ok"));

    let result = store.delete("sites/foo").await.expect("delete ok");
    assert!(!result.commit.is_empty(), "delete returns a commit hash");

    // Gone from the worktree and from the remote's HEAD.
    assert!(
        !Path::new(&repo_path).join("sites/foo.age").exists(),
        "entry removed locally"
    );
    assert!(
        !exists_in_bare(bare_dir.path(), "sites/foo.age"),
        "entry removed from remote HEAD"
    );
}

/// Deleting a missing entry fails the existence gate (after the best-effort sync).
#[tokio::test]
async fn delete_missing_entry_returns_entry_not_found() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;

    let err = store
        .delete("nope/missing")
        .await
        .expect_err("missing entry should fail");
    assert_eq!(err.code, "ENTRY_NOT_FOUND");
}

/// A local-only store (no `origin`) deletes on the happy path: the push is a
/// no-op, so the removal commits locally and returns `Ok`. This is the delete
/// analog of the write path's no-origin push invariant (git.rs).
#[tokio::test]
async fn delete_local_only_store_succeeds_without_origin() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    written(store.set("foo", b"v1").await.expect("set ok"));

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

/// A push rejection (remote diverged) rolls the local back to the pre-delete
/// state and returns `PushRejected` — delete defers conflict resolution to sync.
#[tokio::test]
async fn delete_on_push_rejection_rolls_back() {
    let (bare_dir, _config_dir, store, _identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    // Create + push the entry we'll delete.
    written(store.set("foo", b"v1").await.expect("set ok"));

    // Diverge: an unpushed local commit AND a remote advance on another file.
    // Either makes the subsequent delete push non-fast-forwardable.
    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("other.age", b"other-secret")],
        &recipient,
        "remote moves on another file",
    );

    // The HEAD right before delete == the pre-delete head (sync doesn't move it
    // on divergence).
    let head_before = head_oid(&repo_path);

    let err = store
        .delete("foo")
        .await
        .expect_err("diverged push should be rejected");
    assert_eq!(err.code, "PUSH_REJECTED");

    // Rolled back: the entry is restored locally and HEAD is unchanged.
    assert!(
        Path::new(&repo_path).join("foo.age").exists(),
        "entry restored after rollback"
    );
    assert_eq!(
        head_oid(&repo_path),
        head_before,
        "HEAD rolled back to pre-delete state"
    );
}

/// A push that fails with a NON-rejection error (here: a read-only remote) is
/// propagated, and the LOCAL delete commit is RETAINED (not rolled back) — an
/// offline delete syncs later, mirroring how `set` handles an offline write.
/// This locks the D5 behavior from the eng review.
#[tokio::test]
async fn delete_keeps_local_commit_on_push_failure() {
    let (bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    written(store.set("foo", b"v1").await.expect("set ok"));
    let head_before = head_oid(&repo_path);

    // Make the remote read-only: fetches (sync) still work, but pushes fail with
    // a non-rejection error (permission denied at the object-write level).
    let _writable = ReadOnlyGuard::make(bare_dir.path());

    let err = store
        .delete("foo")
        .await
        .expect_err("push to read-only remote should fail");
    assert_ne!(
        err.code, "PUSH_REJECTED",
        "a read-only remote is a push failure, not a fast-forward rejection"
    );

    // The local delete commit is retained: the file is gone and HEAD advanced.
    assert!(
        !Path::new(&repo_path).join("foo.age").exists(),
        "file removed locally"
    );
    let head_after = head_oid(&repo_path);
    assert_ne!(
        head_after, head_before,
        "local delete commit kept (HEAD advanced)"
    );
    let repo = git2::Repository::open(&repo_path).expect("open repo");
    let commit = repo.find_commit(head_after).expect("head commit");
    assert!(
        commit
            .message()
            .unwrap_or("")
            .contains("Delete secret: foo"),
        "retained commit is the delete"
    );
}

/// RAII: makes a directory tree read-only on construction (so git pushes fail
/// while fetches still work) and restores write bits on drop so the owning
/// `TempDir` can clean up — even if the test panics.
struct ReadOnlyGuard<'a> {
    path: &'a Path,
}
impl<'a> ReadOnlyGuard<'a> {
    fn make(path: &'a Path) -> Self {
        // `chmod -R a-w` removes every write bit; reads/traversal still work.
        let status = std::process::Command::new("chmod")
            .args(["-R", "a-w"])
            .arg(path)
            .status()
            .expect("chmod -R a-w");
        assert!(status.success(), "chmod -R a-w succeeded");
        Self { path }
    }
}
impl Drop for ReadOnlyGuard<'_> {
    fn drop(&mut self) {
        let _ = std::process::Command::new("chmod")
            .args(["-R", "u+w"])
            .arg(self.path)
            .status();
    }
}
