// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::future::Future;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::{fmt, str};

use age::ssh;
use nucleo_matcher::{
    Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use walkdir::WalkDir;
use zeroize::Zeroizing;

use crate::config::{Config, LockMode, RepoConfig};
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::identity::{IdentityType, classify_identity, validate_identity_format};
use crate::recipient::{self, Recipient};
use crate::secret::Secret;
use crate::signing::{
    self, AuthenticityConfig, CommitSigInfo, CommitSigStatus, TrustedKey, VerifyMode,
};
use crate::{crypto, git, template};

/// Default `Idle` auto-lock timeout in seconds (5 minutes). Used as the
/// `Idle` preset's fallback and the fail-safe when the lock-mode cache can't be
/// read; not the app default (that's `LockMode::Immediate`).
pub const DEFAULT_LOCK_TIMEOUT_SECS: u64 = 300;

/// Minimum [`LockMode::Idle`] auto-lock timeout, in seconds. Below this the
/// idle timer races the user (fires before they can act).
pub const LOCK_IDLE_SECS_MIN: u64 = 30;
/// Maximum [`LockMode::Idle`] auto-lock timeout, in seconds. Above this is
/// almost certainly a unit mistake.
pub const LOCK_IDLE_SECS_MAX: u64 = 3600;

/// Minimum view/clipboard auto-clear override, in seconds. `Some(0)` (Never)
/// bypasses the range; any other override is clamped into it.
pub const CLEAR_SECS_MIN: u64 = 5;
/// Maximum view/clipboard auto-clear override, in seconds.
pub const CLEAR_SECS_MAX: u64 = 600;

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

/// Outcome of a sync (pull): a normal pull, or a divergence the caller must
/// resolve. Replaces the bare [`SyncResult`] return of [`Store::sync`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// Normal fast-forward pull (changed or not).
    FastForwarded(SyncResult),
    /// Local and remote have diverged; the working branch is unchanged. The
    /// caller must resolve via [`Store::resolve_sync_divergence`].
    Diverged(SyncDivergence),
}

/// Local-vs-remote divergence, surfaced so the user can decide whether to
/// discard local-only changes and adopt the remote. Carries no secrets — entry
/// *names* / file paths only.
#[derive(Debug, Clone, Serialize)]
pub struct SyncDivergence {
    /// Commits reachable from local HEAD but not the merge base.
    pub local_ahead: usize,
    /// Commits reachable from the remote tip but not the merge base.
    pub remote_ahead: usize,
    /// Full hash of the remote tip this preview was computed against. Passed
    /// back to [`Store::resolve_sync_divergence`] so we adopt exactly what was
    /// reviewed (no stale-confirmation TOCTOU).
    pub remote_tip: String,
    /// Secret entries (`.age` stripped) present locally, absent remotely —
    /// **deleted** by "adopt remote".
    pub local_only_entries: Vec<String>,
    /// Secret entries present on both sides whose `.age` bytes differ —
    /// **overwritten** by "adopt remote". (May over-report identical-plaintext
    /// re-encryptions until the decrypt-and-compare enhancement lands.)
    pub modified_entries: Vec<String>,
    /// Non-secret tracked files changed on the local side (templates,
    /// recipients, …) — also discarded/overwritten by a hard reset.
    pub other_changed_files: Vec<String>,
}

/// Result of a successful write (`Store::set`) — the new HEAD commit hash.
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    /// Short hash (7 chars) of the commit that recorded the write.
    pub commit: String,
}

/// The default commit author identity, surfaced to the UI so it can display
/// what's in effect without hardcoding the value.
#[derive(Debug, Clone, Serialize)]
pub struct CommitIdentity {
    /// Commit author name.
    pub name: String,
    /// Commit author email.
    pub email: String,
}

/// Outcome of a write attempt.
///
/// gopass syncs (pulls) before writing and pushes immediately after; if the
/// remote advanced in between, the push is rejected. Unlike gopass — which
/// surfaces a raw git merge conflict on the binary `.age` file — gpm detects
/// the specific *same-name* collision and offers a decrypt-aware resolution.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteOutcome {
    /// The secret was written, committed, and pushed. Carries the new HEAD.
    Written(WriteResult),
    /// The remote already has a different version of the same entry. No data
    /// was pushed; the local store was rolled back to the pre-write state. The
    /// caller must ask the user how to proceed via
    /// [`Store::resolve_write_conflict`].
    ///
    /// Note: this carries **no plaintext**. If `remote_decryptable` is true the
    /// caller can show the remote version through the existing secure `get`
    /// path (view auto-clear), never by embedding it here.
    Conflict(WriteConflict),
}

/// Description of a write-path conflict on a same-name remote entry.
#[derive(Debug, Clone, Serialize)]
pub struct WriteConflict {
    /// The entry name that collided.
    pub name: String,
    /// Whether the remote's version of this entry could be decrypted with our
    /// key. If `true` it was encrypted to us (a legitimate co-recipient) and
    /// the user may inspect it and choose freely. If `false` we cannot read it
    /// — overwriting it would destroy data we can't see, so `KeepMine` is
    /// refused (use `KeepMineForce` to override).
    pub remote_decryptable: bool,
}

/// How to resolve a [`WriteConflict`] (the user's choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    /// Overwrite the remote with our version (replay the write on the remote
    /// tip and push). Refused with `UnsafeOverwrite` when the remote version is
    /// undecryptable to us — that would silently destroy data we can't read.
    KeepMine,
    /// Like `KeepMine` but proceeds even when the remote version is
    /// undecryptable. Destructive: the caller must have explicitly confirmed.
    KeepMineForce,
    /// Discard our write and adopt the remote version as-is.
    KeepRemote,
    /// Back out: leave the local store at the pre-write state. A later `sync`
    /// will fast-forward to the remote.
    Cancel,
}

/// How to resolve a [`SyncOutcome::Diverged`] (the user's choice). "Cancel" is
/// client-side — the frontend simply doesn't call
/// [`Store::resolve_sync_divergence`] — so it is absent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceChoice {
    /// Discard local-only changes and adopt the reviewed remote tip exactly.
    AdoptRemote,
    /// Keep local changes: re-encrypt the local-only `.age` entries onto the
    /// reviewed remote tip (with the current recipient set) and push. Refused
    /// ([`ErrorCode::PushRejected`]) for an irreconcilable same-secret conflict
    /// or an undecryptable local entry — the user must adopt or cancel.
    KeepMine,
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
    /// Serializes all repo-mutating operations (writes via [`autosync_write`],
    /// pull, push, divergence resolution) so two in-flight mutations can't race
    /// the git index or let a reviewed divergence go stale vs local HEAD
    /// mid-resolution. Public mutation entry points acquire it; the orchestrator
    /// acquires it once and composes the lock-free `*_locked` inners.
    write_mu: Mutex<()>,
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

/// The signature shared by [`git::commit`] (write) and [`git::commit_removal`]
/// (delete). Used by [`Store::commit_push`] to shell the shared commit→push path
/// for both write kinds.
type CommitFn = fn(&Path, &[String], &str, Option<&str>, Option<&str>) -> Result<String, Error>;

impl Store {
    /// Create a new `Store` backed by the given config directory.
    #[must_use]
    pub fn new(config_dir: PathBuf, master_key: Option<[u8; 32]>) -> Self {
        Self {
            config: Config::new(config_dir, master_key),
            cached_identity: RwLock::new(None),
            write_mu: Mutex::new(()),
        }
    }

    /// Replace the at-rest master key at runtime. The app-launch biometric lock
    /// builds the store without the key (so `repo.json` is unreadable until the
    /// unlock prompt), injects it via this call after a successful biometric
    /// unlock, and wipes it (`None`) when the process is backgrounded. See
    /// [`Config::set_master_key`].
    pub fn set_master_key(&self, master_key: Option<[u8; 32]>) {
        self.config.set_master_key(master_key);
    }

    /// One-time migration: wrap any plaintext config files in the at-rest
    /// envelope. No-op on desktop (no master key) and for already-wrapped
    /// files. Safe to call on every startup.
    ///
    /// # Errors
    ///
    /// Returns an error if a file cannot be read, sealed/unsealed, or written.
    pub async fn migrate_at_rest(&self) -> Result<(), Error> {
        self.config.migrate_at_rest().await
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
    /// `unlock()` populates `cached_identity` for every encrypted identity type
    /// — the decrypted x25519 key (age) or the unencrypted SSH PEM (SSH) — so
    /// this is the sole unlock signal. The raw passphrase is not cached.
    /// Plaintext identities are never `unlock()`-ed, so they report `false`
    /// (they decrypt straight from disk).
    #[must_use]
    pub fn is_unlocked(&self) -> bool {
        self.cached_identity
            .read()
            .is_ok_and(|guard| guard.is_some())
    }

    /// Unlock a passphrase-encrypted identity by decrypting and caching it.
    ///
    /// Calling `unlock()` when already unlocked is idempotent (re-decrypts
    /// and overwrites the cache). For a non-encrypted (plaintext) identity this
    /// is a no-op success — in production it is never called on plaintext (the
    /// router gates `/unlock` on
    /// [`is_identity_encrypted`](Store::is_identity_encrypted)).
    ///
    /// # Errors
    ///
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
        } else if matches!(itype, IdentityType::SshEd25519 | IdentityType::SshRsa) {
            // Encrypted SSH key: decrypt once (the bcrypt KDF is blocking work)
            // and cache the UNENCRYPTED PEM, so per-entry decrypts skip the KDF
            // entirely — age parses the cached PEM as the no-KDF `Unencrypted`
            // variant instead of re-deriving the key every call.
            let pw = passphrase.to_string();
            let decrypted_pem = spawn_blocking(move || {
                let pem = str::from_utf8(&encrypted_bytes).map_err(|_| {
                    Error::new(
                        ErrorCode::InvalidIdentity,
                        "SSH identity is not valid UTF-8",
                    )
                })?;
                crate::ssh::to_unencrypted_pem(pem, &pw)
            })
            .await??;
            let identity_bytes = Zeroizing::new(decrypted_pem.as_str().as_bytes().to_vec());
            {
                let mut cache = self
                    .cached_identity
                    .write()
                    .map_err(|_| Error::new(ErrorCode::StoreError, "Cache lock poisoned"))?;
                *cache = Some(identity_bytes);
            }
        }

