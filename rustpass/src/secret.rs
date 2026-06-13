// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::fmt;

use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// A decrypted secret — aligned with `gopass.Secret`.
///
/// First line = password, remainder = body (key-value pairs + freeform notes).
/// All fields use `Zeroizing<String>` so content is wiped on drop.
pub struct Secret {
    password: Zeroizing<String>,
    body: Zeroizing<String>,
}

/// Custom `Debug` that redacts all fields — prevents accidental log leakage.
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("password", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .finish()
    }
}

impl Secret {
    /// Returns the password (first line of the secret).
    #[must_use]
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Returns the body (all content after the first line).
    ///
    /// In gopass AKV format, this typically contains `key: value` metadata
    /// lines followed by optional freeform notes.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Parse decrypted bytes into a `Secret`.
    ///
    /// First line becomes the password, everything after the first newline
    /// becomes the body. Trailing whitespace is stripped.
    ///
    /// # Errors
    ///
    /// Returns an error if the content is empty or contains only whitespace.
    pub fn parse(content: &[u8]) -> Result<Self, Error> {
        let text = String::from_utf8_lossy(content);
        let text = text.trim_end();

        if text.is_empty() {
            return Err(Error::new(
                ErrorCode::DecryptFailed,
                "Decrypted file is empty",
            ));
        }

        // Normalize CRLF to LF for consistent parsing
        let normalized = text.replace("\r\n", "\n");
        let normalized = normalized.trim_end();

        let (password, body) = if let Some(newline_pos) = normalized.find('\n') {
            (
                Zeroizing::new(normalized[..newline_pos].to_string()),
                Zeroizing::new(normalized[newline_pos + 1..].to_string()),
            )
        } else {
            (
                Zeroizing::new(normalized.to_string()),
                Zeroizing::new(String::new()),
            )
        };

        Ok(Self { password, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_password_only() {
        let secret = Secret::parse(b"hunter2").unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn parse_password_and_body() {
        let content = b"hunter2\nusername: alice\nurl: example.com";
        let secret = Secret::parse(content).unwrap();
        assert_eq!(secret.password(), "hunter2");
        assert!(secret.body().contains("username: alice"));
        assert!(secret.body().contains("url: example.com"));
    }

    #[test]
    fn parse_empty_content_errors() {
        let result = Secret::parse(b"");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "DECRYPT_FAILED");
    }

    #[test]
    fn parse_whitespace_only_errors() {
        let result = Secret::parse(b"  \n  \n");
        assert!(result.is_err());
    }

    #[test]
    fn parse_trailing_newlines_stripped() {
        let secret = Secret::parse(b"pw\nnotes\n").unwrap();
        assert_eq!(secret.password(), "pw");
        assert_eq!(secret.body(), "notes");
    }

    #[test]
    fn parse_crlf_line_endings() {
        let secret = Secret::parse(b"pw\r\nnotes\r\nmore notes\r\n").unwrap();
        assert_eq!(secret.password(), "pw");
        assert_eq!(secret.body(), "notes\nmore notes");
    }

    #[test]
    fn debug_redacts_password() {
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

    #[test]
    fn parse_unicode_content() {
        let secret = Secret::parse("密码123\n用户: 张三\n网址: example.com".as_bytes()).unwrap();
        assert_eq!(secret.password(), "密码123");
        assert!(secret.body().contains("用户: 张三"));
    }

    #[test]
    fn parse_multiline_body() {
        let secret = Secret::parse(b"pw\nline1\nline2\nline3").unwrap();
        assert_eq!(secret.password(), "pw");
        assert_eq!(secret.body(), "line1\nline2\nline3");
    }
}
