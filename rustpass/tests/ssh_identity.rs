// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! End-to-end coverage for SSH-key identities — the one identity shape with no
//! prior integration test. Exercises the SSH-identity cache (unlock decrypts the
//! key once; `get` skips the bcrypt KDF) and the write path (`set` derives our
//! recipient from the cached unencrypted PEM with `passphrase = None`).

mod common;

use common::*;
use rustpass::WriteOutcome;
use rustpass::store::Store;

/// Round-trip with an encrypted ed25519 SSH identity:
/// configure → unlock (caches the decrypted key) → get twice (cache hit, no
/// re-KDF) → set a new secret (recipient derived from the cached PEM,
/// passphrase=None) → get it back → lock.
#[tokio::test]
async fn ssh_identity_unlock_get_set_round_trip() {
    let passphrase = "ssh-test-passphrase";
    let (private_key, public_key) = generate_ssh_test_keypair(passphrase);

    // Seed a git repo with one entry encrypted to the SSH recipient. The
    // entry is pre-encrypted (committed verbatim); no recipients file is
    // needed — `set` below encrypts to our own key alone.
    let existing_ct = encrypt_to_ssh_recipient(b"my-password\nuser: alice", &public_key);
    let (bare_dir, _clone_dir) = create_test_git_repo_with(
        vec![],
        vec![("existing.age", existing_ct.as_slice())],
        &public_key, // only used to encrypt x25519 entries (none here)
    );

    let config_dir = tempfile::tempdir().expect("config dir");
    let store = Store::new(config_dir.path().to_path_buf());

    // configure validates the SSH identity can derive a recipient — for an
    // encrypted key that needs the passphrase — and saves the key as-is.
    store
        .configure(
            bare_dir.path().to_str().expect("utf-8"),
            None,
            None,
            None,
            &private_key,
            Some(passphrase),
        )
        .await
        .expect("configure should succeed");
    assert!(
        store.is_identity_encrypted().await,
        "an encrypted SSH identity must be encrypted"
    );
    assert!(!store.is_unlocked(), "should start locked");

    // get() before unlock fails: identity encrypted, cache empty.
    let err = store
        .get("existing")
        .await
        .expect_err("get should fail while locked");
    assert_eq!(err.code, "IDENTITY_ENCRYPTED");

    // unlock() decrypts the SSH key once and caches the unencrypted PEM.
    store
        .unlock(passphrase)
        .await
        .expect("unlock with the correct passphrase");
    assert!(store.is_unlocked(), "should be unlocked after unlock()");

    // get() decrypts via the cached unencrypted PEM — age takes the no-KDF
    // Unencrypted path instead of re-deriving the key.
    let secret = store.get("existing").await.expect("get after unlock");
    assert_eq!(secret.password(), "my-password");
    // A second get exercises the cached path (no re-KDF, same result).
    let again = store.get("existing").await.expect("second get");
    assert_eq!(again.password(), "my-password");

    // set() derives our recipient from the cached unencrypted PEM with
    // passphrase = None, encrypts, commits, and pushes.
    let outcome = store
        .set("new-secret", b"new-password\n")
        .await
        .expect("set should succeed");
    assert!(
        matches!(outcome, WriteOutcome::Written(_)),
        "expected the write to land, got {outcome:?}"
    );

    // Read back what we just wrote — proves the write encrypted to our key.
    let created = store.get("new-secret").await.expect("get new-secret");
    assert_eq!(created.password(), "new-password");

    // lock() drops the cache; get() fails again (identity encrypted, cache empty).
    store.lock();
    assert!(!store.is_unlocked(), "should be locked after lock()");
    let err = store
        .get("existing")
        .await
        .expect_err("get should fail after lock");
    assert_eq!(err.code, "IDENTITY_ENCRYPTED");
}