        Ok(())
    }

    /// Validate a passphrase against the stored identity WITHOUT caching it.
    ///
    /// Used by the biometric enable flow to reject a wrong passphrase before
    /// sealing it. For age-encrypted identities this runs the scrypt decrypt;
    /// for encrypted SSH keys it decrypts the key; for plaintext or
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

    /// Lock the store: zeroize the cached identity.
    ///
    /// Idempotent — safe to call when already locked.
    pub fn lock(&self) {
        if let Ok(mut cache) = self.cached_identity.write() {
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
        self.clone_only_with(repo_url, pat, ssh_key, ssh_passphrase, None, None)
            .await
    }

    /// Cancellable, progress-reporting variant of [`clone_only`](Store::clone_only).
    ///
    /// `cancel` aborts the in-progress clone (mapped to [`ErrorCode::Cancelled`]
    /// by `git::clone_repo`); `progress` receives transfer stats. Both are `None`
    /// on the plain [`clone_only`](Store::clone_only) path, which is used outside
    /// the user-initiated UI clone.
    ///
    /// # Errors
    ///
    /// Returns an error if the clone fails or the config cannot be persisted.
    pub async fn clone_only_with(
        &self,
        repo_url: &str,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
        cancel: Option<git::CancelToken>,
        progress: Option<git::ProgressSender>,
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
        spawn_blocking(move || {
            git::clone_repo(
                &repo_url_owned,
                &repo_dir_clone,
                &auth,
                cancel.as_ref(),
                progress.as_ref(),
            )
        })
        .await??;

        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, pat, ssh_key, ssh_passphrase, &local_path)
            .await?;

        Ok(())
    }

    /// Create a brand-new gopass-compatible age store on device.
    ///
    /// Mirrors gopass `setup`/`init`: `git init`, seed `.age-recipients` with the
    /// single `recipient`, make the no-parent "Initialized Store" commit, and —
    /// when `repo_url` is given — record an `origin` remote. This is
    /// identity-agnostic: it takes only the public `recipient`, never identity
    /// bytes, so the generated identity is persisted separately via
    /// [`save_identity`](Store::save_identity) (the create flow calls
    /// `complete_setup` afterwards).
    ///
    /// **No push.** The first push is a separate step (`Store::push`), performed
    /// only after both the repo config and the identity are durable — so the
    /// remote can never receive a store whose recipient's identity has been lost
    /// locally (the orphan-recipient hole). If no `repo_url` is given the store
    /// is local-only and never pushed.
    ///
    /// Auth (`pat`/`ssh_key`) is ignored when no `repo_url` is given, so a stray
    /// credential can never be persisted against an empty URL.
    ///
    /// On any failure after `git init`, the partial repo directory and any
    /// config are removed so the next attempt starts clean.
    ///
    /// # Errors
    ///
    /// Returns `InvalidIdentity` if `recipient` is empty, or a git/IO error if
    /// initialization, the recipients write, the commit, the remote add, or
    /// config persistence fails.
    pub async fn create_store(
        &self,
        repo_url: Option<&str>,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
        recipient: &str,
    ) -> Result<(), Error> {
        if recipient.trim().is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                "Recipient must not be empty",
            ));
        }

        // No URL → local-only store: ignore any stray auth (defensive; the
        // frontend also validates). Persisting url="" + pat would silently
        // discard the credential on every future no-op sync.
        let has_url = repo_url.is_some_and(|u| !u.trim().is_empty());
        let url = repo_url.unwrap_or("");
        let (pat, ssh_key, ssh_passphrase) = if has_url {
            (pat, ssh_key, ssh_passphrase)
        } else {
            (None, None, None)
        };

        let repo_dir = self.config.config_dir().join("repo");
        // Remove the repo dir first, then clear the config — mirroring the
        // failure-cleanup order below. If remove_dir_all fails we leave the
        // prior identity + config intact, rather than deleting the identity
        // while the old repo still sits on disk.
        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }
        self.config.clear_all().await?;

        let bootstrap = async {
            let repo_dir_init = repo_dir.clone();
            spawn_blocking(move || git::init_repo(&repo_dir_init)).await??;

            recipient::write_recipients(&repo_dir, &[recipient.to_string()]).await?;

            let message = format!("Initialized Store for {recipient}");
            let repo_dir_commit = repo_dir.clone();
            let rel_paths = vec![".age-recipients".to_string()];
            spawn_blocking(move || git::commit_initial(&repo_dir_commit, &rel_paths, &message))
                .await??;

            if has_url {
                let repo_dir_remote = repo_dir.clone();
                let url_owned = url.to_string();
                spawn_blocking(move || git::remote_add(&repo_dir_remote, "origin", &url_owned))
                    .await??;
            }

            let local_path = repo_dir.to_string_lossy().to_string();
            self.config
                .save_repo_config(url, pat, ssh_key, ssh_passphrase, &local_path)
                .await?;
            // TODO(0016-recipients-pinning): TOFU-pin the seeded recipient on first write.
            Ok::<(), Error>(())
        };

        if let Err(e) = bootstrap.await {
            // Best-effort cleanup: a partial repo dir or half-written config must
            // not leave the store looking initialized.
            let _ = fs::remove_dir_all(&repo_dir).await;
            let _ = self.config.clear_all().await;
            return Err(e);
        }
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
        // age-keygen writes # comment lines before the key; keep only the key
        // so it is parsed and stored consistently with the paste path.
        let identity = crate::identity::normalize_identity_text(identity);
        let identity_bytes = identity.as_bytes();
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
        self.configure_with(
            repo_url,
            pat,
            ssh_key,
            ssh_passphrase,
            identity,
            identity_passphrase,
            None,
            None,
        )
        .await
    }

    /// Cancellable, progress-reporting variant of [`configure`](Store::configure).
    ///
    /// `cancel` aborts the in-progress clone (mapped to [`ErrorCode::Cancelled`]);
    /// `progress` receives transfer stats. Both are `None` on the plain
    /// [`configure`](Store::configure) path.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity format is invalid, the clone fails,
    /// or the config cannot be persisted.
    #[allow(clippy::too_many_arguments)] // mirrors configure + cancel/progress hooks
    pub async fn configure_with(
        &self,
        repo_url: &str,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
        identity: &str,
        identity_passphrase: Option<&str>,
        cancel: Option<git::CancelToken>,
        progress: Option<git::ProgressSender>,
    ) -> Result<(), Error> {
        // age-keygen writes # comment lines before the key; keep only the key
        // so it is parsed and stored consistently with the paste path.
        let identity = crate::identity::normalize_identity_text(identity);
        let identity_bytes = identity.as_bytes();
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
        spawn_blocking(move || {
            git::clone_repo(
                &repo_url_owned,
                &repo_dir_clone,
                &auth,
                cancel.as_ref(),
                progress.as_ref(),
            )
        })
        .await??;

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

    /// Fuzzy-search the configured repository's entries by `query`, ranked by
    /// relevance: best match first, ties broken by `path`. An empty query
    /// returns every entry (identical to [`list`](Store::list)).
    ///
    /// Ranking is a stable strict total order — score descending, then unique
    /// `path` ascending — so paginating a fixed entry set by offset never
    /// splits a tie or reorders between requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the repo path
    /// does not exist.
    pub async fn search(&self, query: &str) -> Result<Vec<Entry>, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let q = query.to_string();
        spawn_blocking(move || search_entries_in(&repo_path, &q)).await?
    }

    /// One page of [`search`](Store::search) results: up to `limit` entries
    /// starting at `offset`, plus the **total** match count (independent of the
    /// slice). Ranking is the same stable strict total order as
    /// [`search`](Store::search), so paging a fixed entry set by offset is
    /// stable across requests — no tie is split, no entry reorders between
    /// pages. [`list_page`](Store::list_page) is this with an empty query.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the repo path
    /// does not exist.
    pub async fn search_page(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<RankedPage, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let q = query.to_string();
        spawn_blocking(move || {
            let ranked = search_entries_in(&repo_path, &q)?;
            Ok(slice_page(ranked, offset, limit))
        })
        .await?
    }

    /// One page of [`list`](Store::list) results —
    /// [`search_page`](Store::search_page) with an empty query, since an empty
    /// query ranks to the alpha-sorted full set (identical to [`list`](Store::list)).
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the repo path
    /// does not exist.
    pub async fn list_page(&self, offset: usize, limit: usize) -> Result<RankedPage, Error> {
        self.search_page("", offset, limit).await
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
        let decrypted = crypto::decrypt_file(&file_path, &identity_bytes, None).await?;
        Secret::parse(&decrypted)
    }

    /// Decrypt the **remote** (`origin` tip) version of `name`, if any — the
    /// teammate's version a write collided with, not the local (rolled-back)
    /// copy. Used by the write-conflict modal's "View existing" so the user
    /// inspects the version they'd actually overwrite (the local copy after a
    /// conflict is the pre-edit version, which is misleading to preview).
    ///
    /// Returns `Ok(None)` when there is no remote entry or it isn't decryptable
    /// by us (e.g. encrypted to a recipient set we've since been removed from).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidEntryName`] for a malformed name, or a git
    /// error from the remote fetch.
    pub async fn remote_secret(&self, name: &str) -> Result<Option<Secret>, Error> {
        validate_secret_name(name)?;
        let Some(blob) = self.fetch_remote_blob(&passfile_rel(name)).await? else {
            return Ok(None);
        };
        if !self.can_decrypt(&blob).await {
            return Ok(None);
        }
        let identity = self.get_identity_bytes().await?;
        let secret = spawn_blocking(move || {
            let plaintext = crypto::decrypt_bytes(&blob, &identity, None)?;
            Secret::parse(&plaintext)
        })
        .await??;
        Ok(Some(secret))
    }

    /// Encrypt and write a secret to the store, then commit **locally** (no
    /// sync, no push).
    ///
    /// This is gopass's `set` (write) command, local-only. The plaintext is
    /// encrypted to every recipient in the store's `.gopass-recipients` /
    /// `.age-recipients`, with our own key guaranteed to be among the encryption
    /// targets (mirroring gopass's `ensureOurKeyID`, so we can always read back
    /// what we wrote), written to `<name>.age`, and committed on the current
    /// branch. It does **not** pull or push — publishing is the caller's job.
    /// Production callers go through [`Store::autosync_write`], which wraps this
    /// in a pull → write → push and routes a rejected push to the sync-time
    /// divergence surface; calling `set` directly skips that serialization, so
    /// it is for tests and the orchestrator only.
    ///
    /// **Limitation (unchanged — see `.plans/0026-edit-base-version-aware.md`):**
    /// with no base-version check, a write built on a prior read can silently
    /// overwrite a teammate's newer same-name change. Decoupling does not fix
    /// this; `autosync_write`'s pre-write pull can fast-forward over the remote
    /// change before this local write commits on top.
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` for a malformed name, `InvalidIdentity` if no
    /// usable recipient (and our own key) can be derived, or a git error if
    /// staging or committing fails.
    pub async fn set(&self, name: &str, plaintext: &[u8]) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let passfile = self.encrypt_and_write(name, plaintext, &repo_path).await?;
        let head = self
            .commit_local(
                repo_path,
                passfile,
                format!("Save secret: {name}"),
                (
                    repo_config.commit_user_name.clone(),
                    repo_config.commit_user_email.clone(),
                ),
                git::commit,
            )
            .await?;
        Ok(WriteResult { commit: head })
    }

    /// Delete a secret: remove `<name>.age` and commit the removal **locally**
    /// (no sync, no push). The delete sibling of [`set`].
    ///
    /// Local-only, like [`set`]: no pre-sync, no push, no rollback. Publishing is
    /// the caller's job — production callers go through [`Store::autosync_write`],
    /// which wraps this in pull → delete → push and routes a rejected push to the
    /// sync-time divergence surface. Calling `delete` directly is for tests and
    /// the orchestrator only.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidEntryName`] for a malformed name,
    /// [`ErrorCode::EntryNotFound`] if the entry doesn't exist, or a git error
    /// from the underlying remove/commit.
    pub async fn delete(&self, name: &str) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let passfile = passfile_rel(name);
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();

        // Existence gate: a local typo guard (checked before any mutation).
        resolve_entry_path(&repo_path, &passfile)?;

        // Remove the worktree file; the index removal is staged in `commit_removal`.
        let file_path = repo_path.join(&passfile);
        assert_within_repo(&repo_path, file_path.parent().unwrap_or(Path::new("")))?;
        match fs::remove_file(&file_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::new(
                    ErrorCode::EntryNotFound,
                    format!("Entry not found: {name}"),
                ));
            }
            Err(e) => return Err(e.into()),
        }

        let head = self
            .commit_local(
                repo_path,
                passfile,
                format!("Delete secret: {name}"),
                (
                    repo_config.commit_user_name.clone(),
                    repo_config.commit_user_email.clone(),
                ),
                git::commit_removal,
            )
            .await?;
        Ok(WriteResult { commit: head })
    }

    /// Wrap a local-only write in the per-device autosync policy. This is the
    /// sole production write entry point: it holds the Store-wide critical
    /// section across pull → write → push so two in-flight saves can't race the
    /// git index and a manual pull/push/resolve can't interleave with a save.
    ///
    /// - **autosync off** (per-device `repo.json` flag): run `local_write` only
    ///   — a local commit, zero network. The change publishes on the next manual
    ///   Sync.
    /// - **autosync on** (the default): pull (cancellable via `cancel`) → run
    ///   `local_write` → push. A pre-write pull that **diverged** is benign
    ///   (local-ahead is common after any unpushed commit; the write still lands
    ///   on HEAD and the push decides). Only an Enforce authenticity block
    ///   aborts, and it does so before the write runs, so the repo is untouched.
    ///   The push is **not** cancellable today (see `.plans/0032-cancellable-saves.md`);
    ///   it is bounded by git's SSH/HTTP timeout. A `PUSH_REJECTED` is a real
    ///   divergence; a network failure leaves the local commit in place to sync
    ///   later.
    ///
    /// `local_write` must be one of the local-only primitives ([`set`] /
    /// [`delete`] / [`create`] / [`update`]) — it runs inside the critical
    /// section and must NOT re-acquire [`write_mu`] (those primitives don't).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::StoreError`] when Enforce blocks the pre-write pull;
    /// [`ErrorCode::PushRejected`] when the push is rejected (real divergence);
    /// a pull/push network error otherwise; or whatever `local_write` returns.
    pub async fn autosync_write<F, Fut>(
        &self,
        cancel: Option<git::CancelToken>,
        local_write: F,
    ) -> Result<WriteResult, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<WriteResult, Error>>,
    {
        // One critical section across pull → write → push. `set`/`delete` (the
        // local-only primitives the closure calls) do NOT re-acquire this guard.
        let _guard = self.write_mu.lock().await;

        let autosync = self.config.load_repo_config().await?.autosync;
        if !autosync {
            return local_write().await;
        }

        // Pull (cancellable). Divergence is benign — proceed and let the push
        // decide. Only an Enforce block aborts, before the write touches anything.
        match self.sync_with_locked(cancel, None).await? {
            SyncOutcome::FastForwarded(result) if result.authenticity.blocked => {
                return Err(Error::new(
                    ErrorCode::StoreError,
                    "Save aborted: the remote failed signature verification under \
                     Enforce authenticity mode. Pull to review, then retry.",
                ));
            }
            _ => {}
        }

        // Local write (encrypt + commit), inside the critical section.
        let result = local_write().await?;

        // Push. Not cancellable today (RFC 0032); a PUSH_REJECTED is a real
        // divergence, a network error leaves the local commit to sync later.
        self.push_locked().await?;
        Ok(result)
    }

    /// Edit a secret in place: overwrite an **existing** entry's body via the
    /// local-only [`Store::set`]. The edit sibling of [`create`] — but gated on
    /// existence and with no template applied, so a typo'd name can't silently
    /// create a stray entry and the user's raw edited body is stored verbatim
    /// (templates shape new secrets, not mutations).
    ///
    /// The existence gate is a **local typo guard**; it is not a remote-state
    /// invariant. Edit inherits [`set`]'s base-version limitation (see its docs):
    /// a newer same-name remote change can be overwritten silently.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidEntryName`] for a malformed name,
    /// [`ErrorCode::EntryNotFound`] if the entry doesn't exist, or whatever
    /// [`Store::set`] returns.
    pub async fn update(&self, name: &str, plaintext: &[u8]) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let repo_path = self.repo_path().await?;
        // Existence gate: a local typo guard so edit can't create a stray entry.
        // resolve_entry_path also guards path traversal (used identically by `get`
        // and `delete`). NOT a remote-state check.
        resolve_entry_path(&repo_path, &passfile_rel(name))?;
        // Raw write primitive (no template), local-only via `set`.
        self.set(name, plaintext).await
    }

    /// Apply the user's choice for a [`WriteConflict`] returned by [`set`].
    ///
    /// - [`ConflictChoice::KeepMine`] replays our write on the remote tip and
    ///   pushes (refused with `UnsafeOverwrite` if the remote version is
    ///   undecryptable — see [`ConflictChoice::KeepMineForce`]).
    /// - [`ConflictChoice::KeepMineForce`] does the same, overwriting an
    ///   undecryptable remote (destructive; caller-confirmed).
    /// - [`ConflictChoice::KeepRemote`] fast-forwards to the remote, discarding
    ///   our write.
    /// - [`ConflictChoice::Cancel`] leaves the store at the pre-write state.
    ///
    /// Returns `Some(WriteResult)` when a write was pushed (`KeepMine`/`Force`),
    /// `None` otherwise.
    ///
    /// # Errors
    ///
    /// Returns `UnsafeOverwrite` if `KeepMine` is chosen for an undecryptable
    /// remote, or a git error if the underlying fetch/push fails.
    pub async fn resolve_write_conflict(
        &self,
        name: &str,
        plaintext: &[u8],
        choice: ConflictChoice,
    ) -> Result<Option<WriteResult>, Error> {
        let _guard = self.write_mu.lock().await;
        self.resolve_write_conflict_locked(name, plaintext, choice)
            .await
    }

    /// Lock-free inner of [`resolve_write_conflict`] (see [`sync_with_locked`]).
    /// Retained for the frozen-frontend command until the `PR2c` retirement.
    async fn resolve_write_conflict_locked(
        &self,
        name: &str,
        plaintext: &[u8],
        choice: ConflictChoice,
    ) -> Result<Option<WriteResult>, Error> {
        validate_secret_name(name)?;
        let passfile = passfile_rel(name);
        let remote_blob = self.fetch_remote_blob(&passfile).await?;
        let decryptable = match &remote_blob {
            Some(blob) => self.can_decrypt(blob).await,
            None => false,
        };

        match choice {
            ConflictChoice::KeepMine | ConflictChoice::KeepMineForce => {
                if !decryptable && choice == ConflictChoice::KeepMine {
                    return Err(Error::new(
                        ErrorCode::UnsafeOverwrite,
                        "Refusing to overwrite a remote secret we can't decrypt. \
                         Confirm with KeepMineForce to override.",
                    ));
                }
                // Build on the remote tip, then write+push our version.
                self.fast_forward_to_remote().await?;
                match self.write_commit_push(name, plaintext).await? {
                    Some(hash) => Ok(Some(WriteResult { commit: hash })),
                    // Remote moved again mid-resolution — surface the conflict.
                    None => Err(Error::new(
                        ErrorCode::PushRejected,
                        "Remote moved again while resolving the conflict; retry.",
                    )),
                }
            }
            ConflictChoice::KeepRemote => {
                self.fast_forward_to_remote().await?;
                Ok(None)
            }
            ConflictChoice::Cancel => Ok(None),
        }
    }

    /// Resolve a [`SyncOutcome::Diverged`] with the user's [`DivergenceChoice`].
    ///
    /// - [`DivergenceChoice::AdoptRemote`] adopts the reviewed remote tip exactly
    ///   (delegating to [`git::adopt_remote`]).
    /// - [`DivergenceChoice::KeepMine`] re-encrypts the local-only `.age` entries
    ///   onto the reviewed remote tip (with the current recipient set) and pushes
    ///   (see [`Self::resolve_keep_mine`]).
    ///
    /// "Cancel" is client-side (the frontend just doesn't call this). Carries no
    /// plaintext across the call boundary — for "keep mine" the local blobs are
    /// decrypted in-process, used to re-encrypt, and dropped.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::PullFfFailed`] if the remote moved past the reviewed
    /// tip; [`ErrorCode::PushRejected`] for an irreconcilable same-secret
    /// conflict or an undecryptable local entry under "keep mine"; or a
    /// git/signing error otherwise. Under Enforce, an authenticity block returns
    /// `Ok` with [`SyncResult::authenticity`] `.blocked = true` (HEAD unchanged).
    pub async fn resolve_sync_divergence(
        &self,
        expected_remote_oid: &str,
        choice: DivergenceChoice,
    ) -> Result<SyncResult, Error> {
        let _guard = self.write_mu.lock().await;
        self.resolve_sync_divergence_locked(expected_remote_oid, choice)
            .await
    }

    /// Lock-free inner of [`resolve_sync_divergence`] (see [`sync_with_locked`]).
    async fn resolve_sync_divergence_locked(
        &self,
        expected_remote_oid: &str,
        choice: DivergenceChoice,
    ) -> Result<SyncResult, Error> {
        match choice {
            DivergenceChoice::AdoptRemote => {
                let repo_config = self.config.load_repo_config().await?;
                let repo_path = Path::new(&repo_config.local_path).to_path_buf();
                let auth = repo_config.to_git_auth();
                let policy = repo_config.authenticity;
                let expected = expected_remote_oid.to_string();
                spawn_blocking(move || git::adopt_remote(&repo_path, &auth, &policy, &expected))
                    .await?
            }
            DivergenceChoice::KeepMine => self.resolve_keep_mine(expected_remote_oid).await,
        }
    }

    /// "Keep mine" divergence resolution ([`DivergenceChoice::KeepMine`]):
    /// re-encrypt the local-only `.age` entries onto the reviewed remote tip and
    /// push, preserving local changes with the **current** recipient set (so a
    /// remote recipient-list change is honored — not a stale-recipient rebase).
    ///
    /// Five steps, with crypto kept in `Store` (git stays pure): plan (single
    /// fetch + stale-guard + authenticity-verify + replay/conflict computation)
    /// → decrypt local blobs → advance to the reviewed tip (no second fetch)
    /// → re-encrypt to current recipients → write + commit + push.
    async fn resolve_keep_mine(&self, expected_remote_oid: &str) -> Result<SyncResult, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();
        let policy = repo_config.authenticity;
        let commit_identity = (
            repo_config.commit_user_name.clone(),
            repo_config.commit_user_email.clone(),
        );
        let expected = expected_remote_oid.to_string();

        // 1. Plan: fetch once, stale-guard, authenticity-verify, compute the
        //    replay set + conflict detection. Does NOT move HEAD.
        let plan = match spawn_blocking(move || {
            git::keep_local_plan(&repo_path, &auth, &policy, &expected)
        })
        .await??
        {
            git::KeepLocalOutcome::Blocked(result) => return Ok(result),
            git::KeepLocalOutcome::Plan(p) => p,
        };
        let git::KeepLocalPlan {
            fetched_oid,
            replays,
            deletes,
            authenticity,
        } = plan;

        // 2. Decrypt each local blob to plaintext (identity). An undecryptable
        //    local entry can't be re-encrypted → refuse (adopt or cancel rather
        //    than silently drop it). Read the identity ONCE and derive our own
        //    recipient here — the re-encrypt step (4) reuses both. `get_identity_bytes`
        //    returns the cached *unlocked* identity, so this works for
        //    passphrase-protected SSH keys (the PEM is already decrypted).
        let identity = self.get_identity_bytes().await?;
        let identity_str = str::from_utf8(&identity)
            .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Identity is not valid UTF-8"))?;
        let our_recipient = recipient::identity_to_recipient(identity_str, None)?;
        let decrypted: Vec<(String, Zeroizing<Vec<u8>>)> = spawn_blocking(move || {
            let mut out = Vec::with_capacity(replays.len());
            for r in replays {
                let plaintext = crypto::decrypt_bytes(&r.blob, &identity, None).map_err(|_| {
                    Error::new(
                        ErrorCode::PushRejected,
                        format!(
                            "Can't keep mine: \"{}\" can't be decrypted to re-encrypt. \
                             Adopt the remote or cancel.",
                            r.rel_path.trim_end_matches(".age")
                        ),
                    )
                })?;
                out.push((r.rel_path, Zeroizing::new(plaintext)));
            }
            Ok::<_, Error>(out)
        })
        .await??;

        // 3. Advance to the reviewed remote tip — reuses the plan's fetched oid
        //    (objects still in the DB), so no second fetch can race past the
        //    reviewed tip and bypass the authenticity check under Enforce.
        let fetched = fetched_oid.clone();
        let repo_path_adv = self.repo_path().await?;
        spawn_blocking(move || git::keep_local_advance(&repo_path_adv, &fetched)).await??;

        // 4. Re-encrypt to the CURRENT (remote-tip) recipients + our own key
        //    (our_recipient derived once in step 2).
        let repo_path_re = self.repo_path().await?;
        let mut recipients: Vec<String> = recipient::list_recipients(&repo_path_re)
            .await?
            .into_iter()
            .map(|r| r.public_key)
            .collect();
        if !recipients.contains(&our_recipient) {
            recipients.push(our_recipient);
        }
        let ciphertexts: Vec<(String, Vec<u8>)> = spawn_blocking(move || {
            let mut out = Vec::with_capacity(decrypted.len());
            for (rel, plaintext) in decrypted {
                let ct = crypto::encrypt_to_recipients(&plaintext, &recipients)?;
                out.push((rel, ct));
            }
            Ok::<_, Error>(out)
        })
        .await??;

        // 5. Write the re-encrypted entries, apply local deletes, commit, push.
        let repo_config = self.config.load_repo_config().await?;
        let repo_path_fin = Path::new(&repo_config.local_path).to_path_buf();
        let auth_fin = repo_config.to_git_auth();
        let deletes = deletes.clone();
        let head = spawn_blocking(move || {
            git::keep_local_finalize(
                &repo_path_fin,
                &auth_fin,
                &ciphertexts,
                &deletes,
                commit_identity.0.as_deref(),
                commit_identity.1.as_deref(),
            )
        })
        .await??;

        Ok(SyncResult {
            changed: true,
            head,
            authenticity,
        })
    }

    /// Compute the local-vs-remote divergence preview on demand, WITHOUT moving
    /// the working branch. Called by the write path after a push rejection (where
    /// divergence is known to be real) so the app can surface the resolution
    /// modal without a separate sync round-trip.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the fetch fails.
    pub async fn sync_divergence_preview(&self) -> Result<SyncDivergence, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();
        spawn_blocking(move || git::preview_divergence(&repo_path, &auth)).await?
    }

    /// Look up the content template (`.pass-template`) that applies to `name`,
    /// walking up the directory tree (gopass `LookupTemplate`).
    ///
    /// Returns `Ok(None)` when no template applies. Templates are stored as
    /// plaintext, so this reads straight from the worktree.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured.
    pub async fn lookup_template(&self, name: &str) -> Result<Option<String>, Error> {
        let repo_path = self.repo_path().await?;
        let name_owned = name.to_string();
        // Filesystem walk; cheap enough to run on a blocking thread.
        Ok(
            spawn_blocking(move || template::lookup_template_in_repo(&repo_path, &name_owned))
                .await?,
        )
    }

    /// Create a secret, applying a matching `.pass-template` if one exists
    /// (gopass `renderTemplate`).
    ///
    /// `content` becomes the template's `.Content` (usually the password); the
    /// rendered template is what gets stored. When no template applies, the
    /// content is stored verbatim. Either way the result is written and
    /// committed locally via [`Store::set`] (no sync/push from `create` itself).
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` for a bad name, `TemplateError` if a template
    /// references an unknown variable, or whatever [`Store::set`] returns.
    pub async fn create(&self, name: &str, content: &[u8]) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let rendered = self.resolve_template(name, content).await?;
        let final_bytes = rendered.map_or_else(|| content.to_vec(), String::into_bytes);
        self.set(name, &final_bytes).await
    }

    /// Resolve a `.pass-template` for `name` against `content` and return the
    /// rendered body, or `None` when no (non-empty) template applies or the
    /// payload isn't UTF-8. Shared by [`Store::create`] and
    /// [`Store::preview_create`].
    async fn resolve_template(&self, name: &str, content: &[u8]) -> Result<Option<String>, Error> {
        // Templates render against text; secrets are text, so a non-UTF-8
        // payload just skips templating.
        Ok(
            match (
                str::from_utf8(content).ok(),
                self.lookup_template(name).await?,
            ) {
                (Some(text), Some(tpl)) if !tpl.trim().is_empty() => {
                    Some(template::render(&tpl, &template_vars(name, text))?)
                }
                _ => None,
            },
        )
    }

    /// Preview what [`Store::create`] would store for `name` + `content`: the
    /// rendered template body when a `.pass-template` applies, or `None` when no
    /// template applies (in which case `content` is stored verbatim). Writes
    /// nothing — used by the UI to show what a template will produce before save.
    ///
    /// `content` becomes the template's `.Content`, exactly as in [`create`].
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` for a bad name, or `TemplateError` if a
    /// template references an unknown variable.
    pub async fn preview_create(
        &self,
        name: &str,
        content: &[u8],
    ) -> Result<Option<String>, Error> {
        validate_secret_name(name)?;
        self.resolve_template(name, content).await
    }

    /// Create a secret from one of the built-in presets (gopass `gopass create`
    /// wizard). `fields` maps each preset field key to its value; the `password`
    /// field becomes the secret's first line and the rest become `key: value`
    /// body lines. The secret is generated at `<prefix>/<name-from-fields>`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` if the preset is unknown or a required field
    /// is missing, or whatever [`Store::create`] returns.
    pub async fn create_from_preset<S: ::std::hash::BuildHasher>(
        &self,
        preset_id: &str,
        fields: &HashMap<&str, String, S>,
    ) -> Result<WriteResult, Error> {
        let preset = template::find_preset(preset_id).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidEntryName,
                format!("unknown create preset: {preset_id:?}"),
            )
        })?;
        let name = template::preset_name(preset, fields)?;
        let body = template::preset_body(preset, fields)?;
        self.create(&name, &body).await
    }

    /// Encrypt to the store recipients (+ our key), write `<name>.age` to the
    /// worktree, commit, and push. Assumes the repo is at a clean synced HEAD.
    ///
    /// Returns `Ok(Some(hash))` on a successful push, `Ok(None)` if the push
    /// was rejected (non-fast-forward) — the caller rolls back / handles it.
    /// The written file and commit are left in place on rejection; the caller
    /// is expected to reset.
    async fn write_commit_push(
        &self,
        name: &str,
        plaintext: &[u8],
    ) -> Result<Option<String>, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();

        let passfile = self.encrypt_and_write(name, plaintext, &repo_path).await?;

        self.commit_push(
            repo_path,
            auth,
            passfile,
            format!("Save secret: {name}"),
            (
                repo_config.commit_user_name.clone(),
                repo_config.commit_user_email.clone(),
            ),
            git::commit,
        )
        .await
    }

    /// Commit `passfile` (the caller has already mutated the worktree) locally,
    /// with **no push**. `commit_fn` is [`git::commit`] for a save or
    /// [`git::commit_removal`] for a delete. This is the local-only commit half
    /// shared by the local-only write primitives ([`Store::set`] / [`Store::delete`])
    /// and by [`commit_push`] (which adds the push).
    async fn commit_local(
        &self,
        repo_path: PathBuf,
        passfile: String,
        message: String,
        commit_identity: (Option<String>, Option<String>),
        commit_fn: CommitFn,
    ) -> Result<String, Error> {
        spawn_blocking(move || {
            let paths = vec![passfile];
            commit_fn(
                &repo_path,
                &paths,
                &message,
                commit_identity.0.as_deref(),
                commit_identity.1.as_deref(),
            )
        })
        .await?
    }

    /// Commit `passfile` (the caller has already mutated the worktree — written
    /// the file for a save, or removed it for a delete) and push. Delegates the
    /// commit to [`commit_local`] (DRY) and adds the push + push-rejection
    /// mapping. Used by the legacy push paths ([`Store::resolve_write_conflict`])
    /// retained until the frontend flip; the local-only write primitives commit
    /// via [`commit_local`] directly.
    ///
    /// Returns `Ok(Some(head))` on success, `Ok(None)` on a push rejection
    /// (non-fast-forward). The commit/file are left in place on rejection; the
    /// caller is expected to reset.
    async fn commit_push(
        &self,
        repo_path: PathBuf,
        auth: git::GitAuth,
        passfile: String,
        message: String,
        commit_identity: (Option<String>, Option<String>),
        commit_fn: CommitFn,
    ) -> Result<Option<String>, Error> {
        let repo_path_for_push = repo_path.clone();
        let head = self
            .commit_local(repo_path, passfile, message, commit_identity, commit_fn)
            .await?;

        let push_result = spawn_blocking(move || git::push(&repo_path_for_push, &auth)).await?;
        match push_result {
            Ok(()) => Ok(Some(head)),
            Err(e) if e.code == "PUSH_REJECTED" => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Encrypt `plaintext` to the store recipients (ensuring our own key is
    /// included) and write it to `<name>.age` atomically. Returns the passfile
    /// path relative to the repo root.
    async fn encrypt_and_write(
        &self,
        name: &str,
        plaintext: &[u8],
        repo_path: &Path,
    ) -> Result<String, Error> {
        let passfile = passfile_rel(name);
        let file_path = repo_path.join(&passfile);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        assert_within_repo(repo_path, file_path.parent().unwrap_or(Path::new("")))?;

        // Recipients: everyone in the store, plus our own key (ensureOurKeyID).
        let recipients = recipient::list_recipients(repo_path).await?;
        let identity_bytes = self.get_identity_bytes().await?;
        let identity_str = str::from_utf8(&identity_bytes)
            .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Identity is not valid UTF-8"))?;
        let our_recipient = recipient::identity_to_recipient(identity_str, None)?;
        let mut recipients_str: Vec<String> =
            recipients.iter().map(|r| r.public_key.clone()).collect();
        if !recipients_str.iter().any(|r| r == &our_recipient) {
            recipients_str.push(our_recipient);
        }

        let plaintext_owned = Zeroizing::new(plaintext.to_vec());
        let ciphertext = spawn_blocking(move || {
            crypto::encrypt_to_recipients(&plaintext_owned, &recipients_str)
        })
        .await??;

        write_atomic(&file_path, &ciphertext).await?;
        Ok(passfile)
    }

    /// Whether `blob` (an age ciphertext) decrypts with our identity.
    async fn can_decrypt(&self, blob: &[u8]) -> bool {
        let Ok(identity_bytes) = self.get_identity_bytes().await else {
            return false;
        };
        let blob_owned = blob.to_vec();
        spawn_blocking(move || crypto::decrypt_bytes(&blob_owned, &identity_bytes, None).is_ok())
            .await
            .unwrap_or(false)
    }

    // ── thin wrappers over git ops (load config + spawn_blocking) ───────────

    async fn fast_forward_to_remote(&self) -> Result<(), Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();
        spawn_blocking(move || git::fast_forward_to_remote(&repo_path, &auth)).await?
    }

    async fn fetch_remote_blob(&self, rel_path: &str) -> Result<Option<Vec<u8>>, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();
        let rel = rel_path.to_string();
        spawn_blocking(move || git::fetch_remote_blob(&repo_path, &auth, &rel)).await?
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
    pub async fn sync(&self) -> Result<SyncOutcome, Error> {
        self.sync_with(None, None).await
    }

    /// Cancellable, progress-reporting variant of [`sync`](Store::sync).
    ///
    /// `cancel` aborts the in-progress fetch (mapped to [`ErrorCode::Cancelled`]);
    /// `progress` receives transfer stats. The internal pre-push sync of the
    /// write path keeps using the plain [`sync`](Store::sync) (silent,
    /// non-cancellable) — only the user-initiated pull opts in.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured, the remote is
    /// unreachable, the branches have diverged, or Enforce mode refuses the
    /// pull.
    pub async fn sync_with(
        &self,
        cancel: Option<git::CancelToken>,
        progress: Option<git::ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let _guard = self.write_mu.lock().await;
        self.sync_with_locked(cancel, progress).await
    }

    /// Lock-free inner of [`sync_with`]. The caller already holds the
    /// [`write_mu`] critical section: [`sync_with`] acquires it for the
    /// standalone pull, and [`autosync_write`] holds it across pull → write →
    /// push and calls this directly.
    async fn sync_with_locked(
        &self,
        cancel: Option<git::CancelToken>,
        progress: Option<git::ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();
        let policy = repo_config.authenticity;
        spawn_blocking(move || {
            git::pull_repo(
                &repo_path,
                &auth,
                &policy,
                cancel.as_ref(),
                progress.as_ref(),
            )
        })
        .await?
    }

    /// Push the current branch to `origin`.
    ///
    /// Used by the create flow's deferred first push — performed after the
    /// identity is durable (via `complete_setup`) so the remote only receives the
    /// store once it can be decrypted locally. A missing `origin` is a no-op
    /// (local-only store), mirroring [`sync`](Store::sync)'s pull no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or the push fails for a
    /// reason other than a missing origin (which is treated as a no-op).
    pub async fn push(&self) -> Result<(), Error> {
        let _guard = self.write_mu.lock().await;
        self.push_locked().await
    }

    /// Lock-free inner of [`push`] (see [`sync_with_locked`]).
    async fn push_locked(&self) -> Result<(), Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path).to_path_buf();
        let auth = repo_config.to_git_auth();
        spawn_blocking(move || git::push(&repo_path, &auth)).await?
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

    /// Set the git commit author identity. `None` (or blank) for a field clears
    /// it, reverting to the app default so the value keeps tracking future
    /// shipped defaults. Values are trimmed; characters that would corrupt a
    /// commit (`<`, `>`, control bytes) are rejected. Returns the persisted
    /// [`RepoConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ConfigError`] if a value contains an invalid
    /// character, or if the config cannot be loaded or persisted.
    pub async fn set_commit_identity(
        &self,
        name: Option<String>,
        email: Option<String>,
    ) -> Result<RepoConfig, Error> {
        let normalize = |v: Option<String>| -> Result<Option<String>, Error> {
            let Some(s) = v else {
                return Ok(None);
            };
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            // Reject characters that corrupt the commit's `Name <email>` line:
            // control bytes (newline, NUL, …) and the envelope delimiters. The
            // `git` CLI rejects these for user.name/user.email; libgit2's
            // `Signature::now` validates nothing, so gpm must.
            if let Some(c) = t.chars().find(|&c| c.is_control() || c == '<' || c == '>') {
                return Err(Error::new(
                    ErrorCode::ConfigError,
                    format!(
                        "Commit identity contains an invalid character ({c:?}). Newlines, \
                         angle brackets, and control characters corrupt git commits."
                    ),
                ));
            }
            Ok(Some(t.to_string()))
        };
        let name = normalize(name)?;
        let email = normalize(email)?;
        let mut rc = self.config.load_repo_config().await?;
        rc.commit_user_name = name;
        rc.commit_user_email = email;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Set the auto-lock mode. `Idle(n)` seconds are clamped to
    /// `[LOCK_IDLE_SECS_MIN, LOCK_IDLE_SECS_MAX]`; `Immediate` and `Never` take
    /// no duration. Returns the persisted [`RepoConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be loaded or persisted.
    pub async fn set_lock_mode(&self, mode: LockMode) -> Result<RepoConfig, Error> {
        let mode = match mode {
            LockMode::Idle(secs) => {
                LockMode::Idle(secs.clamp(LOCK_IDLE_SECS_MIN, LOCK_IDLE_SECS_MAX))
            }
            other => other,
        };
        let mut rc = self.config.load_repo_config().await?;
        rc.lock_mode = mode;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Set the password-view auto-clear override. `None` clears it (resolves to
    /// the default); `Some(0)` means never auto-clear; any other value is clamped
    /// to `[CLEAR_SECS_MIN, CLEAR_SECS_MAX]`. Returns the persisted [`RepoConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be loaded or persisted.
    pub async fn set_view_clear_secs(&self, secs: Option<u64>) -> Result<RepoConfig, Error> {
        let secs = normalize_clear_secs(secs);
        let mut rc = self.config.load_repo_config().await?;
        rc.view_clear_secs = secs;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Set the clipboard auto-clear override. Same resolution rule as
    /// [`set_view_clear_secs`](Store::set_view_clear_secs).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be loaded or persisted.
    pub async fn set_clipboard_clear_secs(&self, secs: Option<u64>) -> Result<RepoConfig, Error> {
        let secs = normalize_clear_secs(secs);
        let mut rc = self.config.load_repo_config().await?;
        rc.clipboard_clear_secs = secs;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Set the per-device autosync flag (whether each save wraps in a
    /// pull → write → push via [`autosync_write`]). Default `true`; when `false`,
    /// saves stay local until a manual Sync.
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be loaded or persisted.
    pub async fn set_autosync(&self, enabled: bool) -> Result<RepoConfig, Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.autosync = enabled;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Persist the app-launch biometric gate flag. This only stores the intent;
    /// the actual master-key migration between the auth-free and biometric-gated
    /// Keystore stores is orchestrated by the app layer (which owns the plugins),
    /// so the flag and the key's location are kept consistent together there.
    ///
    /// # Errors
    ///
    /// Returns an error if `repo.json` cannot be read or written.
    pub async fn set_biometric_app_lock(&self, enabled: bool) -> Result<RepoConfig, Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.biometric_app_lock = enabled;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Persist the "unlock the identity together with the app" opt-in. A pure
    /// preference (no key migration), read by the app-unlock path right after the
    /// master key is injected.
    ///
    /// # Errors
    ///
    /// Returns an error if `repo.json` cannot be read or written.
    pub async fn set_unlock_identity_with_app(&self, enabled: bool) -> Result<RepoConfig, Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.unlock_identity_with_app = enabled;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Seal the identity passphrase under the at-rest master key, for the
    /// identity-auto-unlock opt-in. See [`Config::save_app_identity_pass`].
    ///
    /// # Errors
    ///
    /// Returns an error if the AEAD seal or the write fails.
    pub async fn save_app_identity_pass(&self, passphrase: &str) -> Result<(), Error> {
        self.config
            .save_app_identity_pass(passphrase.as_bytes())
            .await
    }

    /// Load the sealed identity passphrase. See
    /// [`Config::load_app_identity_pass`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::NoIdentity`] if the slot is absent, or an error if
    /// the AEAD unseal fails (e.g. the master key is wiped).
    pub async fn load_app_identity_pass(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        Ok(Zeroizing::new(self.config.load_app_identity_pass().await?))
    }

    /// Clear the sealed identity passphrase slot. See
    /// [`Config::clear_app_identity_pass`].
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be removed.
    pub async fn clear_app_identity_pass(&self) -> Result<(), Error> {
        self.config.clear_app_identity_pass().await
    }

    /// The default commit author identity, for UI display. Reads the shipped
    /// default so the frontend never hardcodes it.
    #[must_use]
    pub fn commit_identity_default() -> CommitIdentity {
        CommitIdentity {
            name: crate::config::DEFAULT_COMMIT_NAME.to_string(),
            email: crate::config::DEFAULT_COMMIT_EMAIL.to_string(),
        }
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

/// Fuzzy-rank `entries` by `query`, best match first.
///
/// Each entry is scored against its `name` and `path` (the higher score wins),
/// so an entry appears at most once. Entries are ordered by score descending,
/// then by `path` ascending. Because `path` is unique, this is a **strict total
/// order** — stable and safe to paginate by offset for a fixed entry set.
///
/// Matching is subsequence-based (fzf-style) and case-insensitive; it is not
/// typo-tolerant. An empty query returns `entries` unchanged, preserving the
/// caller's order (e.g. the alpha order from [`list_entries`]).
#[must_use]
pub fn rank_entries(entries: Vec<Entry>, query: &str) -> Vec<Entry> {
    let q = query.trim();
    if q.is_empty() {
        return entries;
    }
    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
    let pattern = Pattern::parse(q, CaseMatching::Ignore, Normalization::Smart);
    let mut scored: Vec<(u32, Entry)> = entries
        .into_iter()
        .filter_map(|e| {
            let best = fuzzy_score(&mut matcher, &pattern, &e.name).max(fuzzy_score(
                &mut matcher,
                &pattern,
                &e.path,
            ))?;
            Some((best, e))
        })
        .collect();
    // score desc, then path asc (path is unique → strict total order)
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
    scored.into_iter().map(|(_, e)| e).collect()
}

/// Walk the store at `repo_path` and return its entries fuzzy-ranked by `query`
/// (empty query → all entries, alpha-sorted, like [`list_entries`]). A thin
/// wrapper over [`list_entries`] + [`rank_entries`] for callers that want search
/// semantics directly off a path (and for tests).
///
/// # Errors
///
/// Returns an error if the repository path does not exist (via [`list_entries`]).
pub fn search_entries_in(repo_path: &Path, query: &str) -> Result<Vec<Entry>, Error> {
    Ok(rank_entries(list_entries(repo_path)?, query))
}

/// One page of a ranked entry set: a slice of up to `limit` entries starting at
/// `offset`, together with the **total** count of the full ranked set the slice
/// was taken from (independent of the slice). The caller derives `has_more` as
/// `offset + entries.len() < total`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedPage {
    /// The page's entries (up to `limit`, starting at `offset`).
    pub entries: Vec<Entry>,
    /// Total entries in the full ranked set the page was sliced from.
    pub total: usize,
}

/// Slice a ranked `Vec<Entry>` to one page of up to `limit` entries starting at
/// `offset`. An `offset` past the end yields an empty page carrying the real
/// `total`. Pure over the input order — the caller ranks first.
#[must_use]
pub fn slice_page(ranked: Vec<Entry>, offset: usize, limit: usize) -> RankedPage {
    let total = ranked.len();
    let entries = ranked.into_iter().skip(offset).take(limit).collect();
    RankedPage { entries, total }
}

/// Score `haystack` against the parsed `pattern` (`None` when it does not
/// fuzzy-match). ASCII haystacks take the fast [`Utf32Str::Ascii`] path;
/// non-ASCII names fall back to a `Vec<char>` buffer.
fn fuzzy_score(matcher: &mut Matcher, pattern: &Pattern, haystack: &str) -> Option<u32> {
    if haystack.is_ascii() {
        pattern.score(Utf32Str::Ascii(haystack.as_bytes()), matcher)
    } else {
        let buf: Vec<char> = haystack.chars().collect();
        pattern.score(Utf32Str::Unicode(&buf), matcher)
    }
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

/// Build the [`template::TemplateVars`] for an entry named `name` with the
/// given content text. All name-derived slices borrow `name`.
fn template_vars<'a>(name: &'a str, content: &'a str) -> template::TemplateVars<'a> {
    let base = name.rfind('/').map_or(name, |i| &name[i + 1..]);
    let dir = name.rfind('/').map_or("", |i| &name[..i]);
    let dirname = dir.rfind('/').map_or(dir, |i| &dir[i + 1..]);
    template::TemplateVars {
        content,
        name: base,
        path: name,
        dir,
        dirname,
    }
}

/// The on-disk relative path for a secret named `name` (gopass `passfile`).
///
/// A leading `/` is stripped; if the name already ends in `.age` it is kept
/// as-is, otherwise `.age` is appended. Matches the resolution `get` uses.
fn passfile_rel(name: &str) -> String {
    let name = name.trim_start_matches('/');
    if Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("age"))
    {
        name.to_string()
    } else {
        format!("{name}.age")
    }
}

/// Validate a secret name before writing (gopass `ValidateSecretName`).
///
/// Rejects empty/whitespace names, leading or trailing `/`, empty segments
/// (`//`), backslashes, NUL and other control characters, and `.`/`..` path
/// segments. This is the front-line path-traversal guard; [`assert_within_repo`]
/// is the defense-in-depth backstop.
fn validate_secret_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() {
        return Err(invalid_name("Secret name must not be empty"));
    }
    if name.starts_with('/') || name.ends_with('/') {
        return Err(invalid_name("Secret name must not start or end with '/'"));
    }
    if name.contains("//") {
        return Err(invalid_name(
            "Secret name must not contain empty path segments",
        ));
    }
    if name.contains('\\') || name.contains('\0') {
        return Err(invalid_name(
            "Secret name must not contain backslashes or NUL bytes",
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid_name(
            "Secret name must not contain control characters",
        ));
    }
    if name.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(invalid_name(
            "Secret name must not contain '.' or '..' segments",
        ));
    }
    Ok(())
}

/// Defense-in-depth check that `dir` resolves inside `repo_path`.
///
/// Used after creating a secret's parent directory: the directory exists, so it
/// can be canonicalized, and we assert it is contained by the canonical repo
/// root. Catches any traversal a name-validation gap would otherwise allow.
fn assert_within_repo(repo_path: &Path, dir: &Path) -> Result<(), Error> {
    let canonical_repo = repo_path.canonicalize()?;
    let canonical_dir = dir.canonicalize()?;
    if !canonical_dir.starts_with(&canonical_repo) {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            "Entry path is outside repository",
        ));
    }
    Ok(())
}

/// Atomic write: write to a temp file beside the target, then rename over it.
///
/// Mirrors [`Config`'s](crate::config::Config) atomic write so a failed write
/// can never leave a half-written ciphertext behind.
async fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, data).await?;
    fs::rename(&temp_path, path).await?;
    Ok(())
}

/// Build an `InvalidEntryName` error (keeps call sites terse).
fn invalid_name(message: &str) -> Error {
    Error::new(ErrorCode::InvalidEntryName, message)
}

/// Normalize a view/clipboard auto-clear override: `None` stays (default),
/// `Some(0)` stays (Never), any other `Some(n)` is clamped to
/// `[CLEAR_SECS_MIN, CLEAR_SECS_MAX]`. Infallible — out-of-range clamps rather
/// than erroring, since the UI sends only preset values.
fn normalize_clear_secs(secs: Option<u64>) -> Option<u64> {
    match secs {
        None => None,
        Some(0) => Some(0),
        Some(n) => Some(n.clamp(CLEAR_SECS_MIN, CLEAR_SECS_MAX)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;

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

    // ── rank_entries (fuzzy search ranking) ───────────────────────────

    fn rank_sample_entries() -> Vec<Entry> {
        vec![
            Entry {
                path: "cloud/aws/root.age".to_string(),
                name: "cloud/aws/root".to_string(),
            },
            Entry {
                path: "github.com/user.age".to_string(),
                name: "github-token".to_string(),
            },
            Entry {
                path: "email/personal.age".to_string(),
                name: "personal-email".to_string(),
            },
            Entry {
                path: "servers/prod.age".to_string(),
                name: "prod-server".to_string(),
            },
        ]
    }

    #[test]
    fn rank_entries_empty_query_returns_all_unchanged() {
        assert_eq!(
            rank_entries(rank_sample_entries(), ""),
            rank_sample_entries()
        );
        assert_eq!(
            rank_entries(rank_sample_entries(), "   "),
            rank_sample_entries(),
            "whitespace-only query is treated as empty"
        );
    }

    #[test]
    fn rank_entries_subsequence_non_contiguous_match() {
        // "awroot" matches "cloud/aws/root" as a subsequence (chars in order, gaps ok).
        let r = rank_entries(rank_sample_entries(), "awroot");
        assert!(r.iter().any(|e| e.path == "cloud/aws/root.age"));
    }

    #[test]
    fn rank_entries_case_insensitive() {
        let r = rank_entries(rank_sample_entries(), "AWS");
        assert!(r.iter().any(|e| e.path == "cloud/aws/root.age"));
    }

    #[test]
    fn rank_entries_matches_non_ascii_names() {
        // Exercises the Utf32Str::Unicode branch in fuzzy_score (non-ASCII haystack).
        let e = vec![Entry {
            path: "accounts/café.age".to_string(),
            name: "accounts/café".to_string(),
        }];
        let r = rank_entries(e, "café");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rank_entries_no_match_returns_empty() {
        assert!(rank_entries(rank_sample_entries(), "zzznomatch").is_empty());
    }

    #[test]
    fn rank_entries_query_longer_than_any_target_excluded() {
        assert!(rank_entries(rank_sample_entries(), "abcdefghijklmnopqrstuvwxyz").is_empty());
    }

    #[test]
    fn rank_entries_best_match_first() {
        let r = rank_entries(rank_sample_entries(), "github");
        assert_eq!(
            r.first().map(|e| e.path.as_str()),
            Some("github.com/user.age")
        );
    }

    #[test]
    fn rank_entries_dedups_across_name_and_path() {
        // "github" matches both the name and the path of one entry → appears once.
        let r = rank_entries(rank_sample_entries(), "github");
        assert_eq!(
            r.iter().filter(|e| e.path == "github.com/user.age").count(),
            1
        );
    }

    #[test]
    fn rank_entries_strict_total_order_tiebreak_by_path() {
        // Two entries with identical names → equal name-score → tiebreak by unique path.
        let e = vec![
            Entry {
                path: "b/zzz.age".to_string(),
                name: "same".to_string(),
            },
            Entry {
                path: "a/zzz.age".to_string(),
                name: "same".to_string(),
            },
        ];
        let r = rank_entries(e, "same");
        assert_eq!(r.len(), 2);
        let paths: Vec<&str> = r.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(paths, vec!["a/zzz.age", "b/zzz.age"]);
    }

    #[test]
    fn rank_entries_perf_5k_synthetic() {
        // Coarse regression guard: ranking 5k entries must stay under this
        // deliberately-loose budget (debug build; generous to avoid CI flakes;
        // catches an O(n^2) or accidental-clone regression). Measured time printed.
        let entries: Vec<Entry> = (0..5_000)
            .map(|i| Entry {
                path: format!("dir/entry-{i}.age"),
                name: format!("dir/entry-{i}"),
            })
            .collect();
        let start = std::time::Instant::now();
        let r = rank_entries(entries, "entry-42");
        let elapsed = start.elapsed();
        eprintln!("rank_entries 5k: {elapsed:?}");
        assert!(
            elapsed.as_millis() < 1000,
            "rank_entries 5k took too long: {elapsed:?}"
        );
        assert!(r.iter().any(|e| e.name == "dir/entry-42"));
    }

    // ── slice_page (pagination) ───────────────────────────────────────

    fn page_sample() -> Vec<Entry> {
        vec![
            Entry {
                path: "a.age".to_string(),
                name: "a".to_string(),
            },
            Entry {
                path: "b.age".to_string(),
                name: "b".to_string(),
            },
            Entry {
                path: "c.age".to_string(),
                name: "c".to_string(),
            },
            Entry {
                path: "d.age".to_string(),
                name: "d".to_string(),
            },
            Entry {
                path: "e.age".to_string(),
                name: "e".to_string(),
            },
        ]
    }

    #[test]
    fn slice_page_basic_offset_limit() {
        let p = slice_page(page_sample(), 0, 2);
        assert_eq!(p.total, 5);
        assert_eq!(p.entries.len(), 2);
        let paths: Vec<&str> = p.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a.age", "b.age"]);
    }

    #[test]
    fn slice_page_second_page() {
        let p = slice_page(page_sample(), 2, 2);
        assert_eq!(p.total, 5);
        let paths: Vec<&str> = p.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["c.age", "d.age"]);
    }

    #[test]
    fn slice_page_last_partial_page() {
        // 5 entries, pages of 2 → the last page has 1.
        let p = slice_page(page_sample(), 4, 2);
        assert_eq!(p.entries.len(), 1);
        let paths: Vec<&str> = p.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["e.age"]);
        assert_eq!(p.total, 5);
    }

    #[test]
    fn slice_page_offset_beyond_total_is_empty() {
        let p = slice_page(page_sample(), 10, 2);
        assert!(p.entries.is_empty());
        assert_eq!(p.total, 5, "total stays the real full count");
    }

    #[test]
    fn slice_page_offset_at_boundary() {
        // offset exactly == len → empty page, total preserved.
        let p = slice_page(page_sample(), 5, 2);
        assert!(p.entries.is_empty());
        assert_eq!(p.total, 5);
    }

    #[test]
    fn slice_page_limit_zero_is_empty() {
        let p = slice_page(page_sample(), 0, 0);
        assert!(p.entries.is_empty());
        assert_eq!(p.total, 5);
    }

    #[test]
    fn slice_page_preserves_strict_order_across_pages() {
        // The load-bearing pagination-correctness test: concatenating pages
        // reproduces the full order, including a final partial page. Empty
        // query keeps the input order, so this is deterministic.
        let ranked = rank_entries(page_sample(), "");
        let full: Vec<String> = ranked.iter().map(|e| e.path.clone()).collect();
        let mut paged: Vec<String> = Vec::new();
        let mut offset = 0;
        loop {
            let p = slice_page(ranked.clone(), offset, 2);
            paged.extend(p.entries.iter().map(|e| e.path.clone()));
            offset += 2;
            if p.entries.len() < 2 {
                break;
            }
        }
        assert_eq!(paged, full);
        assert_eq!(full.len(), 5, "sanity: spans 2 full pages + 1 partial");
    }

    // ── unlock/lock tests ──────────────────────────────────────────────

    #[test]
    fn lock_clears_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        assert!(!store.is_unlocked());
        store.lock();
        assert!(!store.is_unlocked());
    }

    #[tokio::test]
    async fn unlock_is_noop_for_plaintext_identity() {
        // The raw-passphrase cache is gone, so unlock() on a plaintext identity
        // is a true no-op — nothing is cached and is_unlocked() stays false. (In
        // production unlock() is never called on plaintext: the router gates
        // /unlock on is_identity_encrypted().) Plaintext identities decrypt
        // straight from disk via get() without unlocking.
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        store.unlock("passphrase").await.unwrap();
        assert!(
            store.cached_identity.read().is_ok_and(|g| g.is_none()),
            "plaintext identity must not populate the decrypted-identity cache"
        );
        assert!(
            !store.is_unlocked(),
            "unlock() on a plaintext identity must not mark the store unlocked"
        );
    }

    #[tokio::test]
    async fn unlock_caches_decrypted_ssh_identity() {
        // An encrypted SSH identity decrypts ONCE at unlock() and caches the
        // UNENCRYPTED PEM in cached_identity (previously only the passphrase was
        // cached and the key was re-derived per entry). The cached bytes must be
        // an unencrypted OpenSSH PEM so per-entry decrypts skip the bcrypt KDF.
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        assert!(
            !store.is_unlocked(),
            "store must start locked for an encrypted SSH identity"
        );

        store.unlock("test-passphrase").await.unwrap();

        let guard = store.cached_identity.read().expect("cache lock");
        let cached = guard
            .as_ref()
            .expect("cached_identity must be populated for an SSH identity");
        let pem = str::from_utf8(cached).expect("cached bytes are a PEM string");
        assert!(
            pem.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"),
            "cached SSH identity must be an OpenSSH PEM"
        );
        assert!(
            matches!(
                ssh::Identity::from_buffer(pem.as_bytes(), None),
                Ok(ssh::Identity::Unencrypted(_))
            ),
            "cached SSH PEM must parse as Unencrypted (no KDF)"
        );
        assert!(
            store.is_unlocked(),
            "an encrypted SSH identity must be recognised as unlocked after unlock()"
        );
    }

    /// A wrong passphrase for an encrypted SSH identity returns
    /// `WrongPassphrase` — the exact code `UnlockModal` and biometric
    /// self-healing key on.
    #[tokio::test]
    async fn unlock_wrong_ssh_passphrase_returns_wrong_passphrase() {
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        let err = store.unlock("wrong-passphrase").await.unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");
        assert!(
            !store.is_unlocked(),
            "a failed unlock must not unlock the store"
        );
    }

    /// A legacy RSA PEM identity is NOT classified as encrypted, so `unlock()`
    /// is never routed to the SSH-caching path for it. This is what keeps legacy
    /// RSA identities working — `to_unencrypted_pem` is OpenSSH-only, but it
    /// never sees legacy RSA because `is_identity_encrypted()` returns false
    /// (age reads unencrypted PEM as `Unencrypted`, encrypted PEM as
    /// `Unsupported` — never `Encrypted`). Unencrypted legacy RSA still decrypts
    /// via the normal `get()` path without unlocking.
    #[tokio::test]
    async fn is_identity_encrypted_false_for_legacy_rsa_pem() {
        let rsa_key = b"-----BEGIN RSA PRIVATE KEY-----\nMIIEogIBAAKCAQEAxO5yF0xjbmkQTfbaCP8DQC7kHnPJr5bdIie6Nzmg9lL6Chye\n0vK5iJ+BYkA1Hnf1WnNzoVIm3otZPkwZptertkY95JYFmTiA4IvHeL1yiOTd2AYc\na947EPpM9XPomeM/7U7c99OvuCuOl1YlTFsMsoPY/NiZ+NZjgMvb3XgyH0OXy3mh\nqp+SsJU+tRjZGfqM1iv2TZUCJTQnKF8YSVCyLPV67XM1slQQHmtZ5Q6NFhzg3j8a\nCY5rDR66UF5+Zn/TvN8bNdKn01I50VLePI0ZnnRcuLXK2t0Bpkk0NymZ3vsF10m9\nHCKVyxr2Y0Ejx4BtYXOK97gaYks73rBi7+/VywIDAQABAoIBADGsf8TWtOH9yGoS\nES9hu90ttsbjqAUNhdv+r18Mv0hC5+UzEPDe3uPScB1rWrrDwXS+WHVhtoI+HhWz\ntmi6UArbLvOA0Aq1EPUS7Q7Mop5bNIYwDG09EiMXL+BeC1b91nsygFRW5iULf502\n0pOvB8XjshEdRcFZuqGbSmtTzTjLLxYS/aboBtZLHrH4cRlFMpHWCSuJng8Psahp\nSnJbkjL7fHG81dlH+M3qm5EwdDJ1UmNkBfoSfGRs2pupk2cSJaL+SPkvNX+6Xyoy\nyvfnbJzKUTcV6rf+0S0P0yrWK3zRK9maPJ1N60lFui9LvFsunCLkSAluGKiMwEjb\nfm40F4kCgYEA+QzIeIGMwnaOQdAW4oc7hX5MgRPXJ836iALy56BCkZpZMjZ+VKpk\n8P4E1HrEywpgqHMox08hfCTGX3Ph6fFIlS1/mkLojcgkrqmg1IrRvh8vvaZqzaAf\nGKEhxxRta9Pvm44E2nUY97iCKzE3Vfh+FIyQLRuc+0COu49Me4HPtBUCgYEAym1T\nvNZKPfC/eTMh+MbWMsQArOePdoHQyRC38zeWrLaDFOUVzwzEvCQ0IzSs0PnLWkZ4\nxx60wBg5ZdU4iH4cnOYgjavQrbRFrCmZ1KDUm2+NAMw3avcLQqu41jqzyAlkktUL\nfZzyqHIBmKYLqut5GslkGnQVg6hB4psutHhiel8CgYA3yy9WH9/C6QBxqgaWdSlW\nfLby69j1p+WKdu6oCXUgXW3CHActPIckniPC3kYcHpUM58+o5wdfYnW2iKWB3XYf\nRXQiwP6MVNwy7PmE5Byc9Sui1xdyPX75648/pEnnMDGrraNUtYsEZCd1Oa9l6SeF\nvv/Fuzvt5caUKkQ+HxTDCQKBgFhqUiXr7zeIvQkiFVeE+a/ovmbHKXlYkCoSPFZm\nVFCR00VAHjt2V0PaCE/MRSNtx61hlIVcWxSAQCnDbNLpSnQZa+SVRCtqzve4n/Eo\nYlSV75+GkzoMN4XiXXRs5XOc7qnXlhJCiBac3Segdv4rpZTWm/uV8oOz7TseDtNS\ntai/AoGAC0CiIJAzmmXscXNS/stLrL9bb3Yb+VZi9zN7Cb/w7B0IJ35N5UOFmKWA\nQIGpMU4gh6p52S1eLttpIf2+39rEDzo8pY6BVmEp3fKN3jWmGS4mJQ31tWefupC+\nfGNu+wyKxPnSU3svsuvrOdwwDKvfqCNyYK878qKAAaBqbGT1NJ8=\n-----END RSA PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(rsa_key, None).await.unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        assert!(
            !store.is_identity_encrypted().await,
            "legacy RSA PEM must not be treated as encrypted"
        );
    }

    #[tokio::test]
    async fn is_identity_encrypted_false_for_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        assert!(!store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn is_identity_encrypted_true_after_encrypted_save() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("pass123"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        assert!(store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn is_identity_encrypted_true_for_encrypted_ssh_key() {
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        assert!(store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn is_identity_encrypted_false_for_unencrypted_ssh_key() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(unencrypted_ssh_key, None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        assert!(!store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn save_identity_stores_ssh_key_as_plaintext_even_with_passphrase() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);

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
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        let err = store.set_passphrase("").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    #[tokio::test]
    async fn set_passphrase_rejects_already_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("old"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        let err = store.set_passphrase("new").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_ENCRYPTED");
    }

    #[tokio::test]
    async fn set_passphrase_rejects_ssh_key() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(unencrypted_ssh_key, None)
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        let err = store.set_passphrase("new").await.unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    }

    #[tokio::test]
    async fn change_passphrase_rejects_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("old"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        assert_eq!(
            store.change_passphrase("", "new").await.unwrap_err().code,
            "IDENTITY_NOT_ENCRYPTED"
        );
        assert_eq!(
            store.change_passphrase("old", "").await.unwrap_err().code,
            "IDENTITY_NOT_ENCRYPTED"
        );
    }

    // ── validate_passphrase (biometric enable) ───────────────────────

    #[tokio::test]
    async fn validate_passphrase_accepts_correct_ssh_passphrase() {
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        store
            .validate_passphrase("test-passphrase")
            .await
            .expect("correct SSH passphrase must validate");
    }

    #[tokio::test]
    async fn validate_passphrase_rejects_wrong_ssh_passphrase() {
        // Enabling biometric with a wrong SSH passphrase must fail before
        // the passphrase is sealed into the Keystore.
        let encrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(encrypted_ssh_key, None).await.unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
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
        let config = Config::new(dir.path().to_path_buf(), None);
        // Save an age-encrypted identity (uses a fixed test recipient).
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("correct-pw"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        let err = store.validate_passphrase("nope").await.unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }

    // ── auto-lock / clear-secs setters ──────────────────────────────────

    /// A store with a repo config on disk (the setters load + save repo.json).
    async fn store_with_repo_config() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", None)
            .await
            .unwrap();
        config
            .save_repo_config("https://x/repo", None, None, None, "/p")
            .await
            .unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        (store, dir)
    }

    #[tokio::test]
    async fn set_lock_mode_roundtrip_and_clamps_idle() {
        let (store, _d) = store_with_repo_config().await;

        // Idle secs below the minimum clamp up.
        let rc = store.set_lock_mode(LockMode::Idle(1)).await.unwrap();
        assert_eq!(rc.lock_mode, LockMode::Idle(LOCK_IDLE_SECS_MIN));
        // Idle secs above the maximum clamp down.
        let rc = store.set_lock_mode(LockMode::Idle(99_999)).await.unwrap();
        assert_eq!(rc.lock_mode, LockMode::Idle(LOCK_IDLE_SECS_MAX));
        // Never + Immediate pass through unchanged.
        let rc = store.set_lock_mode(LockMode::Never).await.unwrap();
        assert_eq!(rc.lock_mode, LockMode::Never);
        let rc = store.set_lock_mode(LockMode::Immediate).await.unwrap();
        assert_eq!(rc.lock_mode, LockMode::Immediate);
        // Persisted to disk.
        assert_eq!(store.config().await.unwrap().lock_mode, LockMode::Immediate);
    }

    #[tokio::test]
    async fn set_clear_secs_clamp_keep_never_and_default() {
        let (store, _d) = store_with_repo_config().await;

        // A nonzero value below the minimum clamps up; Never (0) is preserved.
        let rc = store.set_view_clear_secs(Some(1)).await.unwrap();
        assert_eq!(rc.view_clear_secs, Some(CLEAR_SECS_MIN));
        let rc = store.set_view_clear_secs(Some(0)).await.unwrap();
        assert_eq!(rc.view_clear_secs, Some(0), "Some(0) (Never) must be kept");
        // None clears the override (resolves to the default).
        let rc = store.set_view_clear_secs(None).await.unwrap();
        assert_eq!(rc.view_clear_secs, None);

        // Clipboard secs behave identically.
        let rc = store.set_clipboard_clear_secs(Some(999_999)).await.unwrap();
        assert_eq!(rc.clipboard_clear_secs, Some(CLEAR_SECS_MAX));
        let rc = store.set_clipboard_clear_secs(Some(0)).await.unwrap();
        assert_eq!(rc.clipboard_clear_secs, Some(0));
    }
}
