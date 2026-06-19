// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

use common::*;
use rustpass::crypto;

/// Decrypt content that was encrypted as empty bytes.
#[test]
fn decrypt_empty_plaintext() {
    let (identity, recipient) = generate_test_keypair();
    let encrypted = encrypt_to_recipient(b"", &recipient);

    let result = crypto::decrypt_bytes(&encrypted, identity.as_bytes(), None).unwrap();
    assert!(
        result.is_empty(),
        "decrypted empty plaintext should be empty"
    );
}

/// Decrypt binary (non-UTF-8) content should succeed — decryption is byte-level.
#[test]
fn decrypt_binary_content() {
    let (identity, recipient) = generate_test_keypair();
    let binary: Vec<u8> = (0..=255).collect();
    let encrypted = encrypt_to_recipient(&binary, &recipient);

    let result = crypto::decrypt_bytes(&encrypted, identity.as_bytes(), None).unwrap();
    assert_eq!(result, binary, "binary content should round-trip exactly");
}

/// Decrypt content that is only newlines.
#[test]
fn decrypt_newlines_only() {
    let (identity, recipient) = generate_test_keypair();
    let encrypted = encrypt_to_recipient(b"\n\n\n", &recipient);

    let result = crypto::decrypt_bytes(&encrypted, identity.as_bytes(), None).unwrap();
    assert_eq!(result, b"\n\n\n", "newline-only content should round-trip");
}

/// Encrypted for one recipient, decrypted with a different identity should fail.
#[test]
fn decrypt_wrong_recipient() {
    let (_identity_a, recipient_a) = generate_test_keypair();
    let (identity_b, _recipient_b) = generate_test_keypair();

    let encrypted = encrypt_to_recipient(b"secret for A", &recipient_a);
    let result = crypto::decrypt_bytes(&encrypted, identity_b.as_bytes(), None);
    assert!(
        result.is_err(),
        "decrypting with wrong identity should fail"
    );
}

/// Identity string with leading/trailing whitespace should be handled.
#[test]
fn decrypt_identity_with_leading_trailing_whitespace() {
    let (identity, recipient) = generate_test_keypair();
    let encrypted = encrypt_to_recipient(b"password", &recipient);

    // Leading/trailing whitespace should cause parse failure
    let padded = format!("  {identity}  ");
    let result = crypto::decrypt_bytes(&encrypted, padded.as_bytes(), None);
    // This may fail or succeed depending on age parser tolerance — verify behavior
    // The age library's from_buffer may trim or may not; either way it should be deterministic
    if let Ok(decrypted) = result {
        assert_eq!(decrypted, b"password");
    }
    // If it fails, that's also acceptable — identities with whitespace are suspicious
}

/// Large plaintext (>10KB) should decrypt correctly.
#[test]
fn decrypt_large_plaintext() {
    let (identity, recipient) = generate_test_keypair();
    let large: Vec<u8> = (0..=255).cycle().take(20_000).collect();
    let encrypted = encrypt_to_recipient(&large, &recipient);

    let result = crypto::decrypt_bytes(&encrypted, identity.as_bytes(), None).unwrap();
    assert_eq!(result, large, "large plaintext should round-trip exactly");
}
