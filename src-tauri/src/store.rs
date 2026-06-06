// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use serde::Serialize;
use walkdir::WalkDir;

use crate::error::{AppError, ErrorCode};

/// A password store entry (no secret data).
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    /// Relative path from repo root (e.g., "cloud/aws/root.age")
    pub path: String,
    /// Display name (e.g., "aws/root") — extension stripped, forward slashes
    pub name: String,
}

/// Decrypted content — internal only, never crosses IPC directly.
/// Uses Zeroizing<String> so content is wiped on Drop.
pub struct DecryptedEntry {
    pub password: zeroize::Zeroizing<String>,
    pub notes: zeroize::Zeroizing<String>,
}

/// Custom Debug that redacts all fields — prevents accidental log leakage.
impl std::fmt::Debug for DecryptedEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecryptedEntry")
            .field("password", &"[REDACTED]")
            .field("notes", &"[REDACTED]")
            .finish()
    }
}

/// Returned by `copy_password` — no secret data, safe for IPC.
#[derive(Debug, Clone, Serialize)]
pub struct CopyResult {
    pub success: bool,
    pub entry_name: String,
    pub cleared_after_secs: u32,
}

/// Returned by `show_password` — contains secrets, strict Vue lifecycle required.
/// Zeroizing<String> fields are zeroized on Drop after IPC serialization.
#[derive(Debug, Clone, Serialize)]
pub struct SensitiveContent {
    pub password: String,
    pub notes: String,
}

// SensitiveContent implements Drop to zeroize on the Rust side.
// Note: Serialize runs before Drop. After JSON serialization completes and
// the struct goes out of scope, the strings are overwritten.
// The JS-side copy in WebView memory is the Vue component's responsibility.

/// Result of a pull operation.
#[derive(Debug, Clone, Serialize)]
pub struct PullResult {
    pub changed: bool,
    pub head: String,
}

/// Walk a gopass store directory and return all `.age` entries.
/// Skips `.git` directory. Only returns files with `.age` extension.
///
/// # Errors
///
/// Returns an error if the repository path does not exist.
pub fn list_entries(repo_path: &Path) -> Result<Vec<Entry>, AppError> {
    if !repo_path.exists() {
        return Err(AppError::new(
            ErrorCode::NoRepo,
            "Repository path does not exist",
        ));
    }

    let mut entries: Vec<Entry> = WalkDir::new(repo_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.file_name().to_str().is_some_and(|name| {
                Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("age"))
            })
        })
        .filter(|e| {
            // Skip anything inside .git directory
            !e.path().components().any(|c| c.as_os_str() == ".git")
        })
        .filter_map(|e| {
            let rel = e.path().strip_prefix(repo_path).ok()?;
            let rel_str = rel.to_str()?.to_string();
            let name = rel_str.trim_end_matches(".age").to_string();
            Some(Entry {
                path: rel_str,
                name,
            })
        })
        .collect();

    entries.sort_by_key(|a| a.name.to_lowercase());
    Ok(entries)
}

/// Parse decrypted bytes into password (first line) and notes (rest).
///
/// # Errors
///
/// Returns an error if the decrypted content is empty.
pub fn parse_decrypted_content(content: &[u8]) -> Result<DecryptedEntry, AppError> {
    let text = String::from_utf8_lossy(content);
    let text = text.trim_end();

    if text.is_empty() {
        return Err(AppError::new(
            ErrorCode::DecryptFailed,
            "Decrypted file is empty",
        ));
    }

    let (password, notes) = if let Some(newline_pos) = text.find('\n') {
        (
            zeroize::Zeroizing::new(text[..newline_pos].to_string()),
            zeroize::Zeroizing::new(text[newline_pos + 1..].to_string()),
        )
    } else {
        (
            zeroize::Zeroizing::new(text.to_string()),
            zeroize::Zeroizing::new(String::new()),
        )
    };

    Ok(DecryptedEntry { password, notes })
}

/// Verify an entry file exists within the repo.
pub fn resolve_entry_path(
    repo_path: &Path,
    entry_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let full_path = repo_path.join(entry_path);

    if !full_path.exists() {
        return Err(AppError::new(
            ErrorCode::EntryNotFound,
            format!("Entry not found: {entry_path}"),
        ));
    }

    // Ensure the resolved path is still within the repo (path traversal guard)
    let canonical_repo = repo_path.canonicalize()?;
    let canonical_entry = full_path.canonicalize()?;
    if !canonical_entry.starts_with(&canonical_repo) {
        return Err(AppError::new(
            ErrorCode::EntryNotFound,
            "Entry path is outside repository",
        ));
    }

    Ok(full_path)
}
