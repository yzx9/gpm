// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use crate::error::{AppError, ErrorCode};

/// MVP secure storage: stores identity in an app-private file.
/// Post-MVP: Android Keystore-backed encryption.
///
/// The identity file is stored in the app's config directory.
/// On Android, this is app-private storage (other apps can't read it).
/// On desktop, it's in the standard config directory.
#[derive(Debug)]
pub struct SecureStorage {
    config_dir: PathBuf,
}

impl SecureStorage {
    /// Create a new storage instance rooted at the given config directory.
    #[must_use]
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Get the default config directory for the app.
    ///
    /// # Errors
    ///
    /// Returns an error if the system config directory cannot be determined.
    pub fn default_config_dir() -> Result<PathBuf, AppError> {
        let dir = dirs::config_dir().ok_or_else(|| {
            AppError::new(ErrorCode::ConfigError, "Cannot determine config directory")
        })?;
        Ok(dir.join("gpm"))
    }

    fn identity_path(&self) -> PathBuf {
        self.config_dir.join("identity")
    }

    fn repo_config_path(&self) -> PathBuf {
        self.config_dir.join("repo.json")
    }

    /// Save the age identity to local storage.
    /// The caller is responsible for zeroizing the identity bytes after this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created or the file
    /// cannot be written.
    pub fn save_identity(&self, identity: &[u8]) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::write(self.identity_path(), identity)?;
        Ok(())
    }

    /// Load the age identity from local storage.
    /// The caller MUST zeroize the returned bytes after use.
    ///
    /// # Errors
    ///
    /// Returns an error if no identity has been configured or the file cannot
    /// be read.
    pub fn load_identity(&self) -> Result<Vec<u8>, AppError> {
        let path = self.identity_path();
        if !path.exists() {
            return Err(AppError::new(
                ErrorCode::NoIdentity,
                "No identity configured. Run setup first.",
            ));
        }
        Ok(std::fs::read(&path)?)
    }

    /// Delete the stored identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be removed.
    #[allow(dead_code)]
    pub fn delete_identity(&self) -> Result<(), AppError> {
        let path = self.identity_path();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Save repository configuration (URL + local path).
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created or the file
    /// cannot be written.
    pub fn save_repo_config(
        &self,
        url: &str,
        pat: Option<&str>,
        local_path: &str,
    ) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.config_dir)?;
        let config = RepoConfig {
            url: url.to_string(),
            pat: pat.map(String::from),
            local_path: local_path.to_string(),
        };
        let json = serde_json::to_string_pretty(&config)?;
        std::fs::write(self.repo_config_path(), json)?;
        Ok(())
    }

    /// Load repository configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if no config exists, the file cannot be read, or the
    /// JSON is malformed.
    pub fn load_repo_config(&self) -> Result<RepoConfig, AppError> {
        let path = self.repo_config_path();
        if !path.exists() {
            return Err(AppError::new(
                ErrorCode::NoRepo,
                "No repository configured. Run setup first.",
            ));
        }
        let json = std::fs::read_to_string(&path)?;
        let config: RepoConfig = serde_json::from_str(&json)?;
        Ok(config)
    }

    /// Check if setup is complete (both identity and repo config exist).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.identity_path().exists() && self.repo_config_path().exists()
    }

    /// Clear all stored configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be removed.
    pub fn clear_all(&self) -> Result<(), AppError> {
        if self.identity_path().exists() {
            std::fs::remove_file(self.identity_path())?;
        }
        if self.repo_config_path().exists() {
            std::fs::remove_file(self.repo_config_path())?;
        }
        Ok(())
    }
}

/// Repository configuration persisted to disk.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoConfig {
    /// Remote repository URL.
    pub url: String,
    /// Optional personal access token for HTTPS authentication.
    pub pat: Option<String>,
    /// Local filesystem path where the repo is cloned.
    pub local_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_storage() -> (SecureStorage, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let storage = SecureStorage::new(dir.path().to_path_buf());
        (storage, dir)
    }

    #[test]
    fn save_load_identity_roundtrip() {
        let (storage, _dir) = create_storage();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        storage.save_identity(identity).unwrap();
        let loaded = storage.load_identity().unwrap();

        assert_eq!(loaded, identity);
    }

    #[test]
    fn load_identity_missing() {
        let (storage, _dir) = create_storage();

        let err = storage.load_identity().unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[test]
    fn delete_identity_removes_file() {
        let (storage, _dir) = create_storage();

        storage.save_identity(b"test-identity").unwrap();
        assert!(storage.identity_path().exists());

        storage.delete_identity().unwrap();
        assert!(!storage.identity_path().exists());

        let err = storage.load_identity().unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[test]
    fn save_load_repo_config_roundtrip() {
        let (storage, _dir) = create_storage();

        storage
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat-token"),
                "/local/repo",
            )
            .unwrap();

        let config = storage.load_repo_config().unwrap();
        assert_eq!(config.url, "https://example.com/repo.git");
        assert_eq!(config.pat, Some(String::from("pat-token")));
        assert_eq!(config.local_path, "/local/repo");
    }

    #[test]
    fn repo_config_with_pat() {
        let (storage, _dir) = create_storage();

        storage
            .save_repo_config(
                "https://example.com/repo.git",
                Some("my-secret-pat"),
                "/local/path",
            )
            .unwrap();

        let config = storage.load_repo_config().unwrap();
        assert_eq!(config.pat, Some(String::from("my-secret-pat")));
    }

    #[test]
    fn repo_config_without_pat() {
        let (storage, _dir) = create_storage();

        storage
            .save_repo_config("https://example.com/repo.git", None, "/local/path")
            .unwrap();

        let config = storage.load_repo_config().unwrap();
        assert_eq!(config.pat, None);
    }

    #[test]
    fn is_configured_false_initially() {
        let (storage, _dir) = create_storage();

        assert!(!storage.is_configured());
    }

    #[test]
    fn is_configured_true_after_setup() {
        let (storage, _dir) = create_storage();

        storage.save_identity(b"test-identity").unwrap();
        storage
            .save_repo_config("https://example.com/repo.git", None, "/local/path")
            .unwrap();

        assert!(storage.is_configured());
    }

    #[test]
    fn clear_all_removes_everything() {
        let (storage, _dir) = create_storage();

        storage.save_identity(b"test-identity").unwrap();
        storage
            .save_repo_config("https://example.com/repo.git", Some("pat"), "/local/path")
            .unwrap();
        assert!(storage.is_configured());

        storage.clear_all().unwrap();

        assert!(!storage.is_configured());
        let identity_err = storage.load_identity().unwrap_err();
        assert_eq!(identity_err.code, "NO_IDENTITY");
        let repo_err = storage.load_repo_config().unwrap_err();
        assert_eq!(repo_err.code, "NO_REPO");
    }
}
