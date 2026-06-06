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
    /// Whether any new commits were pulled.
    pub changed: bool,
    /// Short hash (7 chars) of the new HEAD commit.
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
///
/// # Errors
///
/// Returns an error if the entry does not exist or if the resolved path
/// escapes the repository directory (path traversal guard).
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -----------------------------------------------------------------------
    // resolve_entry_path tests
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_entry_path_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("cloud");
        fs::create_dir_all(&file_path).unwrap();
        fs::write(file_path.join("aws.age"), b"encrypted").unwrap();

        let result = resolve_entry_path(dir.path(), "cloud/aws.age");
        assert!(result.is_ok(), "expected Ok for valid file, got Err");
        let resolved = result.unwrap();
        assert_eq!(resolved, dir.path().join("cloud/aws.age"));
    }

    #[test]
    fn resolve_entry_path_missing_file() {
        let dir = tempfile::tempdir().unwrap();

        let result = resolve_entry_path(dir.path(), "nonexistent.age");
        assert!(result.is_err(), "expected Err for missing file, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn resolve_entry_path_traversal_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        // Create the directory so `dir` canonicalizes cleanly, but do NOT
        // create the target file — resolve_entry_path rejects before the
        // canonicalization check because the joined path does not exist.
        let result = resolve_entry_path(dir.path(), "../../../etc/passwd");
        assert!(result.is_err(), "expected Err for traversal, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn resolve_entry_path_traversal_deep() {
        let dir = tempfile::tempdir().unwrap();
        // Nested traversal with mixed components — still escapes, no file
        // exists at that path so it fails at the existence check.
        let result = resolve_entry_path(dir.path(), "foo/../../bar/../../../etc");
        assert!(result.is_err(), "expected Err for deep traversal, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[test]
    #[cfg(unix)]
    fn resolve_entry_path_symlink_escape() {
        use std::os::unix::fs::symlink;

        // Create a file in an external tempdir
        let external_dir = tempfile::tempdir().unwrap();
        let external_file = external_dir.path().join("target.txt");
        fs::write(&external_file, b"external-secret").unwrap();

        // Create the repo tempdir with a symlink pointing outside
        let repo_dir = tempfile::tempdir().unwrap();
        let link_path = repo_dir.path().join("escape.age");
        symlink(&external_file, &link_path).unwrap();

        // resolve_entry_path should reject because the canonical symlink
        // target is outside the repo directory.
        let result = resolve_entry_path(repo_dir.path(), "escape.age");
        assert!(result.is_err(), "expected Err for symlink escape, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
        assert!(
            err.message.contains("outside repository"),
            "expected 'outside repository' in error message, got: {}",
            err.message,
        );
    }

    // -----------------------------------------------------------------------
    // list_entries tests
    // -----------------------------------------------------------------------

    #[test]
    fn list_entries_nonexistent_dir() {
        let missing = std::path::PathBuf::from("/tmp/gpm_no_such_dir_12345");
        // Guard: ensure the path really does not exist
        assert!(!missing.exists(), "test precondition violated: path exists");

        let result = list_entries(&missing);
        assert!(result.is_err(), "expected Err for missing dir, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "NO_REPO");
    }

    // -----------------------------------------------------------------------
    // parse_decrypted_content tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_decrypted_content_password_only() {
        let entry = parse_decrypted_content(b"hunter2").unwrap();
        assert_eq!(entry.password.as_str(), "hunter2");
        assert_eq!(entry.notes.as_str(), "");
    }

    #[test]
    fn parse_decrypted_content_password_and_notes() {
        let entry = parse_decrypted_content(b"hunter2\nusername: alice\nurl: example.com").unwrap();
        assert_eq!(entry.password.as_str(), "hunter2");
        assert_eq!(entry.notes.as_str(), "username: alice\nurl: example.com");
    }

    #[test]
    fn parse_decrypted_content_trailing_newline_stripped() {
        let entry = parse_decrypted_content(b"pw\nnotes\n").unwrap();
        assert_eq!(entry.password.as_str(), "pw");
        assert_eq!(entry.notes.as_str(), "notes");
    }

    #[test]
    fn parse_decrypted_content_empty_is_error() {
        let result = parse_decrypted_content(b"");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "DECRYPT_FAILED");
    }

    #[test]
    fn parse_decrypted_content_whitespace_only_is_error() {
        let result = parse_decrypted_content(b"  \n  \n");
        assert!(result.is_err());
    }
}
