// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use serde::Serialize;

/// Machine-readable error codes — all messages are safe (no secrets).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    CloneFailed,
    PullFfFailed,
    DecryptFailed,
    InvalidIdentity,
    NoRepo,
    NoIdentity,
    NetworkError,
    EntryNotFound,
    IoError,
    ClipboardError,
    ConfigError,
}

/// Safe error type that never contains secret content.
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    /// Machine-readable error code string (e.g. `"CLONE_FAILED"`).
    pub code: String,
    /// Human-readable error message (no secrets).
    pub message: String,
}

impl AppError {
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
                ErrorCode::ClipboardError => "CLIPBOARD_ERROR",
                ErrorCode::ConfigError => "CONFIG_ERROR",
            }
            .to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::new(ErrorCode::IoError, format!("Filesystem error: {e}"))
    }
}

impl From<git2::Error> for AppError {
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
        AppError::new(code, msg)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::new(ErrorCode::ConfigError, format!("Config error: {e}"))
    }
}
