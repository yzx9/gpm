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
use std::sync::Arc;

use common::*;
use rustpass::SyncOutcome;
use rustpass::crypto;
use rustpass::store::{DivergenceChoice, Store};

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

/// Unwrap a `WriteOutcome::Written`'s commit hash, panicking on any other
/// variant — the autosync success-path tests all expect `Written`.
fn written_commit(outcome: rustpass::WriteOutcome) -> String {
    match outcome {
        rustpass::WriteOutcome::Written(w) => w.commit,
        other => panic!("expected WriteOutcome::Written, got {other:?}"),
    }
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

// ── Store::autosync_write — the pull → write → push orchestrator ─────────────

/// Autosync OFF: `autosync_write` runs the local write only — no pull, no push.
/// The entry commits locally and the remote (bare) is unchanged.
#[tokio::test]
async fn autosync_off_skips_network() {
    let (bare_dir, _cfg, store, _recipient) = store_with_base(vec![]).await;
    let store = Arc::new(store);
    store.set_autosync(false).await.expect("autosync off");
    let bare_before = bare_head_oid(bare_dir.path());

    let s = store.clone();
    let outcome = store
        .autosync_write(None, move || {
            let s = s.clone();
            async move { s.set("offline-entry", b"local-only").await }
        })
        .await
        .expect("autosync-off write");

    assert!(!written_commit(outcome).is_empty(), "local commit was made");
    assert_eq!(
        store.get("offline-entry").await.expect("get").password(),
        "local-only"
    );
    assert_eq!(
        bare_head_oid(bare_dir.path()),
        bare_before,
        "autosync off must NOT push — the remote is unchanged"
    );
}

/// Autosync ON (the default): `autosync_write` pulls, writes, and pushes — the
/// remote (bare) advances to the new commit and the entry is readable.
#[tokio::test]
async fn autosync_on_publishes_via_pull_write_push() {
    let (bare_dir, _cfg, store, _recipient) = store_with_base(vec![]).await;
    let store = Arc::new(store);
    let bare_before = bare_head_oid(bare_dir.path());

    let s = store.clone();
    let outcome = store
        .autosync_write(None, move || {
            let s = s.clone();
            async move { s.set("published", b"via-orchestrator").await }
        })
        .await
        .expect("autosync-on write");
    let commit = written_commit(outcome);

    assert!(!commit.is_empty(), "commit was made");
    // The push published: the bare tip advanced to our commit.
    assert_ne!(bare_head_oid(bare_dir.path()), bare_before);
    assert!(
        bare_head_oid(bare_dir.path()).starts_with(&commit),
        "bare tip is the orchestrator's pushed commit"
    );
    assert_eq!(
        store.get("published").await.expect("get").password(),
        "via-orchestrator"
    );
}

/// Autosync ON with a divergent remote: the orchestrator's pull sees divergence
/// (benign — it proceeds), the local write commits on the diverged HEAD, and the
/// push is rejected — surfacing as `WriteOutcome::NeedsDivergenceResolve` with a
/// populated preview (no second round-trip). This is the push-rejection race the
/// divergence modal catches (NOT the stale-read clobber — see
/// `autosync_silently_clobbers_remote_same_name_change`).
#[tokio::test]
async fn autosync_on_push_rejected_returns_needs_divergence_resolve() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;
    let store = Arc::new(store);

    // Diverge: an unpushed local commit AND a remote advance on another file.
    let unpushed: &[u8] = b"x";
    local_commit_files(
        Path::new(&repo_path),
        &[("local-only.txt", unpushed)],
        "local-only",
    );
    let remote_blob: Vec<u8> = b"r".to_vec();
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote.age", remote_blob.as_slice())],
        &recipient,
        "remote advance",
    );

    let s = store.clone();
    let outcome = store
        .autosync_write(None, move || {
            let s = s.clone();
            async move { s.set("new", b"v").await }
        })
        .await
        .expect("a divergent push surfaces as NeedsDivergenceResolve");
    match outcome {
        rustpass::WriteOutcome::NeedsDivergenceResolve(div) => {
            assert!(
                !div.remote_tip.is_empty(),
                "carries a populated divergence preview"
            );
            assert!(
                div.local_ahead >= 1,
                "local is ahead by the just-made commit(s)"
            );
            assert!(
                div.remote_ahead >= 1,
                "remote is ahead — the cause of the push rejection"
            );
        }
        other => panic!("expected NeedsDivergenceResolve, got {other:?}"),
    }
}

/// Two `autosync_write` calls in parallel both complete and both entries land —
/// the `write_mu` critical section serializes them (no deadlock, no git-index
/// corruption). The local commits interleave cleanly under the lock.
#[tokio::test]
async fn autosync_concurrent_writes_both_land() {
    let (bare_dir, _cfg, store, _recipient) = store_with_base(vec![]).await;
    let store = Arc::new(store);

    let s1 = store.clone();
    let s2 = store.clone();
    let (r1, r2) = tokio::join!(
        store.autosync_write(None, move || {
            let s = s1.clone();
            async move { s.set("a", b"1").await }
        }),
        store.autosync_write(None, move || {
            let s = s2.clone();
            async move { s.set("b", b"2").await }
        }),
    );
    let _ = r1.expect("concurrent write a");
    let _ = r2.expect("concurrent write b");

    assert_eq!(store.get("a").await.expect("get a").password(), "1");
    assert_eq!(store.get("b").await.expect("get b").password(), "2");
    // Both commits published.
    assert!(bare_head_oid(bare_dir.path()).len() > 7);
}

