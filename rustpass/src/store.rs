// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;
use zeroize::Zeroize;

use crate::config::Config;
use crate::crypto;
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::git;
use crate::secret::Secret;

/// Result of a sync (pull) operation — aligned with gopass `Store.Sync`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    /// Whether any new commits were pulled.
    pub changed: bool,
    /// Short hash (7 chars) of the new HEAD commit.
    pub head: String,
}

/// Password store — aligned with `gopass.Store` interface.
///
/// Provides read-only operations on a gopass-compatible password store:
/// [`list`](Store::list), [`get`](Store::get), and [`sync`](Store::sync) (pull).
#[derive(Debug)]
pub struct Store {
    config: Config,
}

impl Store {
    /// Create a new `Store` backed by the given config directory.
    #[must_use]
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config: Config::new(config_dir),
        }
    }

    /// Check if the store has been configured (identity + repo exist).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.config.is_configured()
    }

    /// Configure the store: validate identity, clone repo, save config.
    ///
    /// This is the setup/init operation. It clears any existing configuration
    /// before applying the new one.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity format is invalid, the clone fails,
    /// or the config cannot be persisted.
    pub fn configure(
        &self,
        repo_url: &str,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
        identity: &str,
    ) -> Result<(), Error> {
        // Validate identity format
        let identity_bytes = identity.trim().as_bytes();
        if !identity.trim().starts_with("AGE-SECRET-KEY-") {
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                "Identity must start with AGE-SECRET-KEY-...",
            ));
        }

        // Build auth from provided credentials
        let auth = match (ssh_key, pat) {
            (Some(key), _) => git::GitAuth::Ssh {
                username: "git".to_string(),
                private_key: key.to_string(),
                passphrase: ssh_passphrase.map(String::from),
            },
            (_, Some(token)) => git::GitAuth::Pat(token.to_string()),
            _ => git::GitAuth::None,
        };

        // Determine local repo path
        let repo_dir = self.config.config_dir().join("repo");

        // Clear any existing configuration
        self.config.clear_all()?;

        // Remove existing repo directory if present
        if repo_dir.exists() {
            std::fs::remove_dir_all(&repo_dir)?;
        }

        // Save identity first (before clone, so decrypt can work)
        self.config.save_identity(identity_bytes)?;

        // Clone the repo
        git::clone_repo(repo_url, &repo_dir, &auth)?;

        // Save repo config
        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, pat, ssh_key, ssh_passphrase, &local_path)?;

        Ok(())
    }

    /// List all `.age` entries in the configured repository.
    ///
    /// Aligned with `gopass.Store.List`.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the repo path
    /// does not exist.
    pub fn list(&self) -> Result<Vec<Entry>, Error> {
        let repo_config = self.config.load_repo_config()?;
        let repo_path = Path::new(&repo_config.local_path);
        list_entries(repo_path)
    }

    /// Decrypt and return a secret by entry name.
    ///
    /// The entry name is the display name without `.age` extension
    /// (e.g., `"cloud/aws/root"`). Aligned with `gopass.Store.Get`.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry does not exist, the identity is missing,
    /// or decryption fails.
    pub fn get(&self, name: &str) -> Result<Secret, Error> {
        let repo_config = self.config.load_repo_config()?;
        let repo_path = Path::new(&repo_config.local_path);

        // Resolve entry name to file path (append .age extension if needed)
        let entry_path = if Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("age"))
        {
            name.to_string()
        } else {
            format!("{name}.age")
        };

        let file_path = resolve_entry_path(repo_path, &entry_path)?;

        // Load identity (caller must zeroize)
        let mut identity_bytes = self.config.load_identity()?;

        // Decrypt
        let decrypted = crypto::decrypt_file(&file_path, &identity_bytes)?;

        // Zeroize identity immediately after decryption
        identity_bytes.zeroize();

        // Parse into Secret
        Secret::parse(&decrypted)
    }

    /// Pull latest changes from the remote (fast-forward only).
    ///
    /// Aligned with `gopass.Store.Sync` (pull-only).
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured, the remote is
    /// unreachable, or the branches have diverged.
    pub fn sync(&self) -> Result<SyncResult, Error> {
        let repo_config = self.config.load_repo_config()?;
        let repo_path = Path::new(&repo_config.local_path);
        let auth = repo_config.to_git_auth();
        git::pull_repo(repo_path, &auth)
    }

    /// Reset all configuration and local data.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be removed.
    pub fn reset(&self) -> Result<(), Error> {
        // Remove local repo if it exists
        if let Ok(repo_config) = self.config.load_repo_config() {
            let repo_path = Path::new(&repo_config.local_path);
            if repo_path.exists() {
                std::fs::remove_dir_all(repo_path)?;
            }
        }
        self.config.clear_all()
    }

    /// Get the current repository configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured.
    pub fn config(&self) -> Result<crate::config::RepoConfig, Error> {
        self.config.load_repo_config()
    }
}

// ---------------------------------------------------------------------------
// Low-level functions (used by Store, also publicly accessible)
// ---------------------------------------------------------------------------

/// Walk a gopass store directory and return all `.age` entries.
///
/// Skips `.git` directory. Only returns files with `.age` extension.
///
/// # Errors
///
/// Returns an error if the repository path does not exist.
pub fn list_entries(repo_path: &Path) -> Result<Vec<Entry>, Error> {
    if !repo_path.exists() {
        return Err(Error::new(
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

/// Verify an entry file exists within the repo.
///
/// # Errors
///
/// Returns an error if the entry does not exist or if the resolved path
/// escapes the repository directory (path traversal guard).
pub fn resolve_entry_path(repo_path: &Path, entry_path: &str) -> Result<PathBuf, Error> {
    let full_path = repo_path.join(entry_path);

    if !full_path.exists() {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            format!("Entry not found: {entry_path}"),
        ));
    }

    // Ensure the resolved path is still within the repo (path traversal guard)
    let canonical_repo = repo_path.canonicalize()?;
    let canonical_entry = full_path.canonicalize()?;
    if !canonical_entry.starts_with(&canonical_repo) {
        return Err(Error::new(
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
        let result = resolve_entry_path(dir.path(), "../../../etc/passwd");
        assert!(result.is_err(), "expected Err for traversal, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn resolve_entry_path_traversal_deep() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_entry_path(dir.path(), "foo/../../bar/../../../etc");
        assert!(result.is_err(), "expected Err for deep traversal, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[test]
    #[cfg(unix)]
    fn resolve_entry_path_symlink_escape() {
        use std::os::unix::fs::symlink;

        let external_dir = tempfile::tempdir().unwrap();
        let external_file = external_dir.path().join("target.txt");
        fs::write(&external_file, b"external-secret").unwrap();

        let repo_dir = tempfile::tempdir().unwrap();
        let link_path = repo_dir.path().join("escape.age");
        symlink(&external_file, &link_path).unwrap();

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
        let missing = PathBuf::from("/tmp/gpm_no_such_dir_12345");
        assert!(!missing.exists(), "test precondition violated: path exists");

        let result = list_entries(&missing);
        assert!(result.is_err(), "expected Err for missing dir, got Ok");
        let err = result.unwrap_err();
        assert_eq!(err.code, "NO_REPO");
    }
}
