// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! `Store::resolve_write_conflict` — the legacy write-time conflict resolver,
//! retained (alive via the frozen `resolve_write_conflict` command) until the
//! frontend flip in `PR2c`. Conflict surfacing moved to sync time
//! (`resolve_sync_divergence` / `resolve_keep_mine`, covered in `sync_resolve.rs`);
//! `set` itself is local-only and no longer surfaces a `Conflict`, so these
//! tests drive the resolver directly against a manual divergence (an unpushed
//! local commit + a remote same-name advance).
//!
//! `set`-surfaces-Conflict and `set`-auto-replays behavior is gone with the
//! local-only write path and is intentionally not ported.

mod common;

use std::path::Path;

use common::*;
use rustpass::crypto;
use rustpass::store::{ConflictChoice, Store};

/// Make a local commit in `repo_path` (the store's working repo) WITHOUT
/// pushing it. This is the "unpushed local write" that creates divergence.
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

/// Configure a store against a fresh bare repo carrying a recipients file
/// listing the store's own key.
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

/// KeepMine on a decryptable remote overwrites it with our version (and pushes).
#[tokio::test]
async fn resolve_keep_mine_overwrites_decryptable_remote() {
    let (bare_dir, _config_dir, store, _identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs")],
        &recipient,
        "remote same-name",
    );

    // Resolve directly — the resolver operates on the remote same-name state.
    let result = store
        .resolve_write_conflict("conflict", b"ours", ConflictChoice::KeepMine)
        .await
        .expect("resolve");
    assert!(result.is_some(), "KeepMine should push our version");

    // The remote now holds OUR version (readable through the store).
    assert_eq!(store.get("conflict").await.expect("get").password(), "ours");
}

/// KeepMine is REFUSED on an undecryptable remote (would destroy data we can't
/// read) — surfaces `UnsafeOverwrite`.
#[tokio::test]
async fn resolve_keep_mine_refused_on_undecryptable() {
    let (_other_identity, other_recipient) = generate_test_keypair();
    let (bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs")],
        &other_recipient,
        "remote same-name (other key)",
    );

    let err = store
        .resolve_write_conflict("conflict", b"ours", ConflictChoice::KeepMine)
        .await
        .unwrap_err();
    assert_eq!(
        err.code, "UNSAFE_OVERWRITE",
        "KeepMine on undecryptable must be refused, got {err}"
    );
}

/// KeepMineForce overwrites even an undecryptable remote (explicit, destructive).
#[tokio::test]
async fn resolve_keep_mine_force_overwrites_undecryptable() {
    let (_other_identity, other_recipient) = generate_test_keypair();
    let (bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs")],
        &other_recipient,
        "remote same-name (other key)",
    );

    let result = store
        .resolve_write_conflict("conflict", b"ours", ConflictChoice::KeepMineForce)
        .await
        .expect("force resolve");
    assert!(result.is_some(), "KeepMineForce should push our version");

    // We can read our own version back.
    assert_eq!(store.get("conflict").await.expect("get").password(), "ours");
}

/// KeepRemote adopts the remote version, discarding our write.
#[tokio::test]
async fn resolve_keep_remote_adopts_remote() {
    let (bare_dir, _config_dir, store, _identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs")],
        &recipient,
        "remote same-name",
    );

    let result = store
        .resolve_write_conflict("conflict", b"ours", ConflictChoice::KeepRemote)
        .await
        .expect("resolve");
    assert!(result.is_none(), "KeepRemote pushes nothing");

    // The remote version is now local.
    assert_eq!(
        store.get("conflict").await.expect("get").password(),
        "theirs"
    );
}

/// Cancel leaves the store at the remote tip (the resolver's pre-write state);
/// our write is not pushed.
#[tokio::test]
async fn resolve_cancel_does_not_push() {
    let (bare_dir, _config_dir, store, identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs")],
        &recipient,
        "remote same-name",
    );

    let result = store
        .resolve_write_conflict("conflict", b"ours", ConflictChoice::Cancel)
        .await
        .expect("resolve");
    assert!(result.is_none());

    // Cancel writes nothing and does not fast-forward, so the entry (which the
    // manual setup only put on the remote) is still absent locally.
    let err = store.get("conflict").await.unwrap_err();
    assert_eq!(
        err.code, "ENTRY_NOT_FOUND",
        "Cancel wrote nothing of ours; conflict.age is not local"
    );

    // And our "ours" was never pushed: the remote still holds "theirs".
    let pushed = read_from_bare(bare_dir.path(), "conflict.age");
    assert_eq!(
        crypto::decrypt_bytes(&pushed, identity.as_bytes(), None).unwrap(),
        b"theirs"
    );
}
