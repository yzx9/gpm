// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Conflict-stash lifecycle — the in-memory `(name, plaintext)` a write collision
//! stashes so a re-resolve doesn't round-trip the secret across IPC again. The
//! security invariant: the stash is consumed on every resolve (success *or*
//! failure) and on lock, so a plaintext never lingers behind a wiped identity.
//!
//! App-free: the cores ([`write::create_and_stash`], [`write::resolve_pending`],
//! [`write::stash_pending`], [`write::clear_pending`]) take `&AppState` directly.

use std::path::Path;

use rustpass::{ConflictChoice, WriteOutcome};

use crate::tests::make_unlocked_state;
use crate::write;

/// Stashing fills the pending slot; clearing empties it.
#[tokio::test]
async fn stash_then_clear_round_trip() {
    let (state, _guard) = make_unlocked_state(&[]).await;

    write::stash_pending(&state.pending_write, "sites/foo", b"hunter2".to_vec());
    assert!(
        state.pending_write.lock().unwrap().is_some(),
        "stash should fill the pending slot"
    );

    write::clear_pending(&state.pending_write);
    assert!(
        state.pending_write.lock().unwrap().is_none(),
        "clear should empty the pending slot"
    );
}

/// Resolving with nothing stashed returns a store error and consumes nothing.
#[tokio::test]
async fn resolve_with_no_pending_errors() {
    let (state, _guard) = make_unlocked_state(&[]).await;

    let err = write::resolve_pending(&state, ConflictChoice::Cancel)
        .await
        .expect_err("resolving with no pending should error");
    assert_eq!(err.code, "STORE_ERROR");
    assert!(
        state.pending_write.lock().unwrap().is_none(),
        "nothing was stashed, so nothing to consume"
    );
}

/// The stash is consumed even when the underlying resolve errors — the
/// "never linger" invariant. (The store here isn't in a real conflict state, so
/// the resolve errors; what matters is the plaintext is gone either way.)
#[tokio::test]
async fn resolve_consumes_pending_even_on_error() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    write::stash_pending(&state.pending_write, "sites/foo", b"hunter2".to_vec());

    let _ = write::resolve_pending(&state, ConflictChoice::Cancel).await;

    assert!(
        state.pending_write.lock().unwrap().is_none(),
        "the stash must be consumed even when resolve errors"
    );
}

/// Commit `rel_path` to the store's working repo WITHOUT pushing — the unpushed
/// local commit that diverges from the remote. (Mirrors the rustpass helpers.)
fn local_unpushed_commit(repo_path: &Path, rel_path: &str, content: &[u8], message: &str) {
    let repo = git2::Repository::open(repo_path).expect("open work repo");
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

/// Push a new commit onto the bare remote rewriting `rel_path` to `plaintext`
/// (encrypted to a fresh keypair the store can't decrypt), advancing the remote
/// so a subsequent write diverges. (Mirrors rustpass's `add_commit_to_bare`.)
fn advance_bare_same_name(bare_path: &Path, rel_path: &str, plaintext: &[u8]) {
    let (_other_identity, other_recipient) = super::generate_test_keypair();
    let ciphertext = super::encrypt_to_recipient(plaintext, &other_recipient);

    let work_dir = tempfile::tempdir().expect("work dir");
    let repo = git2::Repository::clone(bare_path.to_str().expect("utf-8"), work_dir.path())
        .expect("clone bare");
    let sig = git2::Signature::now("remote", "remote@remote").expect("sig");
    let file_path = work_dir.path().join(rel_path);
    if let Some(p) = file_path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(&file_path, ciphertext).unwrap();
    let mut index = repo.index().expect("index");
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add_all");
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write_tree");
    let tree = repo.find_tree(tree_id).expect("find_tree");
    let head = repo.head().expect("head").target().expect("oid");
    let parent = repo.find_commit(head).expect("parent");
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "remote same-name advance",
        &tree,
        &[&parent],
    )
    .expect("commit");
    let branch = repo.head().expect("head").shorthand().unwrap().to_string();
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut remote = repo.find_remote("origin").expect("origin");
    remote.push(&[&refspec], None).expect("push back");
}

/// `update_and_stash` stashes the edited plaintext when `Store::update` surfaces
/// a same-name conflict, so the shared `resolve_write_conflict` can replay it.
/// The generic stash mechanics are covered above; this locks the EDIT path's
/// wiring (eng-review Codex #7) against a real divergence conflict.
#[tokio::test]
async fn update_and_stash_stashes_on_conflict() {
    let (state, guard) = make_unlocked_state(&[]).await;
    let repo_path = state.store.config().await.expect("config").local_path;

    // Create + push the entry we'll edit.
    assert!(matches!(
        state.store.set("c", b"v1").await,
        Ok(WriteOutcome::Written(_))
    ));

    // Diverge: an unpushed local commit AND a remote same-name advance.
    local_unpushed_commit(Path::new(&repo_path), "local.txt", b"x", "local-only");
    advance_bare_same_name(guard.bare_dir.path(), "c.age", b"theirs");

    let outcome = write::update_and_stash(&state, "c", b"ours-v2".to_vec())
        .await
        .expect("update+stash ok");
    assert!(
        matches!(outcome, WriteOutcome::Conflict(ref c) if c.name == "c"),
        "divergence should surface a Conflict, got {outcome:?}"
    );
    assert!(
        state.pending_write.lock().unwrap().is_some(),
        "edit conflict must stash the plaintext for resolve_write_conflict"
    );
}
