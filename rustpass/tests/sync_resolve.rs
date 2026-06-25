// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Sync-time "keep mine" divergence resolution (`Store::resolve_sync_divergence`
//! with [`DivergenceChoice::KeepMine`]) + the on-demand divergence preview
//! (`Store::sync_divergence_preview`) + the local-ahead pull classification.
//!
//! After the sync/write decoupling, a rejected push routes to a divergence
//! modal; "keep mine" re-encrypts the local-only `.age` entries onto the reviewed
//! remote tip (with the CURRENT recipient set) and pushes — it never rebases old
//! ciphertext (which would keep stale recipients) and never merges `.age` blobs.

mod common;

use std::path::Path;

use common::*;
use rustpass::SyncOutcome;
use rustpass::crypto;
use rustpass::store::DivergenceChoice;

/// Write an encrypted `.age` entry into the store's working repo as an unpushed
/// local commit. `plaintext` is encrypted to `recipient` so the store can decrypt
/// it again during "keep mine".
fn local_secret(repo_path: &Path, rel: &str, plaintext: &[u8], recipient: &str, message: &str) {
    let ciphertext = encrypt_to_recipient(plaintext, recipient);
    local_commit_files(repo_path, &[(rel, ciphertext.as_slice())], message);
}

/// Full HEAD oid of the store's working repo.
fn local_head_oid(repo_path: &Path) -> String {
    let repo = git2::Repository::open(repo_path).expect("open store repo");
    repo.head()
        .expect("head")
        .target()
        .expect("oid")
        .to_string()
}

/// "Keep mine" replays local-only secrets onto the reviewed remote tip: both the
/// local secrets and the remote's unrelated file survive, are readable, and the
/// result is pushed (the bare tip advances to the new commit).
#[tokio::test]
async fn keep_mine_replays_local_secrets_onto_remote() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;

    // Local diverges: two unpushed secrets.
    local_secret(
        Path::new(&repo_path),
        "mine1.age",
        b"mine-1",
        &recipient,
        "local adds mine1",
    );
    local_secret(
        Path::new(&repo_path),
        "mine2.age",
        b"mine-2",
        &recipient,
        "local adds mine2",
    );
    // Remote diverges on an unrelated file.
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote-only.age", b"remote-secret")],
        &recipient,
        "remote adds unrelated",
    );

    let tip = bare_head_oid(bare_dir.path());
    let result = store
        .resolve_sync_divergence(&tip, DivergenceChoice::KeepMine)
        .await
        .expect("keep mine");
    assert!(result.changed, "HEAD should advance");
    // The new commit was pushed: the bare tip is now our keep-mine commit.
    assert!(bare_head_oid(bare_dir.path()).starts_with(&result.head));

    // All three entries survived and are readable through the store.
    assert_eq!(
        store.get("mine1").await.expect("get mine1").password(),
        "mine-1"
    );
    assert_eq!(
        store.get("mine2").await.expect("get mine2").password(),
        "mine-2"
    );
    assert_eq!(
        store
            .get("remote-only")
            .await
            .expect("get remote")
            .password(),
        "remote-secret"
    );
}

/// "Keep mine" refuses an irreconcilable same-secret conflict: when both sides
/// changed the SAME `.age` entry, it surfaces `PushRejected` (adopt or cancel),
/// never a silent overwrite or a blob merge.
#[tokio::test]
async fn keep_mine_refuses_same_secret_conflict() {
    let (bare_dir, _cfg, store, recipient) =
        store_with_base(vec![("shared.age", b"shared-base")]).await;
    let repo_path = store.config().await.expect("config").local_path;

    // Both sides modify the same entry.
    local_secret(
        Path::new(&repo_path),
        "shared.age",
        b"ours",
        &recipient,
        "local edits shared",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("shared.age", b"theirs")],
        &recipient,
        "remote edits shared",
    );

    let tip = bare_head_oid(bare_dir.path());
    let err = store
        .resolve_sync_divergence(&tip, DivergenceChoice::KeepMine)
        .await
        .unwrap_err();
    assert_eq!(
        err.code, "PUSH_REJECTED",
        "same-secret conflict must refuse: {err}"
    );
}

