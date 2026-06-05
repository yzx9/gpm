use std::path::PathBuf;

use crate::error::{AppError, ErrorCode};

/// MVP secure storage: stores identity in an app-private file.
/// Post-MVP: Android Keystore-backed encryption.
///
/// The identity file is stored in the app's config directory.
/// On Android, this is app-private storage (other apps can't read it).
/// On desktop, it's in the standard config directory.
pub struct SecureStorage {
    config_dir: PathBuf,
}

impl SecureStorage {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Get the default config directory for the app.
    pub fn default_config_dir() -> Result<PathBuf, AppError> {
        let dir = dirs::config_dir().ok_or_else(|| {
            AppError::new(
                ErrorCode::ConfigError,
                "Cannot determine config directory",
            )
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
    pub fn save_identity(&self, identity: &[u8]) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::write(self.identity_path(), identity)?;
        Ok(())
    }

    /// Load the age identity from local storage.
    /// The caller MUST zeroize the returned bytes after use.
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
    #[allow(dead_code)]
    pub fn delete_identity(&self) -> Result<(), AppError> {
        let path = self.identity_path();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Save repository configuration (URL + local path).
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
    pub fn is_configured(&self) -> bool {
        self.identity_path().exists() && self.repo_config_path().exists()
    }

    /// Clear all stored configuration.
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoConfig {
    pub url: String,
    pub pat: Option<String>,
    pub local_path: String,
}
