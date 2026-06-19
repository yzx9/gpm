// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Git-conflict handling on the write path (`Store::set` → `WriteOutcome::Conflict`
//! and `Store::resolve_write_conflict`).
//!
//! A real conflict needs true divergence: the local branch has a commit the
//! remote does not (an unpushed write), and the remote has a commit the local
//! does not (a same-name file added upstream). `set`'s fast-forward-only
//! pre-sync refuses to clobber the divergent local, so the subsequent push is
//! rejected and the conflict is surfaced — deterministically, no racing.

mod common;

use std::path::Path;

use common::*;
use rustpass::crypto;
use rustpass::store::{ConflictChoice, Store, WriteOutcome};

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
/// listing the store's own key. Returns `(bare_dir, config_dir, store,
/// identity, recipient)` so tests can craft remote commits decryptable (or
/// not) by the configured identity.
async fn store_with_recipients() -> (tempfile::TempDir, tempfile::TempDir, Store, String, String) {
    let (identity, recipient) = generate_test_keypair();

    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![],
        vec![(".gopass-recipients", recipient.as_bytes())],
        &recipient,
    );

    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf());
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

/// `set` surfaces a Conflict (decryptable) when the remote added a same-name
/// file encrypted to us while we held an unpushed local commit.
#[tokio::test]
async fn set_returns_conflict_when_remote_same_name_is_decryptable() {
    let (bare_dir, _config_dir, store, identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    // 1. Unpushed local write → local diverges from the remote base.
    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );

    // 2. Remote adds a same-name file (encrypted to us → decryptable).
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs-secret")],
        &recipient,
        "remote adds same-name",
    );

    // 3. set: pre-sync refuses to clobber the divergent local; push rejected.
    let outcome = store.set("conflict", b"ours-secret").await.expect("set ok");
    match &outcome {
        WriteOutcome::Conflict(c) => {
            assert_eq!(c.name, "conflict");
            assert!(
                c.remote_decryptable,
                "remote version was encrypted to us, should be decryptable"
            );
        }
        other => panic!("expected Conflict, got {other:?}"),
    }

    // The conflict object must not leak either plaintext.
    let serialized = serde_json::to_string(&outcome).unwrap();
    assert!(!serialized.contains("ours-secret"));
    assert!(!serialized.contains("theirs-secret"));
    // The identity confirms the remote blob really was decryptable.
    let blob = read_from_bare(bare_dir.path(), "conflict.age");
    assert_eq!(
        crypto::decrypt_bytes(&blob, identity.as_bytes(), None).unwrap(),
        b"theirs-secret"
    );
}

/// `set` surfaces a Conflict with `remote_decryptable: false` when the
/// remote's same-name file was encrypted to a key we don't hold.
#[tokio::test]
async fn set_returns_conflict_when_remote_same_name_is_undecryptable() {
    let (_other_identity, other_recipient) = generate_test_keypair();
    let (bare_dir, _config_dir, store, identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );

    // Remote adds a same-name file encrypted to OTHER (not us) → undecryptable.
    add_commit_to_bare(
        bare_dir.path(),
        vec![("conflict.age", b"theirs-secret")],
        &other_recipient,
        "remote adds same-name (other key)",
    );

    let outcome = store.set("conflict", b"ours-secret").await.expect("set ok");
    match outcome {
        WriteOutcome::Conflict(c) => {
            assert_eq!(c.name, "conflict");
            assert!(
                !c.remote_decryptable,
                "remote was encrypted to another key; we can't decrypt it"
            );
        }
        other => panic!("expected Conflict, got {other:?}"),
    }

    // Sanity: we indeed cannot decrypt the remote blob.
    let blob = read_from_bare(bare_dir.path(), "conflict.age");
    assert!(crypto::decrypt_bytes(&blob, identity.as_bytes(), None).is_err());
}

/// KeepMine on a decryptable conflict overwrites the remote with our version.
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

    // Drive into the conflict, then resolve with KeepMine.
    assert!(matches!(
        store.set("conflict", b"ours").await,
        Ok(WriteOutcome::Conflict(_))
    ));

    let result = store
        .resolve_write_conflict("conflict", b"ours", ConflictChoice::KeepMine)
        .await
        .expect("resolve");
    assert!(result.is_some(), "KeepMine should push our version");

    // The remote now holds OUR version (readable through the store).
    assert_eq!(store.get("conflict").await.expect("get").password(), "ours");
}

/// KeepMine is REFUSED on an undecryptable remote (would destroy data we
/// can't read) — surfaces `UnsafeOverwrite`.
#[tokio::test]
async fn resolve_keep_mine_refused_on_undecryptable() {
    let (_other_identity, other_recipient) = generate_test_keypair();
    let (_bare_dir, _config_dir, store, _identity, _recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    add_commit_to_bare(
        _bare_dir.path(),
        vec![("conflict.age", b"theirs")],
        &other_recipient,
        "remote same-name (other key)",
    );

    assert!(matches!(
        store.set("conflict", b"ours").await,
        Ok(WriteOutcome::Conflict(_))
    ));

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

    assert!(matches!(
        store.set("conflict", b"ours").await,
        Ok(WriteOutcome::Conflict(_))
    ));

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

    assert!(matches!(
        store.set("conflict", b"ours").await,
        Ok(WriteOutcome::Conflict(_))
    ));

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

/// Cancel leaves the store at the pre-write state; our write is not pushed.
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

    assert!(matches!(
        store.set("conflict", b"ours").await,
        Ok(WriteOutcome::Conflict(_))
    ));

    let result = store
        .resolve_write_conflict("conflict", b"ours", ConflictChoice::Cancel)
        .await
        .expect("resolve");
    assert!(result.is_none());

    // Cancel rolled back to the pre-write state, so the local repo no longer
    // has the entry.
    let err = store.get("conflict").await.unwrap_err();
    assert_eq!(err.code, "ENTRY_NOT_FOUND");

    // And our "ours" was never pushed: the remote still holds "theirs".
    let pushed = read_from_bare(bare_dir.path(), "conflict.age");
    assert_eq!(
        crypto::decrypt_bytes(&pushed, identity.as_bytes(), None).unwrap(),
        b"theirs"
    );
}

/// When the remote advanced on OTHER files (no same-name collision), `set`
/// transparently replays the write on the remote tip → Written.
#[tokio::test]
async fn set_auto_replays_when_remote_changed_other_files() {
    let (bare_dir, _config_dir, store, _identity, recipient) = store_with_recipients().await;
    let repo_path = store.config().await.expect("config").local_path;

    local_unpushed_commit(
        Path::new(&repo_path),
        "local-only.txt",
        b"unpushed",
        "local-only commit",
    );
    // Remote adds an UNRELATED file (no same-name collision with "mine").
    add_commit_to_bare(
        bare_dir.path(),
        vec![("other.age", b"remote-other")],
        &recipient,
        "remote adds other file",
    );

    let outcome = store.set("mine", b"mine-secret").await.expect("set ok");
    match outcome {
        WriteOutcome::Written(r) => assert!(!r.commit.is_empty()),
        other => panic!("expected Written (auto-replay), got {other:?}"),
    }

    // Both the unrelated remote file and our new file are present & readable.
    assert_eq!(
        store.get("mine").await.expect("get").password(),
        "mine-secret"
    );
    assert_eq!(
        store.get("other").await.expect("get").password(),
        "remote-other"
    );
}
