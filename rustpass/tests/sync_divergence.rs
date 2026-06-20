// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Pull/sync divergence detection + resolution (plan 0012). `Store::sync`
//! returns `SyncOutcome::Diverged` instead of a hard `PullFfFailed`, carrying
//! the full local-side change preview; `Store::resolve_sync_divergence` adopts
//! the exact remote tip the user reviewed.
//!
//! The setup helpers overlap with `write_conflict.rs`; if a third test file
//! needs them, lift `local_commit_files` / `store_with_base` into `common`.

mod common;

use std::path::Path;

use common::*;
use rustpass::store::Store;
use rustpass::{SyncDivergence, SyncOutcome};

/// Commit `files` (rel path → bytes) in the store's working repo WITHOUT
/// pushing — the unpushed local commit that creates divergence.
fn local_commit_files(repo_path: &Path, files: &[(&str, &[u8])], message: &str) {
    let repo = git2::Repository::open(repo_path).expect("open store repo");
    for (rel, content) in files {
        let file_path = repo_path.join(rel);
        if let Some(p) = file_path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(&file_path, content).unwrap();
    }
    let mut index = repo.index().expect("index");
    for (rel, _) in files {
        index.add_path(Path::new(rel)).expect("add_path");
    }
    index.write().expect("write index");
    let tree_id = index.write_tree().expect("write_tree");
    let tree = repo.find_tree(tree_id).expect("find_tree");
    let head = repo.head().expect("head").target().expect("oid");
    let parent = repo.find_commit(head).expect("parent");
    let sig = git2::Signature::now("local", "local@local").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
        .expect("commit");
}

/// Configure a store against a fresh bare repo carrying a recipients file
/// plus `base_entries` (encrypted). Returns `(bare_dir, store, recipient)`.
async fn store_with_base(
    base_entries: Vec<(&str, &[u8])>,
) -> (tempfile::TempDir, tempfile::TempDir, Store, String) {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        base_entries,
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
    (bare_dir, config_dir, store, recipient)
}

/// Full HEAD oid of the bare repo's current branch tip.
fn bare_head_oid(bare_path: &Path) -> String {
    let repo = git2::Repository::open(bare_path).expect("open bare");
    repo.head()
        .expect("head")
        .target()
        .expect("oid")
        .to_string()
}

/// `sync()` surfaces `Diverged` (not "already up to date", not an error) with
/// correct ahead-counts and a full local-side change preview (ARCH1 + codex ③).
#[tokio::test]
async fn sync_detects_divergence_and_classifies_local_changes() {
    let (bare_dir, _config_dir, store, recipient) =
        store_with_base(vec![("shared.age", b"shared-base")]).await;
    let repo_path = store.config().await.expect("config").local_path;

    // Local diverges: modify shared.age, add a local-only secret, add a
    // non-secret file.
    local_commit_files(
        Path::new(&repo_path),
        &[
            ("shared.age", b"shared-local"),
            ("local-only.age", b"local-secret"),
            ("notes.txt", b"notes"),
        ],
        "local diverges",
    );
    // Remote diverges: add a remote-only secret (not a local loss).
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote-only.age", b"remote-secret")],
        &recipient,
        "remote diverges",
    );

    let div = match store.sync().await.expect("sync") {
        SyncOutcome::Diverged(d) => d,
        other => panic!("expected Diverged, got {other:?}"),
    };
    assert_eq!(div.local_ahead, 1, "one unpushed local commit");
    assert_eq!(div.remote_ahead, 1, "one remote commit");
    assert_eq!(div.local_only_entries, vec!["local-only".to_string()]);
    assert_eq!(div.modified_entries, vec!["shared".to_string()]);
    assert_eq!(div.other_changed_files, vec!["notes.txt".to_string()]);
    // A remote-only entry is a remote gain, not a local loss.
    assert!(
        !div.local_only_entries.contains(&"remote-only".to_string())
            && !div.modified_entries.contains(&"remote-only".to_string()),
        "remote-only entry must not appear as a local loss"
    );
    assert_eq!(div.remote_tip, bare_head_oid(bare_dir.path()));
}

/// `resolve_sync_divergence` adopts the reviewed remote tip: HEAD advances,
/// local-only secrets disappear, remote-only secrets appear.
#[tokio::test]
async fn resolve_adopts_reviewed_remote_tip() {
    let (bare_dir, _config_dir, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;

    local_commit_files(
        Path::new(&repo_path),
        &[("local-only.age", b"local-secret")],
        "local diverges",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote-only.age", b"remote-secret")],
        &recipient,
        "remote diverges",
    );

    let SyncDivergence {
        remote_tip,
        local_only_entries,
        ..
    } = match store.sync().await.expect("sync") {
        SyncOutcome::Diverged(d) => d,
        other => panic!("expected Diverged, got {other:?}"),
    };
    assert_eq!(local_only_entries, vec!["local-only".to_string()]);

    let result = store
        .resolve_sync_divergence(&remote_tip)
        .await
        .expect("adopt");
    assert!(result.changed, "HEAD should advance to the remote tip");
    assert_eq!(result.head, bare_head_oid(bare_dir.path())[..7]);

    // Adopting the remote discards the local-only secret and gains the
    // remote-only one.
    assert!(
        !Path::new(&repo_path).join("local-only.age").exists(),
        "local-only secret must be gone after adopt"
    );
    assert!(
        Path::new(&repo_path).join("remote-only.age").exists(),
        "remote-only secret must be present after adopt"
    );
}

/// `resolve_sync_divergence` refuses if the remote advanced past the tip the
/// user reviewed (codex ② stale-confirmation guard) — no silent adopt of a
/// different state than what was confirmed.
#[tokio::test]
async fn resolve_refused_when_remote_moved() {
    let (bare_dir, _config_dir, store, recipient) = store_with_base(vec![]).await;
    let repo_path = store.config().await.expect("config").local_path;

    local_commit_files(
        Path::new(&repo_path),
        &[("local-only.age", b"local-secret")],
        "local diverges",
    );
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote-1.age", b"one")],
        &recipient,
        "remote diverges (1)",
    );

    let SyncDivergence { remote_tip, .. } = match store.sync().await.expect("sync") {
        SyncOutcome::Diverged(d) => d,
        other => panic!("expected Diverged, got {other:?}"),
    };

    // Remote advances AGAIN after the user reviewed `remote_tip`.
    add_commit_to_bare(
        bare_dir.path(),
        vec![("remote-2.age", b"two")],
        &recipient,
        "remote diverges (2)",
    );

    let err = store
        .resolve_sync_divergence(&remote_tip)
        .await
        .unwrap_err();
    assert_eq!(
        err.code, "PULL_FF_FAILED",
        "adopt of a stale tip must be refused, got {err}"
    );
    // HEAD unchanged — the local-only secret is still there.
    assert!(
        Path::new(&repo_path).join("local-only.age").exists(),
        "HEAD must not move on a refused adopt"
    );
}
