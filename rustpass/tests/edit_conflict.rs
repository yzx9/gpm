// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret edit (`Store::update`) — the existence-gated write sibling of
//! `Store::create`. Edit overwrites an existing entry's raw body via `Store::set`
//! (no template), and inherits `set`'s conflict machinery on divergence.
//!
//! Eng-review accepted limitation (D3, pinned by
//! `update_fast_forward_clobber_is_documented_behavior`): because `set`'s
//! conflict detection fires only on push rejection (which needs local
//! divergence), an edit built on a stale read with no local divergence
//! fast-forwards over a teammate's newer same-name change and returns `Written`
//! — silently clobbering it. A base-version-aware fix is deferred.

mod common;

use std::path::Path;

use common::*;
use rustpass::crypto;
use rustpass::store::{ConflictChoice, Store, WriteOutcome};

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

/// Read a file from a bare repo's HEAD tree.
fn read_from_bare(bare_path: &Path, rel_path: &str) -> Vec<u8> {
    let repo = git2::Repository::open(bare_path).expect("open bare");
    let head = repo.head().expect("head");
    let commit = repo
        .find_commit(head.target().expect("oid"))
        .expect("commit");
    let tree = commit.tree().expect("tree");
    let entry = tree
        .get_path(Path::new(rel_path))
        .unwrap_or_else(|_| panic!("{rel_path} in bare HEAD"));
    repo.find_blob(entry.id()).expect("blob").content().to_vec()
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

/// RAII: makes a directory tree read-only on construction (so git pushes fail
/// while fetches still work) and restores write bits on drop so the owning
/// `TempDir` can clean up. (Mirrors `delete_conflict.rs`.)
struct ReadOnlyGuard<'a> {
    path: &'a Path,
}
impl<'a> ReadOnlyGuard<'a> {
    fn make(path: &'a Path) -> Self {
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

/// Edit overwrites an existing entry in place and pushes — and does NOT re-apply
/// a `.pass-template` (templates shape new secrets, not mutations).
#[tokio::test]
async fn update_overwrites_existing_and_pushes() {
    let (bare_dir, _config_dir, store, identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    // A template that WOULD apply on create to anything under `sites/`.
    std::fs::create_dir_all(Path::new(&repo_path).join("sites")).unwrap();
    std::fs::write(
        Path::new(&repo_path).join("sites/.pass-template"),
        "PREFIX:{{ .Content }}",
    )
    .unwrap();

    // create applies the template → the stored body is templated.
    written(
        store
            .create("sites/foo", b"secret")
            .await
            .expect("create ok"),
    );
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
        matches!(result, WriteOutcome::Written(ref w) if !w.commit.is_empty()),
        "edit returns Written"
    );
    assert_eq!(
        store.get("sites/foo").await.expect("get").password(),
        "newsecret",
        "edit stored the raw body, no template re-applied"
    );

    // The edited body also landed on the remote HEAD.
    let blob = read_from_bare(bare_dir.path(), "sites/foo.age");
    assert_eq!(
        crypto::decrypt_bytes(&blob, identity.as_bytes(), None).unwrap(),
        b"newsecret",
        "remote HEAD has the edited body"
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

/// On divergence (unpushed local + remote same-name advance) edit surfaces a
/// `Conflict` for the caller to resolve — it does not silently clobber.
#[tokio::test]
async fn update_on_same_name_conflict_surfaces_conflict() {
    let (bare_dir, _config_dir, store, _identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    // Create + push the entry (so the existence gate passes).
    written(store.set("conflict", b"ours-v1").await.expect("set ok"));

    // Diverge: an unpushed local commit AND a remote same-name advance.
    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs-new")],
        &recipient,
        "remote same-name advance",
    );

    let outcome = store
        .update("conflict", b"ours-v2")
        .await
        .expect("update ok");
    match outcome {
        WriteOutcome::Conflict(c) => {
            assert_eq!(c.name, "conflict");
            assert!(
                c.remote_decryptable,
                "remote was encrypted to us, should be decryptable"
            );
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
}

/// Resolving an edit conflict with KeepMine pushes our edited version.
#[tokio::test]
async fn update_resolve_keep_mine_pushes() {
    let (bare_dir, _config_dir, store, _identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    written(store.set("conflict", b"ours-v1").await.expect("set ok"));
    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs-new")],
        &recipient,
        "remote same-name advance",
    );

    assert!(matches!(
        store.update("conflict", b"ours-v2").await,
        Ok(WriteOutcome::Conflict(_))
    ));

    let result = store
        .resolve_write_conflict("conflict", b"ours-v2", ConflictChoice::KeepMine)
        .await
        .expect("resolve");
    assert!(result.is_some(), "KeepMine pushes our version");
    assert_eq!(
        store.get("conflict").await.expect("get").password(),
        "ours-v2"
    );
}

/// KeepRemote adopts the remote version (pushes nothing); Cancel keeps the
/// pre-edit version (pushes nothing). Both return `None`.
#[tokio::test]
async fn update_resolve_cancel_and_keep_remote() {
    // KeepRemote: adopt the remote's version, push nothing.
    {
        let (bare_dir, _cfg, store, _identity, recipient) = store_with_recipients().await;
        let repo_path = store.config().await.expect("config").local_path;
        written(store.set("c", b"v1").await.expect("set ok"));
        local_unpushed_commit(Path::new(&repo_path), "local.txt", b"x", "local-only");
        add_commit_to_bare(
            bare_dir.path(),
            vec![("c.age", b"theirs")],
            &recipient,
            "remote same-name",
        );
        assert!(matches!(
            store.update("c", b"v2").await,
            Ok(WriteOutcome::Conflict(_))
        ));

        let r = store
            .resolve_write_conflict("c", b"v2", ConflictChoice::KeepRemote)
            .await
            .expect("resolve");
        assert!(r.is_none(), "KeepRemote pushes nothing");
        assert_eq!(
            store.get("c").await.expect("get").password(),
            "theirs",
            "adopted the remote version"
        );
    }

    // Cancel: leave the store at the pre-edit (rolled-back) state, push nothing.
    {
        let (bare_dir, _cfg, store, identity, recipient) = store_with_recipients().await;
        let repo_path = store.config().await.expect("config").local_path;
        written(store.set("c", b"v1").await.expect("set ok"));
        local_unpushed_commit(Path::new(&repo_path), "local.txt", b"x", "local-only");
        add_commit_to_bare(
            bare_dir.path(),
            vec![("c.age", b"theirs")],
            &recipient,
            "remote same-name",
        );
        assert!(matches!(
            store.update("c", b"v2").await,
            Ok(WriteOutcome::Conflict(_))
        ));

        let r = store
            .resolve_write_conflict("c", b"v2", ConflictChoice::Cancel)
            .await
            .expect("resolve");
        assert!(r.is_none(), "Cancel pushes nothing");
        assert_eq!(
            store.get("c").await.expect("get").password(),
            "v1",
            "cancel left the pre-edit version"
        );
        // Our edit was never pushed: the remote still holds the teammate's version.
        let blob = read_from_bare(bare_dir.path(), "c.age");
        assert_eq!(
            crypto::decrypt_bytes(&blob, identity.as_bytes(), None).unwrap(),
            b"theirs",
            "cancel did not push our edit"
        );
    }
}

/// ⚠️ ACCEPTED LIMITATION (eng review D3): when the remote advanced with a newer
/// same-name version but the local had NO unpushed commit, edit fast-forwards
/// over it and returns `Written` — silently clobbering the teammate's change.
/// This test PINS that behavior so it can't regress silently and so the deferred
/// base-version-aware fix has a failing-to-pass target.
#[tokio::test]
async fn update_fast_forward_clobber_is_documented_behavior() {
    let (bare_dir, _config_dir, store, identity, recipient) = store_with_recipients().await;

    // Create + push v1 (local and remote in sync).
    written(store.set("clobber", b"v1").await.expect("set ok"));

    // A teammate advances the SAME entry on the remote. The local has no
    // unpushed commit, so there is NO divergence — only a behind-local.
    add_commit_to_bare(
        bare_dir.path(),
        vec![("clobber.age", b"newer-from-teammate")],
        &recipient,
        "remote advances same-name",
    );

    // The user, editing from the stale v1 snapshot, saves an edit.
    let outcome = store
        .update("clobber", b"stale-edit")
        .await
        .expect("update ok");
    assert!(
        matches!(outcome, WriteOutcome::Written(_)),
        "no divergence → fast-forward → Written (NO Conflict surfaced)"
    );

    // The teammate's newer version was silently overwritten by the stale edit.
    assert_eq!(
        store.get("clobber").await.expect("get").password(),
        "stale-edit",
        "local now reflects the stale edit"
    );
    let blob = read_from_bare(bare_dir.path(), "clobber.age");
    assert_eq!(
        crypto::decrypt_bytes(&blob, identity.as_bytes(), None).unwrap(),
        b"stale-edit",
        "remote HEAD has the stale edit, not the teammate's newer version — the clobber"
    );
}

/// A push that fails with a NON-rejection error (read-only remote) is
/// propagated, and the LOCAL edit commit is RETAINED (syncs later) — mirroring
/// how `set` handles an offline write.
#[tokio::test]
async fn update_keeps_local_commit_on_push_failure() {
    let (bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    written(store.set("foo", b"v1").await.expect("set ok"));
    let head_before = head_oid(&repo_path);

    // Read-only remote: fetches (sync) still work, pushes fail with a non-rejection error.
    let _writable = ReadOnlyGuard::make(bare_dir.path());

    let err = store
        .update("foo", b"v2")
        .await
        .expect_err("push to read-only remote should fail");
    assert_ne!(
        err.code, "PUSH_REJECTED",
        "a read-only remote is a push failure, not a fast-forward rejection"
    );

    // The local edit commit is retained: the body advanced and HEAD moved.
    assert_eq!(
        store.get("foo").await.expect("get").password(),
        "v2",
        "edit applied locally"
    );
    let head_after = head_oid(&repo_path);
    assert_ne!(
        head_after, head_before,
        "local edit commit kept (HEAD advanced)"
    );
    let repo = git2::Repository::open(&repo_path).expect("open repo");
    let commit = repo.find_commit(head_after).expect("head commit");
    assert!(
        commit.message().unwrap_or("").contains("Save secret: foo"),
        "retained commit is the save"
    );
}

/// A local-only store (no `origin`) edits on the happy path: the push is a
/// no-op, so the edit commits locally and returns `Written`.
#[tokio::test]
async fn update_local_only_store_succeeds_without_origin() {
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    written(store.set("foo", b"v1").await.expect("set ok"));

    // Drop the origin remote → local-only store.
    let repo = git2::Repository::open(&repo_path).expect("open repo");
    repo.remote_delete("origin").expect("remove origin");

    let result = store.update("foo", b"v2").await.expect("update ok");
    assert!(
        matches!(result, WriteOutcome::Written(ref w) if !w.commit.is_empty()),
        "local edit returns Written"
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

/// Push a commit to the bare remote that REMOVES `rel_path` (simulates a teammate
/// deleting the entry on another device). Clone-modify-pushback, mirroring
/// `add_commit_to_bare`.
fn delete_from_bare(bare_path: &Path, rel_path: &str) {
    let work_dir = tempfile::tempdir().expect("work dir");
    let repo = git2::Repository::clone(bare_path.to_str().expect("utf-8"), work_dir.path())
        .expect("clone bare");
    let sig = git2::Signature::now("remote", "remote@remote").expect("sig");
    std::fs::remove_file(work_dir.path().join(rel_path)).ok();
    let mut index = repo.index().expect("index");
    index
        .remove_path(Path::new(rel_path))
        .expect("remove_path (stage deletion)");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write_tree");
    let tree = repo.find_tree(tree_id).expect("find_tree");
    let head = repo.head().expect("head").target().expect("oid");
    let parent = repo.find_commit(head).expect("parent");
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "remote deletes entry",
        &tree,
        &[&parent],
    )
    .expect("commit");
    let branch = repo.head().expect("head").shorthand().unwrap().to_string();
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut remote = repo.find_remote("origin").expect("origin");
    remote.push(&[&refspec], None).expect("push back");
}

/// ⚠️ ACCEPTED LIMITATION (eng review D3, same root cause as the fast-forward
/// clobber): if a teammate deletes the entry on the remote and the local (without
/// syncing) edits it, the edit silently RESURRECTS it — the existence gate passes
/// locally, then `set`'s sync fast-forwards to the deletion and write_commit_push
/// re-creates the file. The base-version-aware fix covers both this
/// and the clobber. This test PINS the resurrection so it can't drift silently.
#[tokio::test]
async fn update_resurrects_teammate_deleted_entry_is_documented_behavior() {
    let (bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;

    // Create + push the entry (local and remote in sync).
    written(store.set("foo", b"v1").await.expect("set ok"));

    // A teammate deletes it on the remote; the local has no unpushed commit.
    delete_from_bare(bare_dir.path(), "foo.age");

    // The user edits the (stale, still-present-locally) entry.
    let outcome = store.update("foo", b"edited").await.expect("update ok");
    assert!(
        matches!(outcome, WriteOutcome::Written(_)),
        "no divergence → fast-forward → Written (resurrected, no conflict)"
    );

    // The deleted entry came back with the edit.
    assert_eq!(
        store.get("foo").await.expect("get").password(),
        "edited",
        "edit resurrected the teammate-deleted entry"
    );
}
