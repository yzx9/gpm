// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use crate::crypto;
use crate::error::{Error, ErrorCode};
use crate::identity::classify_identity;
use crate::identity::IdentityType;

/// Atomic write: write data to a temp file then rename over the target.
///
/// Prevents identity file corruption if the write fails mid-operation.
async fn save_identity_raw(path: &Path, data: &[u8]) -> Result<(), Error> {
    let temp_path = path.with_extension("tmp");
    tokio::fs::write(&temp_path, data).await?;
    tokio::fs::rename(&temp_path, path).await?;
    Ok(())
}

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
    /// If `passphrase` is `Some`, the identity is encrypted with age scrypt
    /// before writing. If `None`, the identity is stored as plaintext.
    /// Uses atomic write (temp file + rename) to prevent corruption.
    ///
    /// The caller is responsible for zeroizing the identity bytes after this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created, encryption
    /// fails, or the file cannot be written.
    pub async fn save_identity(
        &self,
        identity: &[u8],
        passphrase: Option<&str>,
    ) -> Result<(), Error> {
        tokio::fs::create_dir_all(&self.config_dir).await?;

        let data = match passphrase {
            Some(pw) if !pw.is_empty() => crypto::encrypt_identity(pw, identity)?,
            _ => identity.to_vec(),
        };

        save_identity_raw(&self.identity_path(), &data).await
    }

    /// Check if the stored identity file is passphrase-encrypted.
    ///
    /// Returns `false` if no identity file exists.
    pub async fn is_identity_encrypted(&self) -> bool {
        match self.load_identity().await {
            Ok(bytes) => classify_identity(&bytes) == IdentityType::AgeEncrypted,
            Err(_) => false,
        }
    }

    /// Load the age identity from local storage.
    ///
    /// The caller **must** zeroize the returned bytes after use.
    ///
    /// # Errors
    ///
    /// Returns an error if no identity has been configured or the file cannot
    /// be read.
    pub async fn load_identity(&self) -> Result<Vec<u8>, Error> {
        let path = self.identity_path();
        if !path.exists() {
            return Err(Error::new(
                ErrorCode::NoIdentity,
                "No identity configured. Run setup first.",
            ));
        }
        Ok(tokio::fs::read(&path).await?)
    }

    /// Delete the stored identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be removed.
    pub async fn delete_identity(&self) -> Result<(), Error> {
        let path = self.identity_path();
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }

    /// Save repository configuration (URL + local path).
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created or the file
    /// cannot be written.
    pub async fn save_repo_config(
        &self,
        url: &str,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
        local_path: &str,
    ) -> Result<(), Error> {
        tokio::fs::create_dir_all(&self.config_dir).await?;
        let config = RepoConfig {
            url: url.to_string(),
            pat: pat.map(String::from),
            ssh_key: ssh_key.map(String::from),
            ssh_passphrase: ssh_passphrase.map(String::from),
            local_path: local_path.to_string(),
        };
        let json = serde_json::to_string_pretty(&config)?;
        tokio::fs::write(self.repo_config_path(), json).await?;
        Ok(())
    }

    /// Load repository configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if no config exists, the file cannot be read, or the
    /// JSON is malformed.
    pub async fn load_repo_config(&self) -> Result<RepoConfig, Error> {
        let path = self.repo_config_path();
        if !path.exists() {
            return Err(Error::new(
                ErrorCode::NoRepo,
                "No repository configured. Run setup first.",
            ));
        }
        let json = tokio::fs::read_to_string(&path).await?;
        let config: RepoConfig = serde_json::from_str(&json)?;
        Ok(config)
    }

    /// Check if setup is complete (both identity and repo config exist).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.identity_path().exists() && self.repo_config_path().exists()
    }

    /// Check if repo config exists (identity may or may not be present).
    #[must_use]
    pub fn repo_config_exists(&self) -> bool {
        self.repo_config_path().exists()
    }

    /// Clear all stored configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be removed.
    pub async fn clear_all(&self) -> Result<(), Error> {
        if self.identity_path().exists() {
            tokio::fs::remove_file(self.identity_path()).await?;
        }
        if self.repo_config_path().exists() {
            tokio::fs::remove_file(self.repo_config_path()).await?;
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
    /// Optional SSH private key for SSH authentication.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ssh_key: Option<String>,
    /// Optional passphrase for encrypted SSH key.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ssh_passphrase: Option<String>,
    /// Local filesystem path where the repo is cloned.
    pub local_path: String,
}

impl RepoConfig {
    /// Build a [`GitAuth`](crate::git::GitAuth) from stored credentials.
    ///
    /// SSH key takes priority if both PAT and SSH key are present.
    #[must_use]
    pub fn to_git_auth(&self) -> crate::git::GitAuth {
        if let Some(key) = &self.ssh_key {
            crate::git::GitAuth::Ssh {
                username: "git".to_string(),
                private_key: key.clone(),
                passphrase: self.ssh_passphrase.clone(),
            }
        } else if let Some(token) = &self.pat {
            crate::git::GitAuth::Pat(token.clone())
        } else {
            crate::git::GitAuth::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_config() -> (Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        (config, dir)
    }

    #[tokio::test]
    async fn save_load_identity_roundtrip() {
        let (config, _dir) = create_config();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        config.save_identity(identity, None).await.unwrap();
        let loaded = config.load_identity().await.unwrap();

        assert_eq!(loaded, identity);
    }

    #[tokio::test]
    async fn save_load_encrypted_identity_roundtrip() {
        let (config, _dir) = create_config();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        config
            .save_identity(identity, Some("test-passphrase"))
            .await
            .unwrap();

        assert!(
            config.is_identity_encrypted().await,
            "identity should be encrypted"
        );

        let loaded = config.load_identity().await.unwrap();
        assert!(loaded.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));

        let decrypted = crypto::decrypt_identity("test-passphrase", &loaded).unwrap();
        assert_eq!(decrypted, identity);
    }

    #[tokio::test]
    async fn save_identity_empty_passphrase_stores_plaintext() {
        let (config, _dir) = create_config();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        config.save_identity(identity, Some("")).await.unwrap();
        assert!(
            !config.is_identity_encrypted().await,
            "empty passphrase should store plaintext"
        );

        let loaded = config.load_identity().await.unwrap();
        assert_eq!(loaded, identity);
    }

    #[tokio::test]
    async fn is_identity_encrypted_false_when_no_identity() {
        let (config, _dir) = create_config();
        assert!(!config.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn load_identity_missing() {
        let (config, _dir) = create_config();

        let err = config.load_identity().await.unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[tokio::test]
    async fn delete_identity_removes_file() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        assert!(config.identity_path().exists());

        config.delete_identity().await.unwrap();
        assert!(!config.identity_path().exists());

        let err = config.load_identity().await.unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[tokio::test]
    async fn save_load_repo_config_roundtrip() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat-token"),
                None,
                None,
                "/local/repo",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.url, "https://example.com/repo.git");
        assert_eq!(cfg.pat, Some(String::from("pat-token")));
        assert_eq!(cfg.local_path, "/local/repo");
    }

    #[tokio::test]
    async fn repo_config_with_pat() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("my-secret-pat"),
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.pat, Some(String::from("my-secret-pat")));
    }

    #[tokio::test]
    async fn repo_config_without_pat() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                None,
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.pat, None);
    }

    #[test]
    fn is_configured_false_initially() {
        let (config, _dir) = create_config();

        assert!(!config.is_configured());
    }

    #[tokio::test]
    async fn is_configured_true_after_setup() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        config
            .save_repo_config(
                "https://example.com/repo.git",
                None,
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        assert!(config.is_configured());
    }

    #[tokio::test]
    async fn clear_all_removes_everything() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat"),
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();
        assert!(config.is_configured());

        config.clear_all().await.unwrap();

        assert!(!config.is_configured());
        let identity_err = config.load_identity().await.unwrap_err();
        assert_eq!(identity_err.code, "NO_IDENTITY");
        let repo_err = config.load_repo_config().await.unwrap_err();
        assert_eq!(repo_err.code, "NO_REPO");
    }

    #[tokio::test]
    async fn overwrite_identity() {
        let (config, _dir) = create_config();

        config.save_identity(b"first-identity", None).await.unwrap();
        config
            .save_identity(b"second-identity", None)
            .await
            .unwrap();

        let loaded = config.load_identity().await.unwrap();
        assert_eq!(loaded, b"second-identity");
    }

    #[tokio::test]
    async fn partial_setup_identity_only() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        assert!(
            !config.is_configured(),
            "should not be configured without repo config"
        );
    }

    #[tokio::test]
    async fn partial_setup_repo_only() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                None,
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();
        assert!(
            !config.is_configured(),
            "should not be configured without identity"
        );
    }

    #[tokio::test]
    async fn repo_config_with_ssh_key() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "git@github.com:user/repo.git",
                None,
                Some("-----BEGIN OPENSSH PRIVATE KEY-----\ntest-key\n-----END OPENSSH PRIVATE KEY-----"),
                Some("passphrase123"),
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.url, "git@github.com:user/repo.git");
        assert_eq!(cfg.pat, None);
        assert!(cfg.ssh_key.is_some(), "ssh_key should be set");
        assert!(
            cfg.ssh_key.as_ref().unwrap().contains("BEGIN OPENSSH"),
            "ssh_key should contain key data"
        );
        assert_eq!(cfg.ssh_passphrase, Some(String::from("passphrase123")));
    }

    #[tokio::test]
    async fn repo_config_backward_compat_no_ssh_fields() {
        let (config, _dir) = create_config();

        // Simulate old config JSON without ssh_key/ssh_passphrase fields
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let old_json =
            r#"{"url":"https://example.com/repo.git","pat":"my-token","local_path":"/local/path"}"#;
        std::fs::write(config.repo_config_path(), old_json).unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.url, "https://example.com/repo.git");
        assert_eq!(cfg.pat, Some(String::from("my-token")));
        assert_eq!(
            cfg.ssh_key, None,
            "ssh_key should default to None for old config"
        );
        assert_eq!(
            cfg.ssh_passphrase, None,
            "ssh_passphrase should default to None for old config"
        );
    }

    #[test]
    fn to_git_auth_returns_ssh_when_key_present() {
        let cfg = RepoConfig {
            url: "git@github.com:user/repo.git".to_string(),
            pat: Some("some-token".to_string()),
            ssh_key: Some("test-key".to_string()),
            ssh_passphrase: Some("test-pass".to_string()),
            local_path: "/local".to_string(),
        };

        let auth = cfg.to_git_auth();
        match auth {
            crate::git::GitAuth::Ssh {
                username,
                private_key,
                passphrase,
            } => {
                assert_eq!(username, "git");
                assert_eq!(private_key, "test-key");
                assert_eq!(passphrase, Some("test-pass".to_string()));
            }
            _ => panic!("expected GitAuth::Ssh, got {auth:?}"),
        }
    }

    #[test]
    fn to_git_auth_returns_pat_when_no_ssh_key() {
        let cfg = RepoConfig {
            url: "https://example.com/repo.git".to_string(),
            pat: Some("my-token".to_string()),
            ssh_key: None,
            ssh_passphrase: None,
            local_path: "/local".to_string(),
        };

        let auth = cfg.to_git_auth();
        match auth {
            crate::git::GitAuth::Pat(token) => assert_eq!(token, "my-token"),
            _ => panic!("expected GitAuth::Pat, got {auth:?}"),
        }
    }

    #[test]
    fn to_git_auth_returns_none_when_no_credentials() {
        let cfg = RepoConfig {
            url: "https://example.com/public-repo.git".to_string(),
            pat: None,
            ssh_key: None,
            ssh_passphrase: None,
            local_path: "/local".to_string(),
        };

        let auth = cfg.to_git_auth();
        assert!(
            matches!(auth, crate::git::GitAuth::None),
            "expected GitAuth::None, got {auth:?}"
        );
    }

    #[test]
    fn to_git_auth_ssh_overrides_pat() {
        // When both PAT and SSH key are present, SSH takes priority
        let cfg = RepoConfig {
            url: "git@github.com:user/repo.git".to_string(),
            pat: Some("ignored-token".to_string()),
            ssh_key: Some("ssh-key".to_string()),
            ssh_passphrase: None,
            local_path: "/local".to_string(),
        };

        let auth = cfg.to_git_auth();
        assert!(
            matches!(auth, crate::git::GitAuth::Ssh { .. }),
            "SSH should take priority over PAT"
        );
    }

    #[tokio::test]
    async fn repo_config_ssh_key_without_passphrase() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "git@github.com:user/repo.git",
                None,
                Some("test-key"),
                None,
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.ssh_key, Some(String::from("test-key")));
        assert_eq!(cfg.ssh_passphrase, None);
    }

    #[tokio::test]
    async fn repo_config_ssh_fields_not_serialized_when_none() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat"),
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        // Read raw JSON to verify ssh fields are omitted
        let json = std::fs::read_to_string(config.repo_config_path()).unwrap();
        assert!(
            !json.contains("ssh_key"),
            "ssh_key should not appear in JSON when None"
        );
        assert!(
            !json.contains("ssh_passphrase"),
            "ssh_passphrase should not appear in JSON when None"
        );
    }

    #[tokio::test]
    async fn creates_config_dir_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        let config = Config::new(nested.clone());

        assert!(!nested.exists(), "precondition: dir does not exist");
        config.save_identity(b"test", None).await.unwrap();
        assert!(
            nested.exists(),
            "save_identity should create the config dir"
        );
    }
}
