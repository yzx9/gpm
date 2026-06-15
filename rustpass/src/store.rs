// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::{fmt, str};

use age::ssh;
use serde::Serialize;
use tokio::fs;
use tokio::task::spawn_blocking;
use walkdir::WalkDir;
use zeroize::Zeroizing;

use crate::config::{Config, RepoConfig};
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::identity::{IdentityType, classify_identity, validate_identity_format};
use crate::recipient::{self, Recipient};
use crate::secret::Secret;
use crate::signing::{
    self, AuthenticityConfig, CommitSigInfo, CommitSigStatus, TrustedKey, VerifyMode,
};
use crate::{crypto, git};

/// Default auto-lock timeout in seconds (5 minutes).
pub const DEFAULT_LOCK_TIMEOUT_SECS: u64 = 300;

/// Result of a sync (pull) operation — aligned with gopass `Store.Sync`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    /// Whether any new commits were pulled (HEAD advanced).
    pub changed: bool,
    /// Short hash (7 chars) of the (current) HEAD commit.
    pub head: String,
    /// Repository-authenticity outcome of this pull.
    pub authenticity: AuthenticityResult,
}

/// Authenticity outcome of a sync (pull) — surfaced to the frontend so it can
/// pop the Audit mismatch modal / Enforce-block modal without re-verifying.
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticityResult {
    /// The mode in force during this pull.
    pub mode: VerifyMode,
    /// Commits in the pulled range `(old HEAD, new HEAD]`, newest first.
    /// Empty for `VerifyMode::Off` (no verification is done).
    pub new_commits: Vec<CommitSigInfo>,
    /// Subset of `new_commits` that are non-Verified and not ignored — the
    /// actionable issues.
    pub open_issues: Vec<CommitSigInfo>,
    /// `true` only when Enforce refused checkout (HEAD did not advance).
    pub blocked: bool,
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

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    pub async fn is_identity_encrypted(&self) -> bool {
        let Ok(bytes) = self.config.load_identity().await else {
            return false;
        };
        let itype = classify_identity(&bytes);

        if itype == IdentityType::AgeEncrypted {
            return true;
        }

        if matches!(itype, IdentityType::SshEd25519 | IdentityType::SshRsa) {
            let Ok(text) = str::from_utf8(&bytes) else {
                return false;
            };
            let buf = BufReader::new(text.trim().as_bytes());
            return matches!(
                ssh::Identity::from_buffer(buf, None),
                Ok(ssh::Identity::Encrypted(_))
            );
        }

        false
    }

    /// Get the type of the stored identity.
    ///
    /// Returns [`IdentityType::Unknown`] if no identity is configured.
    pub async fn identity_type(&self) -> IdentityType {
        match self.config.load_identity().await {
            Ok(bytes) => classify_identity(&bytes),
            Err(_) => IdentityType::Unknown,
        }
    }

    /// Check if the identity cache is populated (identity is unlocked).
    ///
    /// Returns `true` if either cache holds an unlock:
    /// - `cached_identity` is populated for age-encrypted identities (the
    ///   decrypted x25519 key — the passphrase is no longer needed).
    /// - `cached_passphrase` is populated for SSH identities (there is no
    ///   decrypted blob to cache; age re-decrypts the SSH key with the
    ///   passphrase on every entry access, so the cached passphrase *is* the
    ///   unlock state).
    ///
    /// Before `48f5d7c` stopped age-encrypting SSH keys, SSH unlock populated
    /// `cached_identity` and this checked only that. SSH now populates only
    /// `cached_passphrase`, so checking both is required for SSH unlock to be
    /// recognized.
    #[must_use]
    pub fn is_unlocked(&self) -> bool {
        self.cached_identity
            .read()
            .is_ok_and(|guard| guard.is_some())
            || self
                .cached_passphrase
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
    pub async fn unlock(&self, passphrase: &str) -> Result<(), Error> {
        let encrypted_bytes = self.config.load_identity().await?;

        let itype = classify_identity(&encrypted_bytes);

        if itype == IdentityType::AgeEncrypted {
            // Age-encrypted identity: decrypt with passphrase on blocking thread
            // (scrypt is intentionally slow ~100ms+)
            let pw = passphrase.to_string();
            let decrypted =
                spawn_blocking(move || crypto::decrypt_identity(&pw, &encrypted_bytes)).await??;
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

    /// Validate a passphrase against the stored identity WITHOUT caching it.
    ///
    /// Used by the biometric enable flow to reject a wrong passphrase before
    /// sealing it (plan D4). For age-encrypted identities this runs the scrypt
    /// decrypt; for encrypted SSH keys it decrypts the key; for plaintext or
    /// unencrypted identities it is a no-op success.
    ///
    /// # Errors
    ///
    /// Returns `WrongPassphrase` if the passphrase is incorrect for an
    /// age-encrypted identity or an encrypted SSH key.
    pub async fn validate_passphrase(&self, passphrase: &str) -> Result<(), Error> {
        let bytes = self.config.load_identity().await?;
        let itype = classify_identity(&bytes);

        match itype {
            IdentityType::AgeEncrypted => {
                let pw = passphrase.to_string();
                spawn_blocking(move || crypto::decrypt_identity(&pw, &bytes)).await??;
            }
            IdentityType::SshEd25519 | IdentityType::SshRsa => {
                let pw = passphrase.to_string();
                spawn_blocking(move || crypto::validate_ssh_key_passphrase(&bytes, &pw)).await??;
            }
            _ => {}
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

    /// Clone the repository and save repo config.
    ///
    /// Does **not** save the age identity — that is done via
    /// [`save_identity`](Store::save_identity). Clears any existing
    /// configuration before cloning.
    ///
    /// # Errors
    ///
    /// Returns an error if the clone fails or the config cannot be persisted.
    pub async fn clone_only(
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
        self.config.clear_all().await?;

        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }

        let repo_url_owned = repo_url.to_string();
        let repo_dir_clone = repo_dir.clone();
        spawn_blocking(move || git::clone_repo(&repo_url_owned, &repo_dir_clone, &auth)).await??;

        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, pat, ssh_key, ssh_passphrase, &local_path)
            .await?;

        Ok(())
    }

    /// Read recipients from the cloned repository.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo is not configured or the recipients file
    /// cannot be read.
    pub async fn list_recipients(&self) -> Result<Vec<Recipient>, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path);
        recipient::list_recipients(repo_path).await
    }

    /// Save the age identity.
    ///
    /// The single `passphrase` is used differently based on identity type:
    /// - **x25519**: optionally encrypts the identity at rest (like `age -p`).
    ///   `None` stores it in plaintext.
    /// - **SSH key**: decrypts the SSH private key for recipient derivation
    ///   (required if the key is passphrase-protected). SSH keys are stored
    ///   as-is and never re-encrypted by gpm — they rely on the SSH key's
    ///   native passphrase protection, matching age's design.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity format is invalid, the identity does
    /// not match any recipient, or the config cannot be persisted.
    pub async fn save_identity(
        &self,
        identity: &str,
        passphrase: Option<&str>,
    ) -> Result<(), Error> {
        let identity_bytes = identity.trim().as_bytes();
        validate_identity_format(identity_bytes)?;

        let itype = classify_identity(identity_bytes);

        // SSH keys need the passphrase to decrypt the private key for recipient
        // derivation; native x25519 keys are never passphrase-protected.
        let recipient_passphrase = match itype {
            IdentityType::SshEd25519 | IdentityType::SshRsa => passphrase,
            _ => None,
        };
        let derived_recipient = recipient::identity_to_recipient(identity, recipient_passphrase)?;

        let known_recipients = self.list_recipients().await.unwrap_or_default();
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

        // Only native x25519 keys support optional at-rest encryption; SSH keys
        // are stored as-is.
        let storage_passphrase = match itype {
            IdentityType::SshEd25519 | IdentityType::SshRsa => None,
            _ => passphrase,
        };
        self.config
            .save_identity(identity_bytes, storage_passphrase)
            .await?;
        Ok(())
    }

    /// Configure the store: validate identity, clone repo, save config.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity format is invalid, the clone fails,
    /// or the config cannot be persisted.
    pub async fn configure(
        &self,
        repo_url: &str,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
        identity: &str,
        identity_passphrase: Option<&str>,
    ) -> Result<(), Error> {
        let identity_bytes = identity.trim().as_bytes();
        validate_identity_format(identity_bytes)?;

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
        self.config.clear_all().await?;

        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }

        self.config.save_identity(identity_bytes, None).await?;

        let repo_url_owned = repo_url.to_string();
        let repo_dir_clone = repo_dir.clone();
        spawn_blocking(move || git::clone_repo(&repo_url_owned, &repo_dir_clone, &auth)).await??;

        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, pat, ssh_key, ssh_passphrase, &local_path)
            .await?;

        Ok(())
    }

    /// List all `.age` entries in the configured repository.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the repo path
    /// does not exist.
    pub async fn list(&self) -> Result<Vec<Entry>, Error> {
        let repo_config = self.config.load_repo_config().await?;
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
    pub async fn get(&self, name: &str) -> Result<Secret, Error> {
        let repo_config = self.config.load_repo_config().await?;
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
        let identity_bytes = self.get_identity_bytes().await?;
        let passphrase = self.get_cached_passphrase();
        let decrypted =
            crypto::decrypt_file(&file_path, &identity_bytes, passphrase.as_deref()).await?;
        Secret::parse(&decrypted)
    }

    /// Get identity bytes for decryption.
    ///
    /// Checks cache first (for encrypted identities that have been unlocked),
    /// then falls back to loading from disk (for plaintext identities).
    async fn get_identity_bytes(&self) -> Result<Vec<u8>, Error> {
        // Check cache first
        if let Ok(cache) = self.cached_identity.read()
            && let Some(ref cached) = *cache
        {
            return Ok((**cached).clone());
        }

        // Load from disk
        let raw_bytes = self.config.load_identity().await?;

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
    /// Only native x25519 keys support at-rest encryption; SSH keys are
    /// rejected (they rely on their own native passphrase protection).
    ///
    /// # Errors
    ///
    /// Returns `IdentityNotEncrypted` if passphrase is empty or the identity
    /// is an SSH key (not encrypted by gpm).
    /// Returns `IdentityEncrypted` if identity is already encrypted.
    pub async fn set_passphrase(&self, passphrase: &str) -> Result<(), Error> {
        if passphrase.is_empty() {
            return Err(Error::new(
                ErrorCode::IdentityNotEncrypted,
                "Passphrase must not be empty",
            ));
        }

        let raw_bytes = self.config.load_identity().await?;

        match classify_identity(&raw_bytes) {
            IdentityType::AgeEncrypted => {
                return Err(Error::new(
                    ErrorCode::IdentityEncrypted,
                    "Identity is already encrypted — use change_passphrase instead",
                ));
            }
            IdentityType::SshEd25519 | IdentityType::SshRsa => {
                return Err(Error::new(
                    ErrorCode::IdentityNotEncrypted,
                    "SSH keys are not encrypted by gpm; use the SSH key's native passphrase",
                ));
            }
            _ => {}
        }

        self.config
            .save_identity(&raw_bytes, Some(passphrase))
            .await?;
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
    pub async fn change_passphrase(
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

        let encrypted_bytes = self.config.load_identity().await?;

        if classify_identity(&encrypted_bytes) != IdentityType::AgeEncrypted {
            return Err(Error::new(
                ErrorCode::IdentityNotEncrypted,
                "Identity is not encrypted — use set_passphrase instead",
            ));
        }

        // scrypt is intentionally slow (~100ms+), run on blocking thread
        let pw = old_passphrase.to_string();
        let plaintext =
            spawn_blocking(move || crypto::decrypt_identity(&pw, &encrypted_bytes)).await??;
        self.config
            .save_identity(&plaintext, Some(new_passphrase))
            .await?;
        self.lock();
        Ok(())
    }

    /// Pull latest changes from the remote (fast-forward only).
    ///
    /// Applies repository-authenticity verification (per the stored
    /// [`AuthenticityConfig`]) before checkout: in Audit mode issues are
    /// reported without blocking, in Enforce mode a blocking issue aborts the
    /// pull leaving HEAD unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured, the remote is
    /// unreachable, the branches have diverged, or Enforce mode refuses the
    /// pull.
    pub async fn sync(&self) -> Result<SyncResult, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();
        let policy = repo_config.authenticity;
        spawn_blocking(move || git::pull_repo(&repo_path, &auth, &policy)).await?
    }

    // ── Repository authenticity ───────────────────────────────────────────

    /// The configured repo path, or an error if not configured.
    async fn repo_path(&self) -> Result<PathBuf, Error> {
        let repo_config = self.config.load_repo_config().await?;
        Ok(Path::new(&repo_config.local_path).to_path_buf())
    }

    /// Load the persisted authenticity config (the `authenticity` field of
    /// `repo.json`). Defaults to Off / empty when the repo isn't configured
    /// yet — pre-setup there is nothing to verify.
    ///
    /// # Errors
    ///
    /// Returns an error if `repo.json` exists but cannot be read or parsed.
    pub async fn authenticity_config(&self) -> Result<AuthenticityConfig, Error> {
        match self.config.load_repo_config().await {
            Ok(rc) => Ok(rc.authenticity),
            // No repo configured yet → authenticity is trivially Off.
            Err(e) if e.code == "NO_REPO" => Ok(AuthenticityConfig::default()),
            Err(e) => Err(e),
        }
    }

    /// Set the verification mode. Refuses [`VerifyMode::Enforce`] when no
    /// trusted key is recorded yet (Enforce with zero keys would block every
    /// pull). Returns the effective stored mode.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ConfigError`] if Enforce is requested with no
    /// trusted keys, or the config cannot be persisted.
    pub async fn set_verification_mode(&self, mode: VerifyMode) -> Result<VerifyMode, Error> {
        let mut rc = self.config.load_repo_config().await?;
        if mode == VerifyMode::Enforce && rc.authenticity.trusted_keys.is_empty() {
            return Err(Error::new(
                ErrorCode::ConfigError,
                "Add a trusted signing key before enabling Enforce.",
            ));
        }
        rc.authenticity.mode = mode;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc.authenticity.mode)
    }

    /// Add a trusted signing public key. Validates the key, derives its
    /// fingerprint, and dedupes — if a key with the same fingerprint is already
    /// trusted, the existing entry is returned unchanged (idempotent).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::SshKeyInvalid`] if the public key is not a
    /// parseable OpenSSH key, or the config cannot be persisted.
    pub async fn add_trusted_key(
        &self,
        public_key: &str,
        label: &str,
    ) -> Result<TrustedKey, Error> {
        let fingerprint = signing::fingerprint_of_public_key(public_key)?;

        let mut rc = self.config.load_repo_config().await?;
        if let Some(existing) = rc
            .authenticity
            .trusted_keys
            .iter()
            .find(|k| k.fingerprint == fingerprint)
            .cloned()
        {
            return Ok(existing);
        }

        let head = self.current_head_hash().await.unwrap_or_default();
        let key = TrustedKey {
            public_key: public_key.trim().to_string(),
            fingerprint,
            label: label.to_string(),
            added_at_commit: head,
        };
        rc.authenticity.trusted_keys.push(key.clone());
        self.config.save_repo_config_full(&rc).await?;
        Ok(key)
    }

    /// Remove a trusted signing key by fingerprint. Removing the last key
    /// while in Enforce downgrades to Audit (Enforce with zero keys would
    /// block everything).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be persisted.
    pub async fn remove_trusted_key(&self, fingerprint: &str) -> Result<(), Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.authenticity
            .trusted_keys
            .retain(|k| k.fingerprint != fingerprint);
        if rc.authenticity.trusted_keys.is_empty() && rc.authenticity.mode == VerifyMode::Enforce {
            rc.authenticity.mode = VerifyMode::Audit;
        }
        self.config.save_repo_config_full(&rc).await
    }

    /// Record a per-commit ignore, scoped to this commit + its **current**
    /// status. The status is recomputed server-side (the caller passes only the
    /// hash), so the recorded `IgnoredIssue.status` always matches what
    /// `verify_range` will later compute — keeping the per-status ignore match
    /// stable. Idempotent.
    ///
    /// No-op (still Ok) for a commit whose status is not an issue (e.g.
    /// `Verified`) — there is nothing to ignore.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit hash is invalid, the repo cannot be
    /// opened, or the config cannot be persisted.
    pub async fn ignore_commit_issue(&self, commit: &str) -> Result<(), Error> {
        let repo_path = self.repo_path().await?;
        let mut rc = self.config.load_repo_config().await?;
        let trusted = signing::trusted_fingerprints(&rc.authenticity);

        // Recompute the commit's current status so the recorded ignore matches
        // what verification will see later.
        let commit_owned = commit.to_string();
        let status = spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            let oid = git2::Oid::from_str(&commit_owned)?;
            signing::status_of_commit(&repo, oid, &trusted)
        })
        .await??;

        // Nothing to ignore for a non-issue.
        if !status.is_issue() {
            return Ok(());
        }

        let already = rc
            .authenticity
            .ignored
            .iter()
            .any(|i| i.commit == commit && i.status == status);
        if !already {
            let head = self.current_head_hash().await.unwrap_or_default();
            rc.authenticity.ignored.push(signing::IgnoredIssue {
                commit: commit.to_string(),
                status,
                ignored_at_commit: head,
            });
            self.config.save_repo_config_full(&rc).await?;
        }
        Ok(())
    }

    /// The verification status of the current HEAD commit (cheap; cached
    /// config, single commit verify). Used by the indicator badge.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn head_signature_status(&self) -> Result<CommitSigStatus, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::trusted_fingerprints(&rc.authenticity);
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            signing::head_status(&repo, &trusted)
        })
        .await?
    }

    /// The OpenSSH public key of HEAD's SSH-signature signer (for the
    /// "trust this signer" TOFU flow), or `None` if HEAD is unsigned or not
    /// SSH-signed.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn head_signer_public_key(&self) -> Result<Option<String>, Error> {
        let repo_path = self.repo_path().await?;
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            signing::head_signer_public_key(&repo)
        })
        .await?
    }

    /// Trust the SSH-signature signer of a specific commit ("trust this
    /// signer" TOFU from the history detail view). Errors if the commit is
    /// unsigned or not SSH-signed.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::SshKeyInvalid`] if the commit has no SSH signer,
    /// or [`ErrorCode::SshKeyInvalid`] if the public key is invalid.
    pub async fn trust_commit_signer(
        &self,
        commit_hash: &str,
        label: &str,
    ) -> Result<TrustedKey, Error> {
        let repo_path = self.repo_path().await?;
        let hash_owned = commit_hash.to_string();
        let public_key = spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            let oid = repo.revparse_single(&hash_owned)?.id();
            signing::signer_public_key(&repo, oid)
        })
        .await??;
        let public_key = public_key.ok_or_else(|| {
            Error::new(
                ErrorCode::SshKeyInvalid,
                "This commit is not signed by an SSH key — nothing to trust.",
            )
        })?;
        self.add_trusted_key(&public_key, label).await
    }

    /// The full hash of the current HEAD commit, for provenance fields.
    async fn current_head_hash(&self) -> Result<String, Error> {
        let repo_path = self.repo_path().await?;
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            let head = repo
                .head()?
                .target()
                .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD commit"))?;
            Ok(head.to_string())
        })
        .await?
    }

    /// Verify every commit in the half-open range `(from, to]` (newest first)
    /// against the trusted set + ignore list.
    ///
    /// # Errors
    ///
    /// Returns an error if the hashes are invalid, the repo cannot be opened,
    /// or the walk fails.
    pub async fn verify_range(&self, from: &str, to: &str) -> Result<Vec<CommitSigInfo>, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::trusted_fingerprints(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        let from_owned = from.to_string();
        let to_owned = to.to_string();
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            let from = git2::Oid::from_str(&from_owned)?;
            let to = git2::Oid::from_str(&to_owned)?;
            signing::verify_range(&repo, from, to, &trusted, &ignored)
        })
        .await?
    }

    /// The `limit` most recent commits (HEAD and ancestors, newest first) with
    /// per-commit verification status. Used by the `/history` screen.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn list_commit_signatures(&self, limit: usize) -> Result<Vec<CommitSigInfo>, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::trusted_fingerprints(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            signing::list_commit_signatures(&repo, limit, &trusted, &ignored)
        })
        .await?
    }

    /// A single commit's metadata + verification status (the `/history` detail
    /// sheet). `commit_hash` may be a full or short hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the hash is invalid, the commit cannot be found,
    /// or its signature cannot be read.
    pub async fn commit_signature(&self, commit_hash: &str) -> Result<CommitSigInfo, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::trusted_fingerprints(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        let hash_owned = commit_hash.to_string();
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)?;
            let oid = repo.revparse_single(&hash_owned)?.id();
            signing::commit_sig_info(&repo, oid, &trusted, &ignored)
        })
        .await?
    }

    /// Reset all configuration and local data. Clears the identity cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be removed.
    pub async fn reset(&self) -> Result<(), Error> {
        self.lock();

        if let Ok(repo_config) = self.config.load_repo_config().await {
            let repo_path = Path::new(&repo_config.local_path);
            if repo_path.exists() {
                fs::remove_dir_all(repo_path).await?;
            }
        }
        self.config.clear_all().await
    }

    /// Get the current repository configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured.
    pub async fn config(&self) -> Result<RepoConfig, Error> {
        self.config.load_repo_config().await
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

    #[tokio::test]
    async fn unlock_caches_passphrase_for_plaintext_identity() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        // unlock() succeeds for plaintext identities and caches the passphrase.
        // In production unlock() is never called on a plaintext identity (the
        // router only routes to /unlock when is_identity_encrypted()), so this
        // edge case is harmless. With is_unlocked() consulting both caches,
        // the cached passphrase now trivially marks the store unlocked here.
        store.unlock("passphrase").await.unwrap();
        // cached_identity should NOT be populated (identity is not age-encrypted)
        assert!(
            store.cached_identity.read().is_ok_and(|g| g.is_none()),
            "plaintext identity must not populate the decrypted-identity cache"
        );
        // ...but the cached passphrase marks the store as unlocked.
        assert!(
            store.is_unlocked(),
            "unlock() caching the passphrase must mark the store unlocked"
        );
    }

    #[tokio::test]
    async fn unlock_marks_ssh_identity_unlocked() {
        // Regression for the 48f5d7c SSH-unlock recognition bug: an encrypted
        // SSH identity populates only cached_passphrase (no decrypted blob to
        // cache), so is_unlocked() must consult cached_passphrase too.
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(
            !store.is_unlocked(),
            "store must start locked for an encrypted SSH identity"
        );

        store.unlock("test-passphrase").await.unwrap();

        // The decrypted-identity cache stays empty for SSH (there is no
        // decrypted blob to cache); only the passphrase is cached.
        assert!(
            store.cached_identity.read().is_ok_and(|g| g.is_none()),
            "SSH unlock must not populate the decrypted-identity cache"
        );
        assert!(
            store.is_unlocked(),
            "an encrypted SSH identity must be recognised as unlocked after unlock()"
        );
    }

    #[tokio::test]
    async fn is_identity_encrypted_false_for_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(!store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn is_identity_encrypted_true_after_encrypted_save() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("pass123"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn is_identity_encrypted_true_for_encrypted_ssh_key() {
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn is_identity_encrypted_false_for_unencrypted_ssh_key() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(unencrypted_ssh_key, None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert!(!store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn save_identity_stores_ssh_key_as_plaintext_even_with_passphrase() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());

        // Even when a passphrase is supplied, SSH keys are stored as-is — gpm
        // never re-encrypts them (they rely on their own native protection),
        // matching age's design.
        store
            .save_identity(
                str::from_utf8(unencrypted_ssh_key).unwrap(),
                Some("would-be-storage-pass"),
            )
            .await
            .expect("save_identity should succeed for SSH key");

        assert!(
            !store.is_identity_encrypted().await,
            "SSH key must be stored as plaintext, not age-encrypted"
        );
        assert_eq!(
            store.identity_type().await,
            IdentityType::SshEd25519,
            "stored identity should still be an SSH key, not an age-encrypted blob"
        );
    }

    #[tokio::test]
    async fn set_passphrase_rejects_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        let err = store.set_passphrase("").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    #[tokio::test]
    async fn set_passphrase_rejects_already_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("old"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        let err = store.set_passphrase("new").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_ENCRYPTED");
    }

    #[tokio::test]
    async fn set_passphrase_rejects_ssh_key() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(unencrypted_ssh_key, None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        let err = store.set_passphrase("new").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    #[tokio::test]
    async fn change_passphrase_rejects_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("old"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        assert_eq!(
            store.change_passphrase("", "new").await.unwrap_err().code,
            "IDENTITY_NOT_ENCRYPTED"
        );
        assert_eq!(
            store.change_passphrase("old", "").await.unwrap_err().code,
            "IDENTITY_NOT_ENCRYPTED"
        );
    }

    // ── validate_passphrase (biometric enable D4) ───────────────────────

    #[tokio::test]
    async fn validate_passphrase_accepts_correct_ssh_passphrase() {
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf());
        store
            .validate_passphrase("test-passphrase")
            .await
            .expect("correct SSH passphrase must validate");
    }

    #[tokio::test]
    async fn validate_passphrase_rejects_wrong_ssh_passphrase() {
        // D4: enabling biometric with a wrong SSH passphrase must fail before
        // the passphrase is sealed into the Keystore.
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf());
        let err = store
            .validate_passphrase("wrong-passphrase")
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "WRONG_PASSPHRASE",
            "wrong SSH passphrase must be rejected as WRONG_PASSPHRASE"
        );
    }

    #[tokio::test]
    async fn validate_passphrase_age_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf());
        // Save an age-encrypted identity (uses a fixed test recipient).
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("correct-pw"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf());
        let err = store.validate_passphrase("nope").await.unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }
}
