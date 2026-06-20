// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

use common::*;
use rustpass::store::Store;

/// Full lifecycle: create → configure → list → get → sync → config → reset.
#[tokio::test]
async fn store_facade_full_lifecycle() {
    let (identity, recipient) = generate_test_keypair();

    let (bare_dir, _clone_dir) = create_test_git_repo(
        vec![
            ("cloud/aws/root.age", b"AWS-KEY\nuser: admin"),
            ("email/gmail.age", b"gmail-pw\nuser: alice@gmail.com"),
        ],
        &recipient,
    );

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);

    // 1. Not configured initially
    assert!(!store.is_configured(), "should not be configured initially");

    // 2. Configure
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");
    assert!(store.is_configured(), "should be configured after setup");

    // 3. List
    let entries = store.list().await.expect("list should succeed");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.name == "cloud/aws/root"));
    assert!(entries.iter().any(|e| e.name == "email/gmail"));

    // 4. Get
    let secret = store
        .get("cloud/aws/root")
        .await
        .expect("get should succeed");
    assert_eq!(secret.password(), "AWS-KEY");
    assert!(secret.body().contains("user: admin"));

    // 5. Sync (no changes)
    let sync_result = expect_fast_forwarded(store.sync().await.expect("sync should succeed"));
    assert!(!sync_result.changed, "no upstream changes expected");

    // 6. Config
    let repo_config = store.config().await.expect("config should succeed");
    assert!(!repo_config.url.is_empty(), "config URL should be set");
    assert_eq!(repo_config.pat, None);

    // 7. Reset
    store.reset().await.expect("reset should succeed");
    assert!(
        !store.is_configured(),
        "should not be configured after reset"
    );
}

/// `set_commit_identity` persists a custom author, trims whitespace, and
/// clears (reverts to the default) on `None`/blank — the auto-update rule.
#[tokio::test]
async fn set_commit_identity_persists_trims_and_clears() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo(vec![], &recipient);
    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    // Freshly configured → no commit identity (uses the shipped default).
    let rc = store.config().await.expect("config");
    assert_eq!(rc.commit_user_name, None);
    assert_eq!(rc.commit_user_email, None);

    // Custom values are trimmed and persisted; the call returns the result.
    let rc = store
        .set_commit_identity(
            Some("  Alice  ".to_string()),
            Some("alice@example.com".to_string()),
        )
        .await
        .expect("set_commit_identity");
    assert_eq!(rc.commit_user_name.as_deref(), Some("Alice"));
    assert_eq!(rc.commit_user_email.as_deref(), Some("alice@example.com"));

    // A reload confirms it landed on disk.
    let rc = store.config().await.expect("config reload");
    assert_eq!(rc.commit_user_name.as_deref(), Some("Alice"));
    assert_eq!(rc.commit_user_email.as_deref(), Some("alice@example.com"));

    // Blank/None clears the field → reverts to the default (auto-update).
    let rc = store
        .set_commit_identity(Some("   ".to_string()), None)
        .await
        .expect("set_commit_identity clear");
    assert_eq!(rc.commit_user_name, None);
    assert_eq!(rc.commit_user_email, None);
}

/// `set_commit_identity` rejects characters that corrupt a commit's
/// `Name <email>` line (newlines, `<`, `>`, control bytes) and persists
/// nothing on rejection.
#[tokio::test]
async fn set_commit_identity_rejects_invalid_characters() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo(vec![], &recipient);
    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    // A newline corrupts the author line.
    let err = store
        .set_commit_identity(Some("Alice\nMallory".to_string()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code, "CONFIG_ERROR");
    // `<` / `>` break the `Name <email>` envelope.
    let err = store
        .set_commit_identity(None, Some("a@b> <evil@x".to_string()))
        .await
        .unwrap_err();
    assert_eq!(err.code, "CONFIG_ERROR");

    // Nothing persisted — still the default (None).
    let rc = store.config().await.expect("config");
    assert_eq!(rc.commit_user_name, None);
    assert_eq!(rc.commit_user_email, None);
}

/// Get the same entry twice — identity loading/zeroize must not break subsequent calls.
#[tokio::test]
async fn store_facade_get_same_entry_twice() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) =
        create_test_git_repo(vec![("test.age", b"my-password\nnotes")], &recipient);

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    let secret1 = store.get("test").await.expect("first get should succeed");
    assert_eq!(secret1.password(), "my-password");

    let secret2 = store.get("test").await.expect("second get should succeed");
    assert_eq!(secret2.password(), "my-password");
}

