// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

// API-surface lints (missing_docs, pedantic, …) target library code; tests opt out.
#![allow(
    missing_docs,
    unused_qualifications,
    trivial_casts,
    trivial_numeric_casts,
    clippy::pedantic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use rustpass::crypto;
use rustpass::secret::Secret;
use rustpass::signing::AuthenticityConfig;
use rustpass::storage::{GitAuth, GitStorage, StorageBackend, StorageCtx};
use rustpass::store;

// -----------------------------------------------------------------------
// clone tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn clone_local_bare_repo() {
    let (_identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) =
        create_test_git_repo(vec![("example.age", b"password123")], &recipient);

    let dest = tempfile::tempdir().expect("failed to create dest dir");
    let dest_path = dest.path().to_path_buf();
    drop(dest);

    GitStorage::new(&dest_path)
        .clone_repo(
            &GitAuth::None,
            bare_dir.path().to_str().expect("bare path is valid utf-8"),
            None,
            None,
        )
        .await
        .expect("clone should succeed");

    assert!(
        dest_path.join(".git").is_dir(),
        "cloned repo must contain a .git directory"
    );
}

#[tokio::test]
async fn clone_removes_existing_dest() {
    let (_identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo(vec![("real.age", b"secret")], &recipient);

    let dest = tempfile::tempdir().expect("failed to create dest dir");

    std::fs::write(dest.path().join("stale-file.txt"), b"old data")
        .expect("failed to write stale file");
    assert!(
        dest.path().join("stale-file.txt").exists(),
        "precondition: stale file must exist before clone"
    );

    GitStorage::new(dest.path())
        .clone_repo(
            &GitAuth::None,
            bare_dir.path().to_str().expect("bare path is valid utf-8"),
            None,
            None,
        )
        .await
        .expect("clone should succeed");

    assert!(
        !dest.path().join("stale-file.txt").exists(),
        "stale file must be removed by clone"
    );
    assert!(
        dest.path().join(".git").is_dir(),
        "cloned repo must contain a .git directory"
    );
}

// -----------------------------------------------------------------------
// pull tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn pull_fast_forward_succeeds() {
    let (_identity, recipient) = generate_test_keypair();
    let (bare_dir, clone_dir) =
        create_test_git_repo(vec![("initial.age", b"first-password")], &recipient);

    let repo_before = git2::Repository::open(clone_dir.path()).expect("open clone repo");
    let head_before = repo_before
        .head()
        .expect("get head")
        .target()
        .expect("head oid");
    drop(repo_before);

    let new_oid = add_commit_to_bare(
        bare_dir.path(),
        vec![("second.age", b"second-password")],
        &recipient,
        "add second entry",
    );

    let auth = GitAuth::None;
    let policy = AuthenticityConfig::default();
    let ctx = StorageCtx {
        auth: &auth,
        policy: &policy,
        commit_name: None,
        commit_email: None,
    };
    let result = expect_fast_forwarded(
        GitStorage::new(clone_dir.path())
            .pull(&ctx, crypto::SecretExt::AGE, None, None)
            .await
            .expect("pull should succeed"),
    );

    assert_ne!(
        result.head,
        format!("{head_before:.7}"),
        "HEAD hash should advance past the original after fast-forward"
    );
    assert_eq!(
        result.head,
        format!("{new_oid:.7}"),
        "HEAD should match the new upstream commit"
    );
}

#[tokio::test]
async fn pull_no_changes() {
    let (_identity, recipient) = generate_test_keypair();
    let (_bare_dir, clone_dir) =
        create_test_git_repo(vec![("sole.age", b"only-password")], &recipient);

    let auth = GitAuth::None;
    let policy = AuthenticityConfig::default();
    let ctx = StorageCtx {
        auth: &auth,
        policy: &policy,
        commit_name: None,
        commit_email: None,
    };
    let result = expect_fast_forwarded(
        GitStorage::new(clone_dir.path())
            .pull(&ctx, crypto::SecretExt::AGE, None, None)
            .await
            .expect("pull should succeed"),
    );
    assert!(
        !result.changed,
        "pull should report no changes when upstream is unchanged"
    );
}