/// "Keep mine" re-encrypts to the CURRENT recipient set, not a stale replay: a
/// remote recipient rotation is honored, and our own key is re-added so we can
/// still read what we kept.
#[tokio::test]
async fn keep_mine_re_encrypts_to_current_recipients() {
    let (bare_dir, _cfg, store, r1) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;

    // Local adds a secret encrypted to R1 (our key).
    local_secret(
        Path::new(&repo_path),
        "mine.age",
        b"mine-secret",
        &r1,
        "local adds mine",
    );
    // Remote rotates recipients to R2 (a different key).
    let (id2, r2) = generate_test_keypair();
    commit_plain_files_to_bare(
        bare_dir.path(),
        vec![(".gopass-recipients", r2.as_bytes())],
        "remote rotates recipients",
    );

    let tip = bare_head_oid(bare_dir.path());
    let result = store
        .resolve_sync_divergence(&tip, DivergenceChoice::KeepMine)
        .await
        .expect("keep mine");
    assert!(result.changed);

    // We (R1) can still read it — our key was re-added (ensureOurKeyID).
    assert_eq!(
        store.get("mine").await.expect("get mine").password(),
        "mine-secret"
    );

    // And it was re-encrypted to R2 (the current recipients), not a stale replay
    // of the old R1-only ciphertext: R2's identity can now decrypt the pushed copy.
    let pushed = bare_blob(bare_dir.path(), "mine.age");
    assert_eq!(
        crypto::decrypt_bytes(&pushed, id2.as_bytes(), None).expect("R2 can decrypt"),
        b"mine-secret",
        "keep mine must re-encrypt to the new recipient set"
    );
}

/// "Keep mine" preserves a local deletion: the entry is re-deleted on the remote
/// tip (the remote still had it from the base).
#[tokio::test]
async fn keep_mine_preserves_local_deletion() {
    let (bare_dir, _cfg, store, recipient) =
        store_with_base(vec![("doomed.age", b"doomed-base")]).await;
    let repo_path = store.config().await.expect("config").local_path;

    // Local deletes "doomed"; remote diverges on an unrelated file.
    {
        let repo = git2::Repository::open(&repo_path).expect("open store repo");
        let mut index = repo.index().expect("index");
        index
            .remove_path(Path::new("doomed.age"))
            .expect("remove_path");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write_tree");
        let tree = repo.find_tree(tree_id).expect("find_tree");
        let head = repo.head().expect("head").target().expect("oid");
        let parent = repo.find_commit(head).expect("parent");
        let sig = git2::Signature::now("local", "local@local").expect("sig");
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "local deletes doomed",
            &tree,
            &[&parent],
        )
        .expect("commit");
    }
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote-only.age", b"remote-secret")],
        &recipient,
        "remote diverges",
    );

    let tip = bare_head_oid(bare_dir.path());
    let result = store
        .resolve_sync_divergence(&tip, DivergenceChoice::KeepMine)
        .await
        .expect("keep mine");
    assert!(result.changed);

    // The deletion stands locally and was pushed to the remote.
    assert!(
        !Path::new(&repo_path).join("doomed.age").exists(),
        "local deletion preserved"
    );
    assert!(
        !entry_exists_on_bare(bare_dir.path(), "doomed.age"),
        "doomed.age pushed-deleted on remote"
    );
    // The unrelated remote file is still there.
    assert_eq!(
        store
            .get("remote-only")
            .await
            .expect("get remote")
            .password(),
        "remote-secret"
    );
}

/// `bare_blob` errors on a missing path; use a direct existence check instead.
fn entry_exists_on_bare(bare_path: &Path, rel: &str) -> bool {
    let repo = git2::Repository::open(bare_path).expect("open bare");
    let head = repo.head().expect("head");
    let commit = repo
        .find_commit(head.target().expect("oid"))
        .expect("commit");
    commit
        .tree()
        .expect("tree")
        .get_path(Path::new(rel))
        .is_ok()
}

