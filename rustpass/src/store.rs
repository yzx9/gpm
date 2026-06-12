// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::Serialize;
use walkdir::WalkDir;
use zeroize::Zeroizing;

use crate::config::Config;
use crate::crypto;
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::git;
use crate::identity::{classify_identity, IdentityType};
use crate::recipient::{self, Recipient};
use crate::secret::Secret;

/// Default auto-lock timeout in seconds (5 minutes).
pub const DEFAULT_LOCK_TIMEOUT_SECS: u64 = 300;

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
/// Supports optional passphrase-encrypted identity with in-memory caching.
pub struct Store {
    config: Config,
    /// Cached decrypted identity (populated after unlock).
    cached_identity: RwLock<Option<Zeroizing<Vec<u8>>>>,
    /// Cached passphrase for encrypted SSH key decryption.
    cached_passphrase: RwLock<Option<Zeroizing<String>>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("config", &self.config)
            .field(
                "cached_identity",
                &self.cached_identity.read().ok().map(|g| g.is_some()),
            )
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Create a new `Store` backed by the given config directory.
    #[must_use]
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config: Config::new(config_dir),
            cached_identity: RwLock::new(None),
            cached_passphrase: RwLock::new(None),
        }
    }

    /// Check if the store has been configured (identity + repo exist).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.config.is_configured()
    }

    /// Check if the repo has been cloned (identity may not be saved yet).
    #[must_use]
    pub fn is_repo_ready(&self) -> bool {
        self.config.repo_config_exists()
    }

    /// Check if the stored identity requires a passphrase.
    ///
    /// Returns true for age-encrypted identities and encrypted SSH keys.
    /// Returns false for plaintext x25519 keys and unencrypted SSH keys.
    #[must_use]
    pub fn is_identity_encrypted(&self) -> bool {
        let Ok(bytes) = self.config.load_identity() else {
            return false;
        };
        let itype = classify_identity(&bytes);

        if itype == IdentityType::AgeEncrypted {
            return true;
        }

        if matches!(itype, IdentityType::SshEd25519 | IdentityType::SshRsa) {
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return false;
            };
            let buf = std::io::BufReader::new(text.trim().as_bytes());
            return matches!(
                age::ssh::Identity::from_buffer(buf, None),
                Ok(age::ssh::Identity::Encrypted(_))
            );
        }

        false
    }

    /// Check if the identity cache is populated (identity is unlocked).
    #[must_use]
    pub fn is_unlocked(&self) -> bool {
        self.cached_identity
            .read()
            .is_ok_and(|guard| guard.is_some())
    }

    /// Unlock a passphrase-encrypted identity by decrypting and caching it.
    ///
    /// Calling `unlock()` when already unlocked is idempotent (re-decrypts
    /// and overwrites the cache).
    ///
    /// # Errors
    ///
    /// Returns `IdentityNotEncrypted` if the identity is not encrypted.
    /// Returns `WrongPassphrase` if the passphrase is incorrect.
    /// Returns `NoIdentity` if no identity is configured.
    pub fn unlock(&self, passphrase: &str) -> Result<(), Error> {
        let encrypted_bytes = self.config.load_identity()?;

        let itype = classify_identity(&encrypted_bytes);

        if itype == IdentityType::AgeEncrypted {
            // Age-encrypted identity: decrypt with passphrase
            let decrypted = crypto::decrypt_identity(passphrase, &encrypted_bytes)?;
            let zeroizing = Zeroizing::new(decrypted);

            {
                let mut cache = self
                    .cached_identity
                    .write()
                    .map_err(|_| Error::new(ErrorCode::StoreError, "Cache lock poisoned"))?;
                *cache = Some(zeroizing);
            }
        }

        // Cache passphrase for encrypted SSH key decryption (works for both
        // age-encrypted and plaintext encrypted SSH keys)
        {
            let mut cache = self
                .cached_passphrase
                .write()
                .map_err(|_| Error::new(ErrorCode::StoreError, "Cache lock poisoned"))?;
            *cache = Some(Zeroizing::new(passphrase.to_string()));
        }

        Ok(())
    }

    /// Lock the store: zeroize the cached identity and passphrase.
    ///
    /// Idempotent — safe to call when already locked.
    pub fn lock(&self) {
        if let Ok(mut cache) = self.cached_identity.write() {
            *cache = None;
        }
        if let Ok(mut cache) = self.cached_passphrase.write() {
            *cache = None;
        }
    }

    /// Step 1 of two-step setup: clone the repository and save repo config.
    ///
    /// Does **not** save the age identity — that is done via
    /// [`save_identity`](Store::save_identity). Clears any existing
    /// configuration before cloning.
    ///
    /// # Errors
    ///
    /// Returns an error if the clone fails or the config cannot be persisted.
    pub fn clone_only(
        &self,
        repo_url: &str,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
    ) -> Result<(), Error> {
        let auth = match (ssh_key, pat) {
            (Some(key), _) => git::GitAuth::Ssh {
                username: "git".to_string(),
                private_key: key.to_string(),
                passphrase: ssh_passphrase.map(String::from),
            },
            (_, Some(token)) => git::GitAuth::Pat(token.to_string()),
            _ => git::GitAuth::None,
        };

        let repo_dir = self.config.config_dir().join("repo");
        self.config.clear_all()?;

        if repo_dir.exists() {
            std::fs::remove_dir_all(&repo_dir)?;
        }

        git::clone_repo(repo_url, &repo_dir, &auth)?;

        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, pat, ssh_key, ssh_passphrase, &local_path)?;

        Ok(())
    }

    /// Read recipients from the cloned repository.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo is not configured or the recipients file
    /// cannot be read.
    pub fn list_recipients(&self) -> Result<Vec<Recipient>, Error> {
        let repo_config = self.config.load_repo_config()?;
        let repo_path = Path::new(&repo_config.local_path);
        recipient::list_recipients(repo_path)
    }

    /// Step 2 of two-step setup: save the age identity.
    ///
    /// If `passphrase` is provided, the identity is encrypted before storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity format is invalid, the identity does
    /// not match any recipient, or the config cannot be persisted.
    pub fn save_identity(
        &self,
        identity: &str,
        passphrase: Option<&str>,
        ssh_passphrase: Option<&str>,
    ) -> Result<(), Error> {
        let identity_bytes = identity.trim().as_bytes();
        let trimmed = identity.trim();
        if !trimmed.starts_with("AGE-SECRET-KEY-")
            && !trimmed.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
            && !trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----")
        {
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key",
            ));
        }

        let derived_recipient = recipient::identity_to_recipient(identity, ssh_passphrase)?;

        let known_recipients = self.list_recipients().unwrap_or_default();
        if !known_recipients.is_empty() {
            let matches = known_recipients
                .iter()
                .any(|r| r.public_key == derived_recipient);
            if !matches {
                return Err(Error::new(
                    ErrorCode::InvalidIdentity,
                    "Identity does not match any recipient in the repository",
                ));
            }
        }

        self.config.save_identity(identity_bytes, passphrase)?;
        Ok(())
    }

    /// Configure the store: validate identity, clone repo, save config.
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
        identity_passphrase: Option<&str>,
    ) -> Result<(), Error> {
        let identity_bytes = identity.trim().as_bytes();
        let trimmed = identity.trim();
        if !trimmed.starts_with("AGE-SECRET-KEY-")
            && !trimmed.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
            && !trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----")
        {
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key",
            ));
        }

        // Validate identity can derive a recipient (verifies key is usable)
        let _ = recipient::identity_to_recipient(identity, identity_passphrase)?;

        let auth = match (ssh_key, pat) {
            (Some(key), _) => git::GitAuth::Ssh {
                username: "git".to_string(),
                private_key: key.to_string(),
                passphrase: ssh_passphrase.map(String::from),
            },
            (_, Some(token)) => git::GitAuth::Pat(token.to_string()),
            _ => git::GitAuth::None,
        };

        let repo_dir = self.config.config_dir().join("repo");
        self.config.clear_all()?;

        if repo_dir.exists() {
            std::fs::remove_dir_all(&repo_dir)?;
        }

        self.config.save_identity(identity_bytes, None)?;

        git::clone_repo(repo_url, &repo_dir, &auth)?;

        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, pat, ssh_key, ssh_passphrase, &local_path)?;

        Ok(())
    }

    /// List all `.age` entries in the configured repository.
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
    /// If the identity is encrypted, uses the cached (unlocked) identity.
    /// If the identity is plaintext, loads directly from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry does not exist, the identity is missing,
    /// the identity is encrypted but not unlocked, or decryption fails.
    pub fn get(&self, name: &str) -> Result<Secret, Error> {
        let repo_config = self.config.load_repo_config()?;
        let repo_path = Path::new(&repo_config.local_path);

        let entry_path = if Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("age"))
        {
            name.to_string()
        } else {
            format!("{name}.age")
        };

        let file_path = resolve_entry_path(repo_path, &entry_path)?;
        let identity_bytes = self.get_identity_bytes()?;
        let passphrase = self.get_cached_passphrase();
        let decrypted = crypto::decrypt_file(&file_path, &identity_bytes, passphrase.as_deref())?;
        Secret::parse(&decrypted)
    }

    /// Get identity bytes for decryption.
    ///
    /// Checks cache first (for encrypted identities that have been unlocked),
    /// then falls back to loading from disk (for plaintext identities).
    fn get_identity_bytes(&self) -> Result<Vec<u8>, Error> {
        // Check cache first
        if let Ok(cache) = self.cached_identity.read() {
            if let Some(ref cached) = *cache {
                return Ok((**cached).clone());
            }
        }

        // Load from disk
        let raw_bytes = self.config.load_identity()?;

        if classify_identity(&raw_bytes) == IdentityType::AgeEncrypted {
            return Err(Error::new(
                ErrorCode::IdentityEncrypted,
                "Identity is encrypted — unlock with passphrase first",
            ));
        }

        Ok(raw_bytes)
    }

    /// Get the cached passphrase, if any.
    ///
    /// Returns `None` if the store has not been unlocked or has been locked.
    fn get_cached_passphrase(&self) -> Option<String> {
        self.cached_passphrase
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|p| (**p).clone()))
    }

    /// Set a passphrase on an existing plaintext identity.
    ///
    /// Encrypts the current identity file in place. Rejects empty passphrase.
    ///
    /// # Errors
    ///
    /// Returns `IdentityNotEncrypted` if passphrase is empty.
    /// Returns `IdentityEncrypted` if identity is already encrypted.
    pub fn set_passphrase(&self, passphrase: &str) -> Result<(), Error> {
        if passphrase.is_empty() {
            return Err(Error::new(
                ErrorCode::IdentityNotEncrypted,
                "Passphrase must not be empty",
            ));
        }

        let raw_bytes = self.config.load_identity()?;

        if classify_identity(&raw_bytes) == IdentityType::AgeEncrypted {
            return Err(Error::new(
                ErrorCode::IdentityEncrypted,
                "Identity is already encrypted — use change_passphrase instead",
            ));
        }

        self.config.save_identity(&raw_bytes, Some(passphrase))?;
        Ok(())
    }

    /// Change the passphrase on an encrypted identity.
    ///
    /// Decrypts with the old passphrase, re-encrypts with the new one.
    /// Both old and new must be non-empty.
    ///
    /// # Errors
    ///
    /// Returns `IdentityNotEncrypted` if either passphrase is empty or identity is not encrypted.
    /// Returns `WrongPassphrase` if old passphrase is incorrect.
    pub fn change_passphrase(
        &self,
        old_passphrase: &str,
        new_passphrase: &str,
    ) -> Result<(), Error> {
        if old_passphrase.is_empty() || new_passphrase.is_empty() {
            return Err(Error::new(
                ErrorCode::IdentityNotEncrypted,
                "Passphrase must not be empty",
            ));
        }

        let encrypted_bytes = self.config.load_identity()?;

        if classify_identity(&encrypted_bytes) != IdentityType::AgeEncrypted {
            return Err(Error::new(
                ErrorCode::IdentityNotEncrypted,
                "Identity is not encrypted — use set_passphrase instead",
            ));
        }

        let plaintext = crypto::decrypt_identity(old_passphrase, &encrypted_bytes)?;
        self.config
            .save_identity(&plaintext, Some(new_passphrase))?;
        self.lock();
        Ok(())
    }

    /// Pull latest changes from the remote (fast-forward only).
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

    /// Reset all configuration and local data. Clears the identity cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be removed.
    pub fn reset(&self) -> Result<(), Error> {
        self.lock();

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
        .filter(|e| !e.path().components().any(|c| c.as_os_str() == ".git"))
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

    #[test]
    fn resolve_entry_path_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("cloud");
        fs::create_dir_all(&file_path).unwrap();
        fs::write(file_path.join("aws.age"), b"encrypted").unwrap();

        let result = resolve_entry_path(dir.path(), "cloud/aws.age");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), dir.path().join("cloud/aws.age"));
    }

    #[test]
    fn resolve_entry_path_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_entry_path(dir.path(), "nonexistent.age");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn resolve_entry_path_traversal_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_entry_path(dir.path(), "../../../etc/passwd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn resolve_entry_path_traversal_deep() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_entry_path(dir.path(), "foo/../../bar/../../../etc");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENTRY_NOT_FOUND");
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
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
        assert!(err.message.contains("outside repository"));
    }

    #[test]
    fn list_entries_nonexistent_dir() {
        let missing = PathBuf::from("/tmp/gpm_no_such_dir_12345");
        assert!(!missing.exists());
        let result = list_entries(&missing);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "NO_REPO");
    }

    // ── unlock/lock tests ──────────────────────────────────────────────

    #[test]
    fn lock_clears_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        assert!(!store.is_unlocked());
        store.lock();
        assert!(!store.is_unlocked());
    }

    #[test]
    fn unlock_caches_passphrase_for_plaintext_identity() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(b"AGE-SECRET-KEY-1TEST", None).unwrap();

        let store = Store::new(dir.path().to_path_buf());
        // unlock() now succeeds for plaintext identities — caches passphrase
        // for encrypted SSH key decryption (no-op for x25519, harmless)
        store.unlock("passphrase").unwrap();
        // cached_identity should NOT be populated (identity is not age-encrypted)
        assert!(!store.is_unlocked());
    }

    #[test]
    fn is_identity_encrypted_false_for_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(b"AGE-SECRET-KEY-1TEST", None).unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(!store.is_identity_encrypted());
    }

    #[test]
    fn is_identity_encrypted_true_after_encrypted_save() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("pass123"))
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(store.is_identity_encrypted());
    }

    #[test]
    fn is_identity_encrypted_true_for_encrypted_ssh_key() {
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(encrypted_ssh_key, None).unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(store.is_identity_encrypted());
    }

    #[test]
    fn is_identity_encrypted_false_for_unencrypted_ssh_key() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(unencrypted_ssh_key, None).unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(!store.is_identity_encrypted());
    }

    #[test]
    fn set_passphrase_rejects_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(b"AGE-SECRET-KEY-1TEST", None).unwrap();

        let store = Store::new(dir.path().to_path_buf());
        let err = store.set_passphrase("").unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    #[test]
    fn set_passphrase_rejects_already_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("old"))
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        let err = store.set_passphrase("new").unwrap_err();
        assert_eq!(err.code, "IDENTITY_ENCRYPTED");
    }

    #[test]
    fn change_passphrase_rejects_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("old"))
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert_eq!(
            store.change_passphrase("", "new").unwrap_err().code,
            "IDENTITY_NOT_ENCRYPTED"
        );
        assert_eq!(
            store.change_passphrase("old", "").unwrap_err().code,
            "IDENTITY_NOT_ENCRYPTED"
        );
    }
}
