// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

use std::path::Path;

use common::*;
use rustpass::store::{ConflictChoice, Store, WriteConflict, WriteOutcome, WriteResult};

/// Count commits reachable from a repo's HEAD.
fn head_commit_count(repo_path: &Path) -> usize {
    let repo = git2::Repository::open(repo_path).expect("open repo");
    let head = repo.head().expect("head").target().expect("oid");
    let mut revwalk = repo.revwalk().expect("revwalk");
    revwalk.push(head).expect("push head");
    revwalk.count()
}

/// Read the (name, email) of a repo's HEAD commit author.
fn author_of_head(repo_path: &Path) -> (Option<String>, Option<String>) {
    let repo = git2::Repository::open(repo_path).expect("open repo");
    let head = repo.head().expect("head").target().expect("oid");
    let commit = repo.find_commit(head).expect("find commit");
    let author = commit.author();
    (
        author.name().map(String::from),
        author.email().map(String::from),
    )
}

/// Configure a store backed by a fresh bare repo that ships a recipients
/// file, so `set` has recipients to encrypt to. Generates its own keypair.
async fn writable_store() -> (tempfile::TempDir, tempfile::TempDir, Store, Vec<u8>) {
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
        .expect("configure should succeed");

    let identity_bytes = identity.into_bytes();
    (bare_dir, config_dir, store, identity_bytes)
}

/// Full LOCAL write flow: `set` encrypts + commits locally. The remote (bare)
/// is NOT advanced — `set` no longer pushes (the autosync orchestrator does).
/// This pins the local-only regression (HEAD +1 local, origin unchanged).
#[tokio::test]
async fn set_writes_encrypts_and_commits_locally() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![],
        vec![(".gopass-recipients", recipient.as_bytes())],
        &recipient,
    );
    let bare_commits_before = head_commit_count(bare_dir.path());

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
        .expect("configure should succeed");
    let repo_path = store.config().await.expect("config").local_path;
    let local_commits_before = head_commit_count(Path::new(&repo_path));

    let result = store
        .set("cloud/aws/root", b"s3kr3t-password\nuser: admin")
        .await
        .expect("set should succeed");
    assert!(!result.commit.is_empty(), "set should return a commit hash");

    // 1. Local HEAD advanced by exactly one commit; the remote (origin) did NOT.
    assert_eq!(
        head_commit_count(Path::new(&repo_path)),
        local_commits_before + 1,
        "set commits locally (HEAD +1)"
    );
    assert_eq!(
        head_commit_count(bare_dir.path()),
        bare_commits_before,
        "set does NOT push — the bare remote is unchanged"
    );

    // 2. The local store lists the new entry and reads it back (decrypt round-trip).
    let entries = store.list().await.expect("list");
    assert!(entries.iter().any(|e| e.name == "cloud/aws/root"));
    let secret = store.get("cloud/aws/root").await.expect("get");
    assert_eq!(secret.password(), "s3kr3t-password");
    assert!(secret.body().contains("user: admin"));
}

/// A configured commit identity flows into the LOCAL commit's author (set no
/// longer pushes, so the author is checked on the local HEAD).
#[tokio::test]
async fn write_commit_uses_configured_identity() {
    let (_bare_dir, _config_dir, store, _id) = writable_store().await;
    let repo_path = store.config().await.expect("config").local_path;
    store
        .set_commit_identity(
            Some("Alice".to_string()),
            Some("alice@example.com".to_string()),
        )
        .await
        .expect("set_commit_identity");

    store
        .set("cloud/aws/root", b"s3kr3t\n")
        .await
        .expect("set should succeed");

    let (name, email) = author_of_head(Path::new(&repo_path));
    assert_eq!(name.as_deref(), Some("Alice"));
    assert_eq!(email.as_deref(), Some("alice@example.com"));
}

/// With no identity configured, commits fall back to the shipped default.
#[tokio::test]
async fn write_commit_falls_back_to_default_identity() {
    let (_bare_dir, _config_dir, store, _id) = writable_store().await;
    let repo_path = store.config().await.expect("config").local_path;

    store
        .set("cloud/aws/root", b"s3kr3t\n")
        .await
        .expect("set should succeed");

    let default = Store::commit_identity_default();
    let (name, email) = author_of_head(Path::new(&repo_path));
    assert_eq!(name.as_deref(), Some(default.name.as_str()));
    assert_eq!(email.as_deref(), Some(default.email.as_str()));
}