/// Reconfigure replaces the old configuration.
#[tokio::test]
async fn store_facade_reconfigure() {
    let (identity1, recipient1) = generate_test_keypair();
    let (bare_dir1, _clone_dir1) =
        create_test_git_repo(vec![("first.age", b"first-password")], &recipient1);

    let (identity2, recipient2) = generate_test_keypair();
    let (bare_dir2, _clone_dir2) =
        create_test_git_repo(vec![("second.age", b"second-password")], &recipient2);

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);

    // Initial configuration
    store
        .configure(
            bare_dir1.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity1,
            None,
        )
        .await
        .expect("first configure should succeed");
    let entries1 = store.list().await.expect("list should succeed");
    assert!(entries1.iter().any(|e| e.name == "first"));

    // Reconfigure with different repo and identity
    store
        .configure(
            bare_dir2.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity2,
            None,
        )
        .await
        .expect("reconfigure should succeed");
    let entries2 = store
        .list()
        .await
        .expect("list after reconfigure should succeed");
    assert!(
        entries2.iter().any(|e| e.name == "second"),
        "should see entries from new repo"
    );
    assert!(
        !entries2.iter().any(|e| e.name == "first"),
        "should NOT see entries from old repo"
    );

    // Get from the new repo should work
    let secret = store
        .get("second")
        .await
        .expect("get from new repo should succeed");
    assert_eq!(secret.password(), "second-password");
}

/// Configure with an invalid identity (missing prefix) should fail.
#[tokio::test]
async fn store_facade_invalid_identity() {
    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);

    let result = store
        .configure(
            "https://example.com/repo.git",
            None,
            None,
            None,
            "not-a-valid-identity",
            None,
        )
        .await;
    assert!(
        result.is_err(),
        "configure with invalid identity should fail"
    );
    let err = result.unwrap_err();
    assert_eq!(err.code, "INVALID_IDENTITY");
}

/// Get a nonexistent entry should return EntryNotFound.
#[tokio::test]
async fn store_facade_get_nonexistent_entry() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) =
        create_test_git_repo(vec![("exists.age", b"password")], &recipient);

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    let result = store.get("does-not-exist").await;
    assert!(result.is_err(), "get nonexistent entry should fail");
    assert_eq!(result.unwrap_err().code, "ENTRY_NOT_FOUND");
}

/// Get with path traversal should be rejected.
#[tokio::test]
async fn store_facade_get_path_traversal() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo(vec![("real.age", b"password")], &recipient);

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    let result = store.get("../../../etc/passwd").await;
    assert!(result.is_err(), "path traversal should be rejected");
}

/// List on a store with no .age files should return empty.
#[tokio::test]
async fn store_facade_list_empty_store() {
    let (identity, recipient) = generate_test_keypair();
    // Git repo with a non-.age file
    let (bare_dir, _clone_dir) =
        create_test_git_repo(vec![("readme.txt", b"not a password")], &recipient);

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    let entries = store.list().await.expect("list should succeed");
    assert!(entries.is_empty(), "empty store should return no entries");
}

/// Sync when there are no new commits returns unchanged.
#[tokio::test]
async fn store_facade_sync_no_changes() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo(vec![("only.age", b"pw")], &recipient);

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    let result = expect_fast_forwarded(store.sync().await.expect("sync should succeed"));
    assert!(
        !result.changed,
        "sync with no changes should report unchanged"
    );
}

/// Sync pulls new commits, then list shows the new entries.
#[tokio::test]
async fn store_facade_sync_then_list_updated() {
    let (identity, recipient) = generate_test_keypair();
    let (bare_dir, _clone_dir) = create_test_git_repo(vec![("initial.age", b"first")], &recipient);

    let config_dir = tempfile::tempdir().expect("failed to create config dir");
    let store = Store::new(config_dir.path().to_path_buf(), None);
    store
        .configure(
            bare_dir.path().to_str().expect("valid utf-8"),
            None,
            None,
            None,
            &identity,
            None,
        )
        .await
        .expect("configure should succeed");

    // Initially only one entry
    let entries = store.list().await.expect("list should succeed");
    assert_eq!(entries.len(), 1);

    // Add a new commit to bare (upstream)
    add_commit_to_bare(
        bare_dir.path(),
        vec![("new_entry.age", b"second")],
        &recipient,
        "add new entry",
    );

    // Sync should pick up the change
    let sync_result = expect_fast_forwarded(store.sync().await.expect("sync should succeed"));
    assert!(sync_result.changed, "sync should detect upstream changes");

    // List should now show 2 entries
    let entries = store.list().await.expect("list after sync should succeed");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|e| e.name == "initial"));
    assert!(entries.iter().any(|e| e.name == "new_entry"));

    // Get the new entry
    let secret = store
        .get("new_entry")
        .await
        .expect("get new entry should succeed");
    assert_eq!(secret.password(), "second");
}
