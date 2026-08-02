// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

// API-surface lints (missing_docs, pedantic, …) target library code; tests opt out.
#![allow(
    missing_docs,
    unused_qualifications,
    trivial_casts,
    trivial_numeric_casts,
    clippy::pedantic,
    clippy::indexing_slicing
)]

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
use rustpass::GitAuth;
use rustpass::SyncOutcome;
use rustpass::crypto;
use rustpass::store::{DivergenceChoice, EntryConflictChoice, ExpectedEntry, ExpectedKind, Store};

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
        .resolve_sync_divergence(&cancel_slot(), &tip, DivergenceChoice::KeepMine, None)
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
        .resolve_sync_divergence(&cancel_slot(), &tip, DivergenceChoice::KeepMine, None)
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
        vec![(TEST_RECIPIENTS_FILE, r2.as_bytes())],
        "remote rotates recipients",
    );

    let tip = bare_head_oid(bare_dir.path());
    let result = store
        .resolve_sync_divergence(&cancel_slot(), &tip, DivergenceChoice::KeepMine, None)
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
        .resolve_sync_divergence(&cancel_slot(), &tip, DivergenceChoice::KeepMine, None)
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
        .resolve_sync_divergence(
            &cancel_slot(),
            &reviewed_tip,
            DivergenceChoice::KeepMine,
            None,
        )
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
        .resolve_sync_divergence(&cancel_slot(), &tip, DivergenceChoice::KeepMine, None)
        .await
        .unwrap_err();
    assert_eq!(
        err.code, "PUSH_REJECTED",
        "undecryptable local entry must refuse: {err}"
    );
}