/// Writing a nested entry creates intermediate directories (checked locally).
#[tokio::test]
async fn set_creates_nested_directories() {
    let (_bare_dir, _config_dir, store, _id) = writable_store().await;

    store
        .set("a/b/c/deep", b"deep-secret")
        .await
        .expect("set nested");

    let secret = store.get("a/b/c/deep").await.expect("get nested");
    assert_eq!(secret.password(), "deep-secret");
}

/// Our own key is always able to read what we write, even when the
/// recipients file lists additional recipients we don't hold (ensureOurKeyID).
#[tokio::test]
async fn set_encrypts_to_all_recipients_and_stays_readable_by_us() {
    let (identity, recipient) = generate_test_keypair();
    let (_other_identity, other_recipient) = generate_test_keypair();

    // recipients file lists our key AND a second recipient we don't own.
    let recipients = format!("{recipient}\n{other_recipient}\n");
    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![],
        vec![(".gopass-recipients", recipients.as_bytes())],
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

    store
        .set("shared/entry", b"team-secret")
        .await
        .expect("set");

    // We (first identity) can still decrypt it — our key was an encryption target.
    let secret = store.get("shared/entry").await.expect("get");
    assert_eq!(secret.password(), "team-secret");
}

/// Invalid secret names are rejected before any git/crypto work.
#[tokio::test]
async fn set_rejects_invalid_names() {
    let (_bare_dir, _config_dir, store, _id) = writable_store().await;

    for bad in [
        "",
        "  ",
        "/leading",
        "trailing/",
        "a//b",
        "..",
        "foo/../bar",
        "a/..",
    ] {
        let err = store.set(bad, b"x").await.unwrap_err();
        assert_eq!(
            err.code, "INVALID_ENTRY_NAME",
            "name {bad:?} should be rejected as INVALID_ENTRY_NAME, got {err}"
        );
    }
}

/// Path-traversal names are rejected (defense for the write path).
#[tokio::test]
async fn set_rejects_path_traversal() {
    let (_bare_dir, _config_dir, store, _id) = writable_store().await;

    let err = store.set("../escape", b"x").await.unwrap_err();
    assert_eq!(err.code, "INVALID_ENTRY_NAME");
}

/// Overwriting an existing local entry re-encrypts and commits (checked locally).
#[tokio::test]
async fn set_overwrites_existing_entry() {
    let (_bare_dir, _config_dir, store, _id) = writable_store().await;

    store
        .set("rotate/me", b"old-password")
        .await
        .expect("set 1");
    store
        .set("rotate/me", b"new-password")
        .await
        .expect("set 2");

    let secret = store.get("rotate/me").await.expect("get");
    assert_eq!(secret.password(), "new-password");
}

/// `WriteOutcome` serializes as a `kind`-tagged object — the IPC contract
/// the frontend consumes as a discriminated union. (`Conflict` is dead-but-
/// present until `PR2c`; the serialization shape is pinned here.)
#[test]
fn write_outcome_serializes_tagged() {
    let written = WriteOutcome::Written(WriteResult {
        commit: "abc1234".into(),
    });
    assert_eq!(
        serde_json::to_string(&written).unwrap(),
        r#"{"kind":"written","commit":"abc1234"}"#
    );

    let conflict = WriteOutcome::Conflict(WriteConflict {
        name: "sites/foo".into(),
        remote_decryptable: true,
    });
    assert_eq!(
        serde_json::to_string(&conflict).unwrap(),
        r#"{"kind":"conflict","name":"sites/foo","remote_decryptable":true}"#
    );
}

/// `ConflictChoice` round-trips as snake_case — it crosses IPC as a
/// `resolve_write_conflict` command argument. (Frozen for `PR2c` retirement.)
#[test]
fn conflict_choice_round_trips_snake_case() {
    for (choice, s) in [
        (ConflictChoice::KeepMine, "\"keep_mine\""),
        (ConflictChoice::KeepMineForce, "\"keep_mine_force\""),
        (ConflictChoice::KeepRemote, "\"keep_remote\""),
        (ConflictChoice::Cancel, "\"cancel\""),
    ] {
        assert_eq!(serde_json::to_string(&choice).unwrap(), s);
        let back: ConflictChoice = serde_json::from_str(s).unwrap();
        assert_eq!(back, choice);
    }
}