/// ⚠️ ACCEPTED LIMITATION (`0026`, pinned here): with autosync on, a stale edit
/// silently clobbers a teammate's newer same-name change. The orchestrator's
/// pre-write pull fast-forwards over the remote's newer version, the local write
/// commits on top, and the push fast-forwards — returning `Ok`, not a conflict.
/// Base-version-aware edit is the deferred fix. This test pins the clobber so it
/// can't drift silently.
#[tokio::test]
async fn autosync_silently_clobbers_remote_same_name_change() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![("entry.age", b"v1")],
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
    let store = Arc::new(store);

    // A teammate advances the SAME entry on the remote; the local has no
    // unpushed commit, so there is no divergence — only a behind-local.
    let newer: Vec<u8> = b"newer-from-teammate".to_vec();
    add_commit_to_bare(
        bare_dir.path(),
        vec![("entry.age", newer.as_slice())],
        &recipient,
        "remote advances same-name",
    );

    // The user, editing from the stale v1 snapshot, saves via the orchestrator.
    let s = store.clone();
    let outcome = store
        .autosync_write(None, move || {
            let s = s.clone();
            async move { s.set("entry", b"stale-edit").await }
        })
        .await
        .expect("autosync write");
    assert!(
        !written_commit(outcome).is_empty(),
        "no divergence → fast-forward → Ok Written (the silent clobber)"
    );

    // The teammate's newer version was silently overwritten by the stale edit.
    assert_eq!(
        store.get("entry").await.expect("get").password(),
        "stale-edit"
    );
    let blob = bare_blob(bare_dir.path(), "entry.age");
    assert_eq!(
        crypto::decrypt_bytes(&blob, identity.as_bytes(), None).unwrap(),
        b"stale-edit",
        "remote HEAD has the stale edit, not the teammate's newer version — the clobber"
    );
}

// ── Store::sync_repo — manual pull → push (the Sync button) ──────────────────

/// `sync_repo` publishes unpushed local commits when autosync is off: a local
/// write commits, then `sync_repo` advances the bare tip to it (FastForwarded).
#[tokio::test]
async fn sync_repo_publishes_local_commits() {
    let (bare_dir, _cfg, store, _recipient) = store_with_base(vec![]).await;
    let store = Arc::new(store);
    store.set_autosync(false).await.expect("autosync off");
    store
        .set("offline", b"local-then-sync")
        .await
        .expect("local write");
    let bare_before = bare_head_oid(bare_dir.path());

    let outcome = store.sync_repo(None, None).await.expect("sync_repo");
    match outcome {
        SyncOutcome::FastForwarded(r) => {
            assert!(
                !r.head.is_empty(),
                "FastForwarded carries the post-push head"
            );
            assert_ne!(
                bare_head_oid(bare_dir.path()),
                bare_before,
                "the push published — the bare tip advanced"
            );
        }
        other => panic!("expected FastForwarded, got {other:?}"),
    }
    assert_eq!(
        store.get("offline").await.expect("get").password(),
        "local-then-sync"
    );
}

/// `sync_repo` with a pull-side divergence returns `Diverged` without pushing —
/// the bare tip is unchanged; the UI shows the resolve modal. (A push-rejection
/// race within `sync_repo` would surface the same `Diverged` outcome; that path
/// is exercised deterministically for the write orchestrator in
/// `autosync_on_push_rejected_returns_needs_divergence_resolve` — orchestrating
/// a mid-flight remote commit between pull and push isn't reliably raceable.)
#[tokio::test]
async fn sync_repo_pull_diverged_returns_diverged() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;
    let store = Arc::new(store);

    // Diverge: an unpushed local commit AND a remote advance on another file.
    local_commit_files(
        Path::new(&repo_path),
        &[("local-only.txt", b"x")],
        "local-only",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote.age", b"r".as_slice())],
        &recipient,
        "remote advance",
    );
    let bare_after_advance = bare_head_oid(bare_dir.path());

    let outcome = store.sync_repo(None, None).await.expect("sync_repo");
    match outcome {
        SyncOutcome::Diverged(div) => {
            assert!(
                !div.remote_tip.is_empty(),
                "carries the reviewed remote tip"
            );
            assert!(div.local_ahead >= 1, "local is ahead");
            assert!(div.remote_ahead >= 1, "remote is ahead");
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
    // sync_repo returned Diverged WITHOUT pushing — the bare tip is still the
    // post-advance tip (sync_repo added nothing).
    assert_eq!(
        bare_head_oid(bare_dir.path()),
        bare_after_advance,
        "sync_repo must not push when the pull diverged"
    );
}