/// "Keep mine" reuses the CACHED identity across the divergence boundary — it
/// does not wipe the cache or force a second unlock. This is the rustpass half
/// of the deferred-wipe contract: the save orchestrator (src-tauri) defers the
/// Immediate-mode identity wipe on a `NeedsDivergenceResolve` outcome precisely
/// so this resolve step can reuse the cached identity without re-prompting for a
/// passphrase / biometric. If `resolve_keep_mine` wiped the cache, an encrypted
/// identity would fail with `IdentityEncrypted` here (no passphrase is passed to
/// resolve), and `is_unlocked()` would read false — so the resolve succeeding
/// AND `is_unlocked()` staying true pins both that the cache was reused and that
/// it survived.
///
/// The other keep-mine tests use the DEFAULT plaintext identity, which is never
/// cached (`is_unlocked()` is always false — it decrypts straight from disk per
/// op via `get_identity_bytes`), so they exercise the disk-fallback path and
/// can't observe this cache-reuse contract. This test switches to a
/// passphrase-ENCRYPTED identity and unlocks it to populate the cache first.
///
/// (The wipe-on-cancel half of the contract — `discard_divergence` wiping the
/// cache — lives in src-tauri, which calls `Store::lock()`; it is not a rustpass
/// concern and is covered by the src-tauri in-crate tests.)
#[tokio::test]
async fn keep_mine_reuses_cached_identity_without_wiping() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;

    // Switch to a passphrase-encrypted identity, then unlock → cache populated.
    store
        .set_passphrase("cache-pin")
        .await
        .expect("set_passphrase encrypts the identity");
    store.unlock("cache-pin").await.expect("unlock");
    assert!(store.is_unlocked(), "baseline: cache populated by unlock");

    // Diverge: an unpushed local secret (encrypted to our key) + a remote advance.
    local_secret(
        Path::new(&repo_path),
        "mine.age",
        b"mine-secret",
        &recipient,
        "local adds mine",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote.age", b"remote-secret")],
        &recipient,
        "remote diverges",
    );

    let tip = bare_head_oid(bare_dir.path());
    // Keep-mine re-encrypts the local entry — no passphrase is passed here, so
    // success proves the cached identity was reused (no second unlock needed).
    let result = store
        .resolve_sync_divergence(&cancel_slot(), &tip, DivergenceChoice::KeepMine, None)
        .await
        .expect("keep mine reuses the cached identity — no second unlock");
    assert!(result.changed, "HEAD should advance");

    // The cache survived the resolve boundary — not wiped mid-flight. rustpass
    // must leave the cache intact so the orchestrator's terminal wipe is the
    // only wipe (a premature wipe here would strand the next op without a key).
    assert!(
        store.is_unlocked(),
        "keep mine must not wipe the cached identity — the deferred wipe is the caller's job"
    );

    // And the surviving cache is still functional: the kept entry decrypts
    // without a re-unlock.
    assert_eq!(
        store
            .get("mine")
            .await
            .expect("get reuses the surviving cache")
            .password(),
        "mine-secret",
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
    store.set_autosync(false);
    let bare_before = bare_head_oid(bare_dir.path());

    let s = store.clone();
    let outcome = store
        .autosync_write(&cancel_slot(), None, None, move || {
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
        .autosync_write(&cancel_slot(), None, None, move || {
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
        .autosync_write(&cancel_slot(), None, None, move || {
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
    let slot1 = cancel_slot();
    let slot2 = cancel_slot();
    let (r1, r2) = tokio::join!(
        store.autosync_write(&slot1, None, None, move || {
            let s = s1.clone();
            async move { s.set("a", b"1").await }
        }),
        store.autosync_write(&slot2, None, None, move || {
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

/// R026: with autosync on, a base-version-aware edit REFUSES to clobber a
/// teammate's newer same-name change. The user read v1; a teammate advanced the
/// same entry to v2; the user saves an edit built on the stale v1. The
/// orchestrator's pre-write pull fast-forwards local HEAD onto v2, the base-oid
/// guard sees `current (v2) != base (v1)`, and the write is refused as
/// `WriteOutcome::EntryConflict` — no commit, no push, the teammate's v2 is
/// untouched. (Was the pinned silent-clobber regression before R026.)
#[tokio::test]
async fn autosync_detects_stale_edit_same_name_change() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![("entry.age", b"v1")],
        vec![(TEST_RECIPIENTS_FILE, recipient.as_bytes())],
        &recipient,
    );
    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("utf-8"),
            &GitAuth::None,
            &identity,
            None,
        )
        .await
        .expect("configure");
    let store = Arc::new(store);

    // The base version the user read: v1's blob oid, captured at load time
    // (mirrors the edit screen pinning SensitiveContent.version).
    let v1_oid = store
        .entry_oid("entry")
        .await
        .expect("entry_oid")
        .expect("entry present at v1");

    // A teammate advances the SAME entry on the remote; the local has no
    // unpushed commit, so there is no divergence — only a behind-local.
    let newer: Vec<u8> = b"newer-from-teammate".to_vec();
    add_commit_to_bare(
        bare_dir.path(),
        vec![("entry.age", newer.as_slice())],
        &recipient,
        "remote advances same-name",
    );

    // The user, editing from the stale v1 snapshot, saves via the orchestrator
    // with the captured base oid.
    let s = store.clone();
    let outcome = store
        .autosync_write(
            &cancel_slot(),
            None,
            Some(ExpectedEntry {
                name: "entry".to_string(),
                base_oid: v1_oid.clone(),
                kind: ExpectedKind::Edit,
            }),
            move || {
                let s = s.clone();
                async move { s.set("entry", b"stale-edit").await }
            },
        )
        .await
        .expect("autosync write");

    let current_oid = match outcome {
        rustpass::WriteOutcome::EntryConflict {
            name,
            base_oid,
            current_oid,
            op,
            ..
        } => {
            assert_eq!(name, "entry");
            assert_eq!(base_oid, v1_oid);
            assert_eq!(op, ExpectedKind::Edit);
            current_oid
        }
        other => panic!("expected EntryConflict, got {other:?}"),
    };
    // The guard saw the teammate's v2 (a different oid than the v1 base).
    assert_ne!(current_oid.as_deref(), Some(v1_oid.as_str()));
    assert!(
        current_oid.is_some(),
        "entry still present (v2), not deleted"
    );

    // The teammate's v2 is untouched: the pull fast-forwarded local HEAD onto
    // v2, but the write was refused — nothing committed, nothing pushed.
    assert_eq!(
        store.get("entry").await.expect("get").password(),
        "newer-from-teammate",
        "local HEAD == teammate v2; the stale edit was refused"
    );
    let blob = bare_blob(bare_dir.path(), "entry.age");
    assert_eq!(
        crypto::decrypt_bytes(&blob, identity.as_bytes(), None).unwrap(),
        b"newer-from-teammate",
        "remote HEAD unchanged — the stale edit did NOT clobber the teammate's version"
    );
}

/// Set up the R026 stale-edit conflict for the resolve tests: a store at v1 with
/// its base oid captured, a teammate advancing "entry" to v2, and the user's
/// stale-v1 edit refused as `EntryConflict`. Returns the store + the tempdirs
/// that must outlive it + the identity/recipient (for decrypt + further remote
/// commits) + the conflict's reviewed `remote_tip` to hand to
/// `resolve_entry_conflict`.
async fn stale_edit_conflict() -> (
    Arc<Store>,
    tempfile::TempDir,
    tempfile::TempDir,
    String,
    String,
    String,
) {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![("entry.age", b"v1")],
        vec![(TEST_RECIPIENTS_FILE, recipient.as_bytes())],
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

    let v1_oid = store
        .entry_oid("entry")
        .await
        .expect("entry_oid")
        .expect("entry present at v1");

    let newer = b"newer-from-teammate".to_vec();
    add_commit_to_bare(
        bare_dir.path(),
        vec![("entry.age", newer.as_slice())],
        &recipient,
        "remote advances same-name",
    );

    let s = store.clone();
    let outcome = store
        .autosync_write(
            &cancel_slot(),
            None,
            Some(ExpectedEntry {
                name: "entry".to_string(),
                base_oid: v1_oid,
                kind: ExpectedKind::Edit,
            }),
            move || {
                let s = s.clone();
                async move { s.set("entry", b"stale-edit").await }
            },
        )
        .await
        .expect("autosync write");

    let remote_tip = match outcome {
        rustpass::WriteOutcome::EntryConflict { remote_tip, .. } => remote_tip,
        other => panic!("expected EntryConflict, got {other:?}"),
    };
    (store, bare_dir, config_dir, identity, recipient, remote_tip)
}

/// R026 resolve — keep-mine (edit): re-sending the edited plaintext overwrites the
/// teammate's version on the remote and pushes; the bare tip advances to the
/// keep-mine commit.
#[tokio::test]
async fn entry_conflict_keep_mine_edit_overwrites_and_pushes() {
    let (store, bare_dir, _config_dir, identity, _recipient, remote_tip) =
        stale_edit_conflict().await;

    let result = store
        .resolve_entry_conflict(
            &cancel_slot(),
            "entry",
            Some(b"my-edit"),
            &remote_tip,
            ExpectedKind::Edit,
            EntryConflictChoice::KeepMine,
            None,
        )
        .await
        .expect("keep-mine resolve");
    assert!(result.changed, "keep-mine advanced HEAD");
    assert!(!result.head.is_empty());

    // The user's edit landed locally and on the remote (pushed).
    assert_eq!(store.get("entry").await.expect("get").password(), "my-edit");
    let blob = bare_blob(bare_dir.path(), "entry.age");
    assert_eq!(
        crypto::decrypt_bytes(&blob, identity.as_bytes(), None).unwrap(),
        b"my-edit",
        "remote HEAD == the keep-mine edit (pushed)"
    );
}

/// R026 resolve — keep-theirs: a guarded no-op. Local HEAD already sits at the
/// reviewed tip (the teammate's v2); nothing is committed or pushed, and the bare
/// tip is unchanged.
#[tokio::test]
async fn entry_conflict_keep_theirs_is_a_noop() {
    let (store, bare_dir, _config_dir, _identity, _recipient, remote_tip) =
        stale_edit_conflict().await;
    let bare_before = bare_head_oid(bare_dir.path());

    let result = store
        .resolve_entry_conflict(
            &cancel_slot(),
            "entry",
            None,
            &remote_tip,
            ExpectedKind::Edit,
            EntryConflictChoice::KeepTheirs,
            None,
        )
        .await
        .expect("keep-theirs resolve");
    assert!(!result.changed, "keep-theirs changes nothing");

    // Local still reads the teammate's version; the remote tip didn't move.
    assert_eq!(
        store.get("entry").await.expect("get").password(),
        "newer-from-teammate"
    );
    assert_eq!(
        bare_head_oid(bare_dir.path()),
        bare_before,
        "no commit pushed"
    );
}

/// R026 resolve — TOCTOU: if the remote moved again between the conflict and the
/// resolve, keep-mine refuses (the reviewed tip is stale) instead of overwriting a
/// second unseen change.
#[tokio::test]
async fn entry_conflict_resolve_toctou_refuses_when_remote_moved() {
    let (store, bare_dir, _config_dir, _identity, recipient, remote_tip) =
        stale_edit_conflict().await;

    // A second teammate advance lands AFTER the user reviewed the conflict.
    add_commit_to_bare(
        bare_dir.path(),
        vec![("entry.age", b"v3-from-teammate")],
        &recipient,
        "remote advances again",
    );

    let err = store
        .resolve_entry_conflict(
            &cancel_slot(),
            "entry",
            Some(b"my-edit"),
            &remote_tip,
            ExpectedKind::Edit,
            EntryConflictChoice::KeepMine,
            None,
        )
        .await
        .expect_err("TOCTOU must refuse");
    assert_eq!(
        err.code, "PULL_FF_FAILED",
        "stale reviewed tip → refuse: {err}"
    );

    // The second advance is intact (not clobbered by a blind keep-mine); local
    // fast-forwarded to it during the resolve's fetch, but the write was refused.
    assert_eq!(
        store.get("entry").await.expect("get").password(),
        "v3-from-teammate",
        "the unseen v3 survived"
    );
}

/// R026 read primitive: `entry_oid` returns the blob oid for a present entry and
/// `None` for one absent at HEAD (the signal the delete no-op rule keys on).
#[tokio::test]
async fn entry_oid_present_and_absent_at_head() {
    let (_bare_dir, _cfg, store, _recipient) = store_with_base(vec![("entry.age", b"v1")]).await;
    let present = store.entry_oid("entry").await.expect("entry_oid");
    assert!(present.is_some(), "present entry → Some(oid)");
    let absent = store.entry_oid("missing").await.expect("entry_oid missing");
    assert!(absent.is_none(), "absent at HEAD → None");
}

/// R026: a non-stale edit (base oid == current oid) proceeds normally — no
/// false-positive conflict. The edit lands and pushes.
#[tokio::test]
async fn autosync_edit_proceeds_when_base_matches() {
    let (_bare_dir, _cfg, store, _recipient) = store_with_base(vec![("entry.age", b"v1")]).await;
    let store = Arc::new(store);
    let v1_oid = store
        .entry_oid("entry")
        .await
        .expect("entry_oid")
        .expect("present at v1");
    // No remote change → base == current → the guard passes.
    let s = store.clone();
    let outcome = store
        .autosync_write(
            &cancel_slot(),
            None,
            Some(ExpectedEntry {
                name: "entry".to_string(),
                base_oid: v1_oid,
                kind: ExpectedKind::Edit,
            }),
            move || {
                let s = s.clone();
                async move { s.set("entry", b"fresh-edit").await }
            },
        )
        .await
        .expect("autosync edit");
    assert!(
        matches!(outcome, rustpass::WriteOutcome::Written(_)),
        "matching base → Written, got {outcome:?}"
    );
    assert_eq!(
        store.get("entry").await.expect("get").password(),
        "fresh-edit"
    );
}

/// R026: a delete built on a stale snapshot (a teammate advanced the same entry)
/// is refused as EntryConflict — the teammate's newer version survives.
#[tokio::test]
async fn autosync_delete_refuses_when_entry_changed() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![("entry.age", b"v1")]).await;
    let store = Arc::new(store);
    let v1_oid = store
        .entry_oid("entry")
        .await
        .expect("entry_oid")
        .expect("present at v1");
    add_commit_to_bare(
        bare_dir.path(),
        vec![("entry.age", b"v2-from-teammate")],
        &recipient,
        "remote advances same-name",
    );
    let s = store.clone();
    let outcome = store
        .autosync_write(
            &cancel_slot(),
            None,
            Some(ExpectedEntry {
                name: "entry".to_string(),
                base_oid: v1_oid,
                kind: ExpectedKind::Delete,
            }),
            move || {
                let s = s.clone();
                async move { s.delete("entry").await }
            },
        )
        .await
        .expect("autosync delete");
    match outcome {
        rustpass::WriteOutcome::EntryConflict {
            op, current_oid, ..
        } => {
            assert_eq!(op, ExpectedKind::Delete);
            assert!(current_oid.is_some(), "entry still present (v2)");
        }
        other => panic!("expected EntryConflict, got {other:?}"),
    }
    // The teammate's v2 survived (the stale delete was refused, not clobbered).
    assert_eq!(
        store.get("entry").await.expect("get").password(),
        "v2-from-teammate"
    );
}

/// R026: under AutoSync-OFF the base-version guard is skipped by design — a
/// stale edit still commits locally (Written) and surfaces as a repo-level
/// divergence at the next manual Sync. Pin the documented limitation.
#[tokio::test]
async fn autosync_off_skips_base_check_even_with_expected() {
    let (bare_dir, _cfg, store, recipient) = store_with_base(vec![("entry.age", b"v1")]).await;
    let store = Arc::new(store);
    store.set_autosync(false);
    let v1_oid = store
        .entry_oid("entry")
        .await
        .expect("entry_oid")
        .expect("present at v1");
    add_commit_to_bare(
        bare_dir.path(),
        vec![("entry.age", b"v2-from-teammate")],
        &recipient,
        "remote advances same-name",
    );
    let s = store.clone();
    let outcome = store
        .autosync_write(
            &cancel_slot(),
            None,
            Some(ExpectedEntry {
                name: "entry".to_string(),
                base_oid: v1_oid,
                kind: ExpectedKind::Edit,
            }),
            move || {
                let s = s.clone();
                async move { s.set("entry", b"local-edit").await }
            },
        )
        .await
        .expect("autosync-off edit");
    assert!(
        matches!(outcome, rustpass::WriteOutcome::Written(_)),
        "autosync-off skips the base check → Written, got {outcome:?}"
    );
}

// ── Store::sync_repo — manual pull → push (the Sync button) ──────────────────

/// `sync_repo` publishes unpushed local commits when autosync is off: a local
/// write commits, then `sync_repo` advances the bare tip to it (FastForwarded).
#[tokio::test]
async fn sync_repo_publishes_local_commits() {
    let (bare_dir, _cfg, store, _recipient) = store_with_base(vec![]).await;
    let store = Arc::new(store);
    store.set_autosync(false);
    store
        .set("offline", b"local-then-sync")
        .await
        .expect("local write");
    let bare_before = bare_head_oid(bare_dir.path());

    let outcome = store
        .sync_repo(&cancel_slot(), None, None)
        .await
        .expect("sync_repo");
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

    let outcome = store
        .sync_repo(&cancel_slot(), None, None)
        .await
        .expect("sync_repo");
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