#[tokio::test]
async fn pull_nonexistent_repo_errors() {
    let nowhere = tempfile::tempdir().expect("failed to create temp dir");

    let auth = GitAuth::None;
    let policy = AuthenticityConfig::default();
    let ctx = StorageCtx {
        auth: &auth,
        policy: &policy,
        commit_name: None,
        commit_email: None,
    };
    let result = GitStorage::new(nowhere.path())
        .pull(&ctx, crypto::SecretExt::AGE, None, None)
        .await;
    let err = result.expect_err("pull on non-repo dir should fail");
    assert_eq!(
        err.code, "NO_REPO",
        "expected NO_REPO error code, got: {err}"
    );
}

#[tokio::test]
async fn clone_nonexistent_remote_errors() {
    let nowhere = tempfile::tempdir().expect("failed to create temp dir");
    let fake_url = nowhere.path().join("no-such-repo.git");
    assert!(!fake_url.exists(), "precondition: path must not exist");

    let dest = tempfile::tempdir().expect("failed to create dest dir");
    let result = GitStorage::new(dest.path())
        .clone_repo(
            &GitAuth::None,
            fake_url.to_str().expect("path is valid utf-8"),
            None,
            None,
        )
        .await;
    let err = result.expect_err("clone from nonexistent remote should fail");
    assert_eq!(
        err.code, "CLONE_FAILED",
        "expected CLONE_FAILED for nonexistent remote, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Full workflow (clone + list + decrypt)
// -----------------------------------------------------------------------

#[tokio::test]
async fn full_workflow_clone_list_decrypt() {
    let (identity, recipient) = generate_test_keypair();

    let entries: Vec<(&str, &[u8])> = vec![
        (
            "cloud/aws/root.age",
            b"AWS-SECRET-KEY\nuser: admin\nnotes: root account" as &[u8],
        ),
        (
            "email/gmail.age",
            b"gmail-password\nuser: alice@gmail.com" as &[u8],
        ),
        ("ssh/server.age", b"ssh-key-password" as &[u8]),
    ];

    let (bare_dir, _clone_dir) = create_test_git_repo(entries.clone(), &recipient);

    let dest = tempfile::tempdir().expect("failed to create dest dir");
    GitStorage::new(dest.path())
        .clone_repo(
            &GitAuth::None,
            bare_dir.path().to_str().expect("bare path is valid utf-8"),
            None,
            None,
        )
        .await
        .expect("clone should succeed");

    let found = store::list_entries(dest.path(), rustpass::crypto::SecretExt::AGE)
        .expect("list_entries should succeed");
    assert_eq!(
        found.len(),
        entries.len(),
        "should find exactly the entries that were committed"
    );
    assert!(found.iter().any(|e| e.name == "cloud/aws/root"));
    assert!(found.iter().any(|e| e.name == "email/gmail"));
    assert!(found.iter().any(|e| e.name == "ssh/server"));

    let file_path =
        store::resolve_entry_path(dest.path(), "cloud/aws/root.age").expect("resolve entry path");
    let decrypted = crypto::decrypt_file(&file_path, identity.as_bytes(), None)
        .await
        .expect("decrypt should succeed with correct identity");

    let parsed = Secret::parse(&decrypted).expect("parse should succeed");
    assert_eq!(
        parsed.password(),
        "AWS-SECRET-KEY",
        "password must match first line of plaintext"
    );
    assert_eq!(
        parsed.get("user"),
        Some(b"admin".as_slice()),
        "user attribute must be present"
    );
    assert_eq!(
        parsed.attribute_str("notes"),
        Some("root account"),
        "notes attribute must carry the root-account value"
    );
}
