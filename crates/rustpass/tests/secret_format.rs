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

use rustpass::Secret;

/// Standard gopass format: password only, no body.
#[test]
fn secret_password_only() {
    let secret = Secret::parse(b"hunter2").unwrap();
    assert_eq!(secret.password(), "hunter2");
    assert_eq!(secret.body(), "");
    assert!(secret.attributes().is_empty());
}

/// Standard gopass format: password + key-value metadata (now structured attrs).
#[test]
fn secret_password_and_body() {
    let secret = Secret::parse(b"hunter2\nusername: alice\nurl: example.com").unwrap();
    assert_eq!(secret.password(), "hunter2");
    assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
    assert_eq!(secret.get("url"), Some(b"example.com".as_slice()));
    // No free-text line → empty body.
    assert_eq!(secret.body(), "");
}

/// Multi-line body content.
#[test]
fn secret_password_and_multiline_body() {
    let secret = Secret::parse(b"pw\nline1\nline2\nline3").unwrap();
    assert_eq!(secret.password(), "pw");
    assert_eq!(secret.body(), "line1\nline2\nline3");
}

/// Windows-style CRLF line endings should be normalized.
#[test]
fn secret_crlf_line_endings() {
    let secret = Secret::parse(b"pw\r\nnotes\r\nmore notes\r\n").unwrap();
    assert_eq!(secret.password(), "pw");
    assert_eq!(secret.body(), "notes\nmore notes");
}

/// Unicode content in password and attributes.
#[test]
fn secret_unicode_content() {
    let secret = Secret::parse("密码123\n用户: 张三\n网址: example.com".as_bytes()).unwrap();
    assert_eq!(secret.password(), "密码123");
    assert_eq!(secret.attribute_str("用户"), Some("张三"));
    assert_eq!(secret.attribute_str("网址"), Some("example.com"));
}

/// Body containing only whitespace after the password line.
#[test]
fn secret_only_whitespace_body() {
    let secret = Secret::parse(b"pw\n   \n  ").unwrap();
    assert_eq!(secret.password(), "pw");
    // After trim_end, trailing whitespace is removed but inner whitespace remains
    // The body is "   " (after the first newline, trimmed at the end)
}

/// Multiple trailing newlines should be stripped.
#[test]
fn secret_trailing_newlines_stripped() {
    let secret = Secret::parse(b"pw\nnotes\n\n\n").unwrap();
    assert_eq!(secret.password(), "pw");
    assert_eq!(secret.body(), "notes");
}

/// Large body (>1KB) should be handled.
#[test]
fn secret_large_body() {
    let long_body: String = "x".repeat(2048);
    let content = format!("password\n{long_body}");
    let secret = Secret::parse(content.as_bytes()).unwrap();
    assert_eq!(secret.password(), "password");
    assert_eq!(secret.body().len(), 2048);
}

/// Password that looks like a gopass reference (gopass:// protocol).
#[test]
fn secret_with_gopass_reference() {
    let secret = Secret::parse(b"gopass://other/entry\nuser: alice").unwrap();
    assert_eq!(secret.password(), "gopass://other/entry");
    assert_eq!(secret.get("user"), Some(b"alice".as_slice()));
    assert_eq!(secret.body(), "");
}

// ---- deprecated GOPASS-SECRET-1.0 format (read-only compat) ----
//
// gpm never writes this format; these tests cover reading the deprecated
// single-part header-block format gopass wrote mid-2020–v1.13 and still reads.
// Phase 2b: legacy headers become structured attributes (lowercased keys), not
// flattened body text.

/// Legacy MIME secret: password lifted from the `Password:` header, remaining
/// headers become structured attributes, free text preserved after the blank line.
#[test]
fn secret_legacy_basic() {
    let secret =
        Secret::parse(b"GOPASS-SECRET-1.0\nPassword: hunter2\nusername: alice\n\nfree text")
            .unwrap();
    assert_eq!(secret.password(), "hunter2");
    assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
    assert_eq!(secret.body(), "free text");
}

/// Multiple non-Password headers become attributes in source order.
#[test]
fn secret_legacy_multiple_attributes() {
    let secret =
        Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nusername: alice\nurl: example.com\n")
            .unwrap();
    assert_eq!(secret.password(), "p");
    let pairs: Vec<(&[u8], &[u8])> = secret
        .attributes()
        .iter()
        .map(|a| (a.key(), a.value()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (b"username".as_slice(), b"alice".as_slice()),
            (b"url".as_slice(), b"example.com".as_slice()),
        ]
    );
    assert_eq!(secret.body(), "");
}

/// No `Password:` header → empty password, other headers still become attributes.
#[test]
fn secret_legacy_no_password_header() {
    let secret = Secret::parse(b"GOPASS-SECRET-1.0\nusername: alice\n\nnotes").unwrap();
    assert_eq!(secret.password(), "");
    assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
    assert_eq!(secret.body(), "notes");
}

/// RFC-822 folded continuation lines are unfolded into the attribute value.
#[test]
fn secret_legacy_folded_continuation() {
    let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nnote: a\n  b\n\nbody").unwrap();
    assert_eq!(secret.password(), "p");
    assert_eq!(secret.attribute_str("note"), Some("a b"));
    assert_eq!(secret.body(), "body");
}

