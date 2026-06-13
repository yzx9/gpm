// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

mod tests {
    use super::common::*;
    use rustpass::store::Store;

    /// Configure with encrypted identity → unlock → get → lock.
    #[tokio::test]
    async fn encrypted_identity_full_lifecycle() {
        let (identity, recipient) = generate_test_keypair();
        let passphrase = "correct-horse-battery-staple";

        let (bare_dir, _clone_dir) = create_test_git_repo(
            vec![("secret.age", b"my-password\nuser: alice")],
            &recipient,
        );

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

        // Configure with encrypted identity
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

        // Set passphrase (encrypt the identity)
        store
            .set_passphrase(passphrase)
            .await
            .expect("set_passphrase should succeed");
        assert!(
            store.is_identity_encrypted().await,
            "identity should be encrypted"
        );

        // Not unlocked yet
        assert!(!store.is_unlocked(), "should not be unlocked yet");

        // Getting without unlock should fail
        let err = store
            .get("secret")
            .await
            .expect_err("get should fail when locked");
        assert_eq!(err.code, "IDENTITY_ENCRYPTED");

        // Unlock with wrong passphrase should fail
        let err = store
            .unlock("wrong-passphrase")
            .await
            .expect_err("unlock with wrong passphrase should fail");
        assert_eq!(err.code, "WRONG_PASSPHRASE");

        // Unlock with correct passphrase
        store
            .unlock(passphrase)
            .await
            .expect("unlock should succeed");
        assert!(store.is_unlocked(), "should be unlocked");

        // Now get should work
        let secret = store
            .get("secret")
            .await
            .expect("get should succeed after unlock");
        assert_eq!(secret.password(), "my-password");

        // Lock
        store.lock();
        assert!(!store.is_unlocked(), "should not be unlocked after lock");

        // Getting after lock should fail
        let err = store
            .get("secret")
            .await
            .expect_err("get should fail after lock");
        assert_eq!(err.code, "IDENTITY_ENCRYPTED");
    }

    /// Two-step setup with passphrase, then unlock.
    #[tokio::test]
    async fn two_step_setup_with_passphrase() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) = create_test_git_repo(vec![("test.age", b"hello")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

        // Step 1: clone
        store
            .clone_only(
                bare_dir.path().to_str().expect("valid utf-8"),
                None,
                None,
                None,
            )
            .await
            .expect("clone_only should succeed");

        assert!(store.is_repo_ready());
        assert!(!store.is_configured());

        // Step 2: save identity with passphrase
        store
            .save_identity(&identity, Some("mypass"), None)
            .await
            .expect("save_identity should succeed");

        assert!(store.is_configured());
        assert!(store.is_identity_encrypted().await);
        assert!(!store.is_unlocked());

        // Unlock and decrypt
        store.unlock("mypass").await.expect("unlock should succeed");
        let secret = store.get("test").await.expect("get should work");
        assert_eq!(secret.password(), "hello");
    }

    /// Change passphrase: unlock with old → change → unlock with new.
    #[tokio::test]
    async fn change_passphrase_flow() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) =
            create_test_git_repo(vec![("data.age", b"s3cret")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

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

        // Encrypt
        store.set_passphrase("old-pass").await.unwrap();
        assert!(store.is_identity_encrypted().await);

        // Unlock with old passphrase
        store.unlock("old-pass").await.unwrap();
        let s = store.get("data").await.unwrap();
        assert_eq!(s.password(), "s3cret");

        // Change passphrase
        store
            .change_passphrase("old-pass", "new-pass")
            .await
            .unwrap();

        // Cache cleared by change_passphrase
        assert!(!store.is_unlocked());

        // Old passphrase no longer works
        let err = store.unlock("old-pass").await.unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");

        // New passphrase works
        store.unlock("new-pass").await.unwrap();
        let s = store.get("data").await.unwrap();
        assert_eq!(s.password(), "s3cret");
    }

    /// Reset clears the cache.
    #[tokio::test]
    async fn reset_clears_cache() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) = create_test_git_repo(vec![("x.age", b"pw")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

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
            .unwrap();

        store.set_passphrase("pass").await.unwrap();
        store.unlock("pass").await.unwrap();
        assert!(store.is_unlocked());

        // Reset clears everything including cache
        store.reset().await.unwrap();
        assert!(!store.is_unlocked());
        assert!(!store.is_configured());
    }

    /// Unlock is idempotent (calling twice is fine).
    #[tokio::test]
    async fn unlock_is_idempotent() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) = create_test_git_repo(vec![("a.age", b"x")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

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
            .unwrap();

        store.set_passphrase("pass").await.unwrap();

        // Unlock twice
        store.unlock("pass").await.unwrap();
        assert!(store.is_unlocked());
        store.unlock("pass").await.unwrap();
        assert!(store.is_unlocked());

        // Should still work
        let s = store.get("a").await.unwrap();
        assert_eq!(s.password(), "x");
    }

    /// Lock is idempotent when not unlocked.
    #[tokio::test]
    async fn lock_is_idempotent() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) = create_test_git_repo(vec![("b.age", b"y")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

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
            .unwrap();

        store.set_passphrase("pass").await.unwrap();

        // Lock without unlock
        store.lock();
        store.lock();
        assert!(!store.is_unlocked());
    }

    /// set_passphrase rejects when already encrypted.
    #[tokio::test]
    async fn set_passphrase_rejects_already_encrypted() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) = create_test_git_repo(vec![("c.age", b"z")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

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
            .unwrap();

        store.set_passphrase("first").await.unwrap();
        let err = store.set_passphrase("second").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_ENCRYPTED");
    }

    /// change_passphrase rejects when not encrypted.
    #[tokio::test]
    async fn change_passphrase_rejects_not_encrypted() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) = create_test_git_repo(vec![("d.age", b"w")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

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
            .unwrap();

        let err = store.change_passphrase("old", "new").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    /// Plaintext identity works without unlock (no encryption).
    #[tokio::test]
    async fn plaintext_identity_no_unlock_needed() {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) =
            create_test_git_repo(vec![("plain.age", b"plaintext-secret")], &recipient);

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf());

        // Configure without passphrase
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
            .unwrap();

        assert!(!store.is_identity_encrypted().await);
        assert!(!store.is_unlocked());

        // Can still decrypt without unlock
        let secret = store.get("plain").await.unwrap();
        assert_eq!(secret.password(), "plaintext-secret");
    }
}
