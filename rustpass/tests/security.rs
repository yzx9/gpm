// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

use common::*;
use rustpass::crypto;
use rustpass::error::{Error, ErrorCode};
use rustpass::secret::Secret;
use rustpass::store;

// -----------------------------------------------------------------------
// Path traversal tests
// -----------------------------------------------------------------------

#[test]
fn path_traversal_dotdot() {
    let dir = tempfile::tempdir().unwrap();

    let result = store::resolve_entry_path(dir.path(), "../../../etc/passwd");
    assert!(result.is_err(), "expected Err for dotdot traversal, got Ok");
    let err = result.unwrap_err();
    assert_eq!(
        err.code, "ENTRY_NOT_FOUND",
        "expected ENTRY_NOT_FOUND, got: {err}"
    );
}

#[test]
fn path_traversal_encoded_dots() {
    let dir = tempfile::tempdir().unwrap();

    let result = store::resolve_entry_path(dir.path(), "%2e%2e%2f..%2fetc%2fpasswd");
    assert!(
        result.is_err(),
        "expected Err for encoded-dot traversal, got Ok"
    );
    let err = result.unwrap_err();
    assert_eq!(
        err.code, "ENTRY_NOT_FOUND",
        "expected ENTRY_NOT_FOUND, got: {err}"
    );
}

#[test]
#[cfg(unix)]
fn path_traversal_symlink_escape() {
    use std::os::unix::fs::symlink;

    let external_dir = tempfile::tempdir().unwrap();
    let external_file = external_dir.path().join("target.txt");
    std::fs::write(&external_file, b"external-secret").unwrap();

    let repo_dir = tempfile::tempdir().unwrap();
    let link_path = repo_dir.path().join("escape.age");
    symlink(&external_file, &link_path).unwrap();

    let result = store::resolve_entry_path(repo_dir.path(), "escape.age");
    assert!(result.is_err(), "expected Err for symlink escape, got Ok");
    let err = result.unwrap_err();
    assert_eq!(
        err.code, "ENTRY_NOT_FOUND",
        "expected ENTRY_NOT_FOUND, got: {err}"
    );
    assert!(
        err.message.contains("outside repository"),
        "expected 'outside repository' in message, got: {}",
        err.message
    );
}

#[test]
fn path_traversal_null_byte() {
    let dir = tempfile::tempdir().unwrap();

    let entry_with_null = "foo.age\0../bar";
    let result = store::resolve_entry_path(dir.path(), entry_with_null);
    assert!(result.is_err(), "expected Err for null-byte path, got Ok");
    let err = result.unwrap_err();
    assert_eq!(
        err.code, "ENTRY_NOT_FOUND",
        "expected ENTRY_NOT_FOUND, got: {err}"
    );
}

#[test]
fn path_traversal_mixed_separators() {
    let dir = tempfile::tempdir().unwrap();

    let result = store::resolve_entry_path(dir.path(), "foo\\..\\..\\bar.age");
    assert!(
        result.is_err(),
        "expected Err for mixed-separator traversal, got Ok"
    );
    let err = result.unwrap_err();
    assert_eq!(
        err.code, "ENTRY_NOT_FOUND",
        "expected ENTRY_NOT_FOUND, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Error message sanitization tests
// -----------------------------------------------------------------------

#[test]
fn no_identity_in_decrypt_error() {
    let invalid_identity = "not-a-key";
    let result = crypto::decrypt_bytes(b"some data", invalid_identity.as_bytes(), None);
    assert!(result.is_err(), "expected Err for invalid identity, got Ok");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        !err_msg.contains(invalid_identity),
        "error message must not contain the identity string: {err_msg}"
    );
}

#[test]
fn no_plaintext_in_decrypt_error() {
    let (_identity, recipient) = generate_test_keypair();
    let (wrong_identity, _wrong_recipient) = generate_test_keypair();

    let plaintext = "my-real-secret-password";
    let encrypted = encrypt_to_recipient(plaintext.as_bytes(), &recipient);

    let result = crypto::decrypt_bytes(&encrypted, wrong_identity.as_bytes(), None);
    assert!(result.is_err(), "expected Err with wrong identity, got Ok");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        !err_msg.contains(plaintext),
        "error message must not contain the plaintext: {err_msg}"
    );
}

#[test]
fn no_secrets_in_resolve_error() {
    let dir = tempfile::tempdir().unwrap();

    let entry_name = "nonexistent/secret-entry.age";
    let result = store::resolve_entry_path(dir.path(), entry_name);
    assert!(result.is_err(), "expected Err for missing entry, got Ok");
    let err = result.unwrap_err();

    assert_eq!(err.code, "ENTRY_NOT_FOUND");
    assert!(
        err.message.contains(entry_name),
        "error message should contain the entry name: {}",
        err.message
    );
}

#[test]
fn error_serialization_safe() {
    let err = Error::new(ErrorCode::DecryptFailed, "safe description");
    let json = serde_json::to_string(&err).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let obj = parsed.as_object().unwrap_or_else(|| {
        panic!("expected JSON object, got: {json}");
    });

    assert_eq!(obj.len(), 2, "expected exactly 2 fields, got: {json}");
    assert!(obj.contains_key("code"), "expected 'code' field in: {json}");
    assert!(
        obj.contains_key("message"),
        "expected 'message' field in: {json}"
    );

    let code = obj.get("code").and_then(|v| v.as_str()).unwrap_or_else(|| {
        panic!("expected string 'code' value in: {json}");
    });
    let message = obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("expected string 'message' value in: {json}"));

    assert_eq!(code, "DECRYPT_FAILED");
    assert_eq!(message, "safe description");
}

// -----------------------------------------------------------------------
// Debug redaction tests
// -----------------------------------------------------------------------

#[test]
fn secret_debug_redacts() {
    let secret = Secret::parse(b"hunter2\nnotes").unwrap();
    let debug_output = format!("{secret:?}");

    assert!(
        debug_output.contains("[REDACTED]"),
        "Debug output should contain [REDACTED], got: {debug_output}"
    );
    assert!(
        !debug_output.contains("hunter2"),
        "Debug output must not contain the actual password, got: {debug_output}"
    );
}

// -----------------------------------------------------------------------
// Identity validation tests
// -----------------------------------------------------------------------

#[test]
fn identity_missing_prefix_rejected() {
    let result = crypto::decrypt_bytes(b"some data", b"not-a-key", None);
    assert!(
        result.is_err(),
        "expected Err for identity missing prefix, got Ok"
    );
    let err = result.unwrap_err();
    assert_eq!(
        err.code, "INVALID_IDENTITY",
        "expected INVALID_IDENTITY, got: {err}"
    );
}

#[test]
fn identity_only_prefix_rejected() {
    let result = crypto::decrypt_bytes(b"some data", b"AGE-SECRET-KEY-", None);
    assert!(
        result.is_err(),
        "expected Err for prefix-only identity, got Ok"
    );
    let err = result.unwrap_err();
    assert_eq!(
        err.code, "INVALID_IDENTITY",
        "expected INVALID_IDENTITY, got: {err}"
    );
}