/// Magic-only file (no headers, no body) → empty password and body.
#[test]
fn secret_legacy_magic_only() {
    let secret = Secret::parse(b"GOPASS-SECRET-1.0").unwrap();
    assert_eq!(secret.password(), "");
    assert_eq!(secret.body(), "");
}

/// Headers with no blank-line terminator: EOF ends the header block, body empty.
#[test]
fn secret_legacy_no_body_just_headers() {
    let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nusername: alice").unwrap();
    assert_eq!(secret.password(), "p");
    assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
    assert_eq!(secret.body(), "");
}

/// CRLF line endings are normalized before the legacy parse.
#[test]
fn secret_legacy_crlf_line_endings() {
    let secret =
        Secret::parse(b"GOPASS-SECRET-1.0\r\nPassword: hunter2\r\nusername: alice\r\n\r\nbody\r\n")
            .unwrap();
    assert_eq!(secret.password(), "hunter2");
    assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
    assert_eq!(secret.body(), "body");
}

/// The `Password:` key is matched case-insensitively (lower / UPPER / Mixed).
#[test]
fn secret_legacy_password_case_variants() {
    for key in ["password", "PASSWORD", "PaSsWoRd"] {
        let input = format!("GOPASS-SECRET-1.0\n{key}: secret-value\n");
        let s = Secret::parse(input.as_bytes()).unwrap();
        assert_eq!(s.password(), "secret-value", "key {key}");
    }
}

/// A header value containing `:` (e.g. a URL) splits on the first colon only.
#[test]
fn secret_legacy_url_value_with_colon() {
    let secret =
        Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nurl: https://example.com:8080/path")
            .unwrap();
    assert_eq!(secret.password(), "p");
    assert_eq!(
        secret.get("url"),
        Some(b"https://example.com:8080/path".as_slice())
    );
    assert_eq!(secret.body(), "");
}

/// A malformed header block (no-colon line / orphan fold) falls back to the
/// modern text parse, matching gopass's `ParseAKV(in)`: password = the magic.
#[test]
fn secret_legacy_lenient_no_colon_becomes_modern_fallback() {
    let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword: p\nno colon here\nmore").unwrap();
    assert_eq!(secret.password(), "GOPASS-SECRET-1.0");
    // Modern fallback preserves key case, so the attribute is key "Password".
    assert_eq!(secret.get_ci("password"), Some(b"p".as_slice()));
    assert_eq!(secret.body(), "no colon here\nmore");
}

#[test]
fn secret_legacy_orphan_fold_becomes_modern_fallback() {
    let secret = Secret::parse(b"GOPASS-SECRET-1.0\n  orphan\nmore").unwrap();
    assert_eq!(secret.password(), "GOPASS-SECRET-1.0");
    assert_eq!(secret.body(), "  orphan\nmore");
}

/// A modern secret whose first line is a real password is NOT misdetected as
/// legacy.
#[test]
fn secret_legacy_modern_password_not_misdetected() {
    let secret = Secret::parse(b"hunter2\nusername: alice").unwrap();
    assert_eq!(secret.password(), "hunter2");
    assert_eq!(secret.get("username"), Some(b"alice".as_slice()));
    assert_eq!(secret.body(), "");
}

/// gopass parity: a legacy parse reassembled via `to_bytes` re-parses to identical
/// fields (password, attributes, body) — the round-trip-stability invariant.
#[test]
fn secret_legacy_normalizes_to_modern_on_reparse() {
    let s1 = Secret::parse(b"GOPASS-SECRET-1.0\nPassword: hunter2\nusername: alice\n\nfree text")
        .unwrap();
    let modern = s1.to_bytes();
    let s2 = Secret::parse(&modern).unwrap();
    assert_eq!(s1.password(), s2.password());
    assert_eq!(s1.body(), s2.body());
    assert_eq!(s1.attributes(), s2.attributes());
    // And the round-trip is stable (idempotent).
    assert_eq!(s2.to_bytes().as_slice(), modern.as_slice());
}

/// gopass parity: an empty-value `Password:` header is not extracted and
/// stays as a structured `password` attribute (empty value).
#[test]
fn secret_legacy_empty_password_value_kept_in_body() {
    let secret = Secret::parse(b"GOPASS-SECRET-1.0\nPassword:\nFoo: Bar").unwrap();
    assert_eq!(secret.password(), "");
    assert_eq!(secret.get("password"), Some(b"".as_slice()));
    assert_eq!(secret.get("foo"), Some(b"Bar".as_slice()));
    assert_eq!(secret.body(), "");
}

/// End-to-end TOTP interaction: a legacy `Totp:` header is lowercased into a
/// `totp` attribute, which gpm's case-sensitive TOTP detector then finds. Locks
/// the lowercasing→detection chain against the real consumer.
#[test]
fn secret_legacy_totp_header_detected_after_render() {
    // 20-byte / 160-bit base32 secret accepted by totp-rs's ≥128-bit floor.
    const SEED: &str = "KRSXG5CTMVRXEZLUKN2XAZLSKNSWG4TFOQ";
    let secret =
        Secret::parse(format!("GOPASS-SECRET-1.0\nPassword: p\nTotp: {SEED}\n").as_bytes())
            .unwrap();
    // The mixed-case `Totp:` header must be lowercased into a `totp` attribute.
    assert_eq!(secret.attribute_str("totp"), Some(SEED));
    assert!(rustpass::totp::has_totp(&secret));
    assert!(rustpass::totp::extract(&secret).unwrap().is_some());
}
