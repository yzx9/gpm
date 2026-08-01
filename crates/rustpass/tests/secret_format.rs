// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use rustpass::Secret;

/// Standard gopass format: password only, no body.
#[test]
fn secret_password_only() {
    let secret = Secret::parse(b"hunter2").unwrap();
    assert_eq!(secret.password(), "hunter2");
    assert_eq!(secret.body(), "");
}

/// Standard gopass format: password + key-value metadata body.
#[test]
fn secret_password_and_body() {
    let secret = Secret::parse(b"hunter2\nusername: alice\nurl: example.com").unwrap();
    assert_eq!(secret.password(), "hunter2");
    assert!(secret.body().contains("username: alice"));
    assert!(secret.body().contains("url: example.com"));
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

/// Unicode content in password and body.
#[test]
fn secret_unicode_content() {
    let secret = Secret::parse("密码123\n用户: 张三\n网址: example.com".as_bytes()).unwrap();
    assert_eq!(secret.password(), "密码123");
    assert!(secret.body().contains("用户: 张三"));
    assert!(secret.body().contains("网址: example.com"));
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
    assert_eq!(secret.body(), "user: alice");
}
