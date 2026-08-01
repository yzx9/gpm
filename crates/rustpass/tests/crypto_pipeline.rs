// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Seam tests for the `CryptoBackend` encrypt/decrypt pipeline — pin the
//! ensureOurKeyID contract (gopass: the writer's own key is always among the
//! encryption targets) directly on `AgeBackend`, independent of the `Store`
//! facade and its identity cache.

mod common;

use rustpass::crypto::{AgeBackend, CryptoBackend};
use rustpass::storage::{GitStorage, RepoFiles};

use common::{TEST_RECIPIENTS_FILE, generate_test_keypair};

/// `encrypt` adds the identity's own recipient when the recipients index omits
/// it (ensureOurKeyID), so the writer can always decrypt what it wrote — even
/// against an index that lists only other keys. This is the safety net that
/// keeps a new contributor's local write readable by them when the shared index
/// hasn't been updated with their key yet.
#[tokio::test]
async fn encrypt_ensures_our_recipient_when_absent_from_index() {
    let (our_identity, our_recipient) = generate_test_keypair();
    let (_other_identity, other_recipient) = generate_test_keypair();

    // The index lists ONLY the other recipient — ours is deliberately absent.
    let index = format!("{other_recipient}\n");
    assert!(
        !index.contains(&our_recipient),
        "fixture: our recipient must NOT be in the index"
    );

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(TEST_RECIPIENTS_FILE), index).unwrap();

    let backend = AgeBackend;
    let view = RepoFiles::new(&GitStorage, dir.path());

    let plaintext = b"the-package-password";
    let ciphertext = backend
        .encrypt(plaintext, our_identity.as_bytes(), &view)
        .await
        .expect("encrypt must succeed against a valid recipients index");

    // We were absent from the index, yet we can decrypt — ensureOurKeyID added us.
    let decrypted = backend
        .decrypt(&ciphertext, our_identity.as_bytes())
        .await
        .expect("the writer must always be able to decrypt what it wrote");
    assert_eq!(decrypted.as_slice(), plaintext);
}

/// `list_recipients` reads + parses the backend's recipients index through the
/// view, and returns empty for a genuinely-missing index (uninitialized store).
#[tokio::test]
async fn list_recipients_round_trips_and_treats_missing_as_empty() {
    let backend = AgeBackend;

    // Missing index → empty (uninitialized store).
    let dir = tempfile::tempdir().unwrap();
    let view = RepoFiles::new(&GitStorage, dir.path());
    let got = backend
        .list_recipients(&view)
        .await
        .expect("missing index is an uninitialized store, not an error");
    assert!(got.is_empty(), "missing index must read as empty");

    // Present index → parsed. The round-trip back through encrypt/decrypt above
    // already exercises the parsed-recipients path; here we just confirm count.
    let (_id, recipient) = generate_test_keypair();
    std::fs::write(
        dir.path().join(TEST_RECIPIENTS_FILE),
        format!("{recipient}\n"),
    )
    .unwrap();
    let got = backend
        .list_recipients(&view)
        .await
        .expect("valid index parses");
    assert_eq!(got.len(), 1, "one recipient line → one parsed recipient");
}

/// A non-UTF-8 recipients index is a hard error, not an empty set. Parsing the
/// invalid bytes as empty would `ensureOurKeyID` to only our key and silently
/// drop every other recipient on the next encrypt — exactly the shrink the
/// guard exists to prevent.
#[tokio::test]
async fn list_recipients_rejects_non_utf8_index() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(TEST_RECIPIENTS_FILE),
        b"age1abc\n\xff\xfe\n",
    )
    .unwrap();
    let backend = AgeBackend;
    let view = RepoFiles::new(&GitStorage, dir.path());
    let err = backend
        .list_recipients(&view)
        .await
        .expect_err("non-UTF-8 index must be a hard error");
    assert_eq!(
        err.code, "STORE_ERROR",
        "non-UTF-8 index must not read as empty"
    );
}