/// "Keep mine" refuses if the remote advanced past the reviewed tip
/// (stale-confirmation guard) — no silent adopt/re-encrypt against a different
/// state than what was confirmed.
#[tokio::test]
async fn keep_mine_refuses_when_remote_moved() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;
    local_secret(
        Path::new(&repo_path),
        "mine.age",
        b"mine",
        &recipient,
        "local adds mine",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote.age", b"r")],
        &recipient,
        "remote diverges (1)",
    );
    let reviewed_tip = bare_head_oid(bare_dir.path());

    // Remote advances AGAIN after the user reviewed `reviewed_tip`.
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote2.age", b"r2")],
        &recipient,
        "remote diverges (2)",
    );

    let err = store
        .resolve_sync_divergence(&reviewed_tip, DivergenceChoice::KeepMine)
        .await
        .unwrap_err();
    assert_eq!(
        err.code, "PULL_FF_FAILED",
        "stale tip must be refused: {err}"
    );
}

/// "Keep mine" refuses a local entry it can't decrypt to re-encrypt (defensive —
/// in single-identity gpm every local entry is decryptable, but a corrupt blob
/// must not be silently dropped).
#[tokio::test]
async fn keep_mine_refuses_undecryptable_local_entry() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;
    // A corrupt local entry the store can't decrypt.
    local_commit_files(
        Path::new(&repo_path),
        &[("broken.age", b"not-valid-ciphertext")],
        "local adds garbage",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote.age", b"r")],
        &recipient,
        "remote diverges",
    );

    let tip = bare_head_oid(bare_dir.path());
    let err = store
        .resolve_sync_divergence(&tip, DivergenceChoice::KeepMine)
        .await
        .unwrap_err();
    assert_eq!(
        err.code, "PUSH_REJECTED",
        "undecryptable local entry must refuse: {err}"
    );
}

/// `sync_divergence_preview` reports the local-vs-remote divergence on demand
/// (without moving HEAD), matching the preview `sync()` would surface.
#[tokio::test]
async fn sync_divergence_preview_reports_local_changes() {
    let (bare_dir, _cfg, store, recipient) =
        store_with_base(vec![("shared.age", b"shared-base")]).await;
    let repo_path = store.config().await.expect("config").local_path;

    local_secret(
        Path::new(&repo_path),
        "local-only.age",
        b"local",
        &recipient,
        "local diverges",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote-only.age", b"remote-secret")],
        &recipient,
        "remote diverges",
    );

    let div = store.sync_divergence_preview().await.expect("preview");
    assert_eq!(div.remote_tip, bare_head_oid(bare_dir.path()));
    assert_eq!(div.local_ahead, 1, "one unpushed local commit");
    assert_eq!(div.remote_ahead, 1, "one remote commit");
    assert_eq!(div.local_only_entries, vec!["local-only".to_string()]);
    assert!(
        div.modified_entries.is_empty(),
        "shared was not touched locally"
    );
}

/// A strictly-local-ahead repo (unpushed commit, remote unchanged) is a NO-OP
/// pull, not a spurious divergence — the pre-fix bug that modal'd on every write
/// after an unpushed commit.
#[tokio::test]
async fn sync_local_ahead_is_noop_not_divergence() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;

    // Local adds an unpushed secret; remote is unchanged.
    local_secret(
        Path::new(&repo_path),
        "mine.age",
        b"mine",
        &recipient,
        "local unpushed",
    );

    // sync must NOT report divergence — local is strictly ahead.
    let outcome = store.sync().await.expect("sync");
    match outcome {
        SyncOutcome::FastForwarded(r) => assert!(
            !r.changed,
            "local-ahead is a no-op pull (changed=false): {r:?}"
        ),
        other => panic!("expected FastForwarded no-op, got {other:?}"),
    }

    // A push publishes the local commit (the autosync-off path).
    store.push().await.expect("push");
    assert_eq!(
        bare_head_oid(bare_dir.path()),
        local_head_oid(Path::new(&repo_path)),
        "push fast-forwards the remote to local HEAD"
    );
}
