// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use crate::error::{Error, ErrorCode};

/// Configuration and identity persistence for a password store.
///
/// Manages storage of age identity and repository configuration in an
/// app-private directory. On Android, this is app-private storage; on
/// desktop, it's the standard config directory.
#[derive(Debug)]
pub struct Config {
    config_dir: PathBuf,
}

impl Config {
    /// Create a new config instance rooted at the given directory.
    #[must_use]
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Get the config directory used by this instance.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    fn identity_path(&self) -> PathBuf {
        self.config_dir.join("identity")
    }

    fn repo_config_path(&self) -> PathBuf {
        self.config_dir.join("repo.json")
    }

    /// Save the age identity to local storage.
    ///
    /// The caller is responsible for zeroizing the identity bytes after this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created or the file
    /// cannot be written.
    pub fn save_identity(&self, identity: &[u8]) -> Result<(), Error> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::write(self.identity_path(), identity)?;
        Ok(())
    }

    /// Load the age identity from local storage.
    ///
    /// The caller **must** zeroize the returned bytes after use.
    ///
    /// # Errors
    ///
    /// Returns an error if no identity has been configured or the file cannot
    /// be read.
    pub fn load_identity(&self) -> Result<Vec<u8>, Error> {
        let path = self.identity_path();
        if !path.exists() {
            return Err(Error::new(
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
    pub fn delete_identity(&self) -> Result<(), Error> {
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
    ) -> Result<(), Error> {
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
    pub fn load_repo_config(&self) -> Result<RepoConfig, Error> {
        let path = self.repo_config_path();
        if !path.exists() {
            return Err(Error::new(
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
    pub fn clear_all(&self) -> Result<(), Error> {
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

    fn create_config() -> (Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        (config, dir)
    }

    #[test]
    fn save_load_identity_roundtrip() {
        let (config, _dir) = create_config();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        config.save_identity(identity).unwrap();
        let loaded = config.load_identity().unwrap();

        assert_eq!(loaded, identity);
    }

    #[test]
    fn load_identity_missing() {
        let (config, _dir) = create_config();

        let err = config.load_identity().unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[test]
    fn delete_identity_removes_file() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity").unwrap();
        assert!(config.identity_path().exists());

        config.delete_identity().unwrap();
        assert!(!config.identity_path().exists());

        let err = config.load_identity().unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[test]
    fn save_load_repo_config_roundtrip() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat-token"),
                "/local/repo",
            )
            .unwrap();

        let cfg = config.load_repo_config().unwrap();
        assert_eq!(cfg.url, "https://example.com/repo.git");
        assert_eq!(cfg.pat, Some(String::from("pat-token")));
        assert_eq!(cfg.local_path, "/local/repo");
    }

    #[test]
    fn repo_config_with_pat() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("my-secret-pat"),
                "/local/path",
            )
            .unwrap();

        let cfg = config.load_repo_config().unwrap();
        assert_eq!(cfg.pat, Some(String::from("my-secret-pat")));
    }

    #[test]
    fn repo_config_without_pat() {
        let (config, _dir) = create_config();

        config
            .save_repo_config("https://example.com/repo.git", None, "/local/path")
            .unwrap();

        let cfg = config.load_repo_config().unwrap();
        assert_eq!(cfg.pat, None);
    }

    #[test]
    fn is_configured_false_initially() {
        let (config, _dir) = create_config();

        assert!(!config.is_configured());
    }

    #[test]
    fn is_configured_true_after_setup() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity").unwrap();
        config
            .save_repo_config("https://example.com/repo.git", None, "/local/path")
            .unwrap();

        assert!(config.is_configured());
    }

    #[test]
    fn clear_all_removes_everything() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity").unwrap();
        config
            .save_repo_config("https://example.com/repo.git", Some("pat"), "/local/path")
            .unwrap();
        assert!(config.is_configured());

        config.clear_all().unwrap();

        assert!(!config.is_configured());
        let identity_err = config.load_identity().unwrap_err();
        assert_eq!(identity_err.code, "NO_IDENTITY");
        let repo_err = config.load_repo_config().unwrap_err();
        assert_eq!(repo_err.code, "NO_REPO");
    }

    #[test]
    fn overwrite_identity() {
        let (config, _dir) = create_config();

        config.save_identity(b"first-identity").unwrap();
        config.save_identity(b"second-identity").unwrap();

        let loaded = config.load_identity().unwrap();
        assert_eq!(loaded, b"second-identity");
    }

    #[test]
    fn partial_setup_identity_only() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity").unwrap();
        assert!(
            !config.is_configured(),
            "should not be configured without repo config"
        );
    }

    #[test]
    fn partial_setup_repo_only() {
        let (config, _dir) = create_config();

        config
            .save_repo_config("https://example.com/repo.git", None, "/local/path")
            .unwrap();
        assert!(
            !config.is_configured(),
            "should not be configured without identity"
        );
    }

    #[test]
    fn creates_config_dir_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        let config = Config::new(nested.clone());

        assert!(!nested.exists(), "precondition: dir does not exist");
        config.save_identity(b"test").unwrap();
        assert!(
            nested.exists(),
            "save_identity should create the config dir"
        );
    }
}
