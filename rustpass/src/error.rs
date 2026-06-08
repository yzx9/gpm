// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use serde::Serialize;

/// Machine-readable error codes — all messages are safe (no secrets).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Git clone operation failed.
    CloneFailed,
    /// Fast-forward pull failed (diverged branches).
    PullFfFailed,
    /// Age decryption failed.
    DecryptFailed,
    /// Invalid age identity format.
    InvalidIdentity,
    /// No git repository found.
    NoRepo,
    /// No age identity configured.
    NoIdentity,
    /// Network connectivity error.
    NetworkError,
    /// Requested entry not found in repository.
    EntryNotFound,
    /// Filesystem I/O error.
    IoError,
    /// Configuration read/write error.
    ConfigError,
    /// General store error.
    StoreError,
    /// SSH key was invalid or could not be parsed.
    SshKeyInvalid,
}

/// Safe error type that never contains secret content.
#[derive(Debug, Clone, Serialize)]
pub struct Error {
    /// Machine-readable error code string (e.g. `"CLONE_FAILED"`).
    pub code: String,
    /// Human-readable error message (no secrets).
    pub message: String,
}

impl Error {
    /// Create a new error from a code and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: match code {
                ErrorCode::CloneFailed => "CLONE_FAILED",
                ErrorCode::PullFfFailed => "PULL_FF_FAILED",
                ErrorCode::DecryptFailed => "DECRYPT_FAILED",
                ErrorCode::InvalidIdentity => "INVALID_IDENTITY",
                ErrorCode::NoRepo => "NO_REPO",
                ErrorCode::NoIdentity => "NO_IDENTITY",
                ErrorCode::NetworkError => "NETWORK_ERROR",
                ErrorCode::EntryNotFound => "ENTRY_NOT_FOUND",
                ErrorCode::IoError => "IO_ERROR",
                ErrorCode::ConfigError => "CONFIG_ERROR",
                ErrorCode::StoreError => "STORE_ERROR",
                ErrorCode::SshKeyInvalid => "SSH_KEY_INVALID",
            }
            .to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(ErrorCode::IoError, format!("Filesystem error: {e}"))
    }
}

impl From<git2::Error> for Error {
    fn from(e: git2::Error) -> Self {
        let msg = e.message().to_string();
        let code = if msg.contains("authentication")
            || msg.contains("credential")
            || msg.contains("401")
            || msg.contains("403")
        {
            ErrorCode::CloneFailed
        } else if msg.contains("would clobber")
            || msg.contains("non-fast-forward")
            || msg.contains("merge")
        {
            ErrorCode::PullFfFailed
        } else if msg.contains("unable to connect")
            || msg.contains("timeout")
            || msg.contains("network")
        {
            ErrorCode::NetworkError
        } else {
            ErrorCode::CloneFailed
        };
        Error::new(code, msg)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::new(ErrorCode::ConfigError, format!("Config error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_code_string(code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::CloneFailed => "CLONE_FAILED",
            ErrorCode::PullFfFailed => "PULL_FF_FAILED",
            ErrorCode::DecryptFailed => "DECRYPT_FAILED",
            ErrorCode::InvalidIdentity => "INVALID_IDENTITY",
            ErrorCode::NoRepo => "NO_REPO",
            ErrorCode::NoIdentity => "NO_IDENTITY",
            ErrorCode::NetworkError => "NETWORK_ERROR",
            ErrorCode::EntryNotFound => "ENTRY_NOT_FOUND",
            ErrorCode::IoError => "IO_ERROR",
            ErrorCode::ConfigError => "CONFIG_ERROR",
            ErrorCode::StoreError => "STORE_ERROR",
            ErrorCode::SshKeyInvalid => "SSH_KEY_INVALID",
        }
    }

    #[test]
    fn error_code_serialize() {
        let variants = [
            ErrorCode::CloneFailed,
            ErrorCode::PullFfFailed,
            ErrorCode::DecryptFailed,
            ErrorCode::InvalidIdentity,
            ErrorCode::NoRepo,
            ErrorCode::NoIdentity,
            ErrorCode::NetworkError,
            ErrorCode::EntryNotFound,
            ErrorCode::IoError,
            ErrorCode::ConfigError,
            ErrorCode::StoreError,
            ErrorCode::SshKeyInvalid,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap_or_default();
            let expected = format!("\"{}\"", expected_code_string(variant));
            assert_eq!(
                json, expected,
                "ErrorCode::{variant:?} did not serialize correctly"
            );
        }
    }

    #[test]
    fn error_new_maps_codes() {
        let variants = [
            ErrorCode::CloneFailed,
            ErrorCode::PullFfFailed,
            ErrorCode::DecryptFailed,
            ErrorCode::InvalidIdentity,
            ErrorCode::NoRepo,
            ErrorCode::NoIdentity,
            ErrorCode::NetworkError,
            ErrorCode::EntryNotFound,
            ErrorCode::IoError,
            ErrorCode::ConfigError,
            ErrorCode::StoreError,
            ErrorCode::SshKeyInvalid,
        ];
        for variant in variants {
            let err = Error::new(variant, "test message");
            assert_eq!(
                err.code,
                expected_code_string(variant),
                "Error::new code mismatch for {variant:?}"
            );
            assert_eq!(err.message, "test message");
        }
    }

    #[test]
    fn error_display_format() {
        let err = Error::new(ErrorCode::DecryptFailed, "bad key");
        let displayed = format!("{err}");
        assert_eq!(displayed, "DECRYPT_FAILED: bad key");
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let app_err: Error = io_err.into();
        assert_eq!(app_err.code, "IO_ERROR");
        assert!(
            app_err.message.contains("file missing"),
            "message should contain original io error: {}",
            app_err.message
        );
    }

    #[test]
    fn from_git2_error_maps_correctly() {
        let err = git2::Repository::open("/nonexistent/path/that/does/not/exist");
        let Err(git_err) = err else {
            panic!("expected error opening nonexistent repo");
        };
        let app_err: Error = git_err.into();
        assert_eq!(
            app_err.code, "CLONE_FAILED",
            "unmatched git2 error should map to CLONE_FAILED"
        );
    }

    #[test]
    fn from_serde_json_error() {
        let serde_err = serde_json::from_str::<serde_json::Value>("{invalid").unwrap_err();
        let app_err: Error = serde_err.into();
        assert_eq!(app_err.code, "CONFIG_ERROR");
        assert!(
            app_err.message.contains("Config error:"),
            "message should have Config error prefix: {}",
            app_err.message
        );
    }
}
