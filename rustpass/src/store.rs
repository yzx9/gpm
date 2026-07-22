// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::{fmt, str};

use nucleo_matcher::{
    Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use tokio::fs;
use tokio::sync::Mutex;
use tokio::task::spawn_blocking;
use zeroize::Zeroizing;

use crate::config::{Config, LockMode, RepoConfig};
use crate::crypto::{AgeBackend, CryptoBackend, GpgBackend, SecretExt};
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::identity::{IdentityType, classify_identity, validate_identity_format};
use crate::recipient::{Recipient, serialize_recipients};
use crate::secret::Secret;
use crate::signing::{
    self, AuthenticityConfig, CommitSigInfo, CommitSigStatus, TrustedGpgKey, TrustedKey, VerifyMode,
};
use crate::storage::git::passfile_rel;
use crate::storage::{
    CancelToken, CommitKind, GitAuth, KeepLocalOutcome, KeepLocalPlan, ProgressSender, RepoFiles,
    StorageBackend, StorageCtx, StorageRegistry,
};
use crate::template;

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

// Shared sync / write / commit-identity result types live in `crate::storage`
// (the future `StorageBackend` trait home) so the upcoming `Store` → trait edge
// doesn't form a `store ↔ storage` module cycle. Re-exported here for callers
// that still reach them via `rustpass::store::`.
pub use crate::storage::{
    AuthenticityResult, CommitIdentity, DivergenceChoice, SyncDivergence, SyncOutcome, SyncResult,
    WriteOutcome, WriteResult,
};
// `list_entries` / `resolve_entry_path` were relocated to `storage::git`
// and are re-exported here so existing integration-test call sites
// (`store::list_entries`, `store::resolve_entry_path`) keep compiling unchanged.
pub use crate::storage::git::{list_entries, resolve_entry_path};

/// Password store — aligned with `gopass.Store` interface.
///
/// Provides read-only operations on a gopass-compatible password store:
/// [`list`](Store::list), [`get`](Store::get), and [`sync`](Store::sync) (pull).
/// Supports optional passphrase-encrypted identity with in-memory caching.
pub struct Store {
    /// The crypto backend (age by default; GPG once `repo.json` selects it). The
    /// only path to encrypt/decrypt, recipient derivation, and identity
    /// management — `Store` never touches the age/GPG libraries directly.
    /// Lazily resolved post-unlock — the backend kind lives in sealed
    /// `repo.json`, unreadable until app unlock — so `None` until
    /// [`resolve_crypto`](Self::resolve_crypto) runs. `std::sync::Mutex` (not
    /// `tokio::sync`) because the guard is dropped before any `.await`:
    /// [`crypto`](Self::crypto)() clones the `Arc` out and releases.
    ///
    /// `Arc<dyn>` (not `Box`) so a cloned handle survives across the async
    /// encrypt/decrypt `.await`s without holding the mutex guard. Safe to share:
    /// every backend is a stateless unit struct (`AgeBackend`, `GpgBackend`) —
    /// `GpgBackend`'s keyring is read through `RepoFileView` per call, never held
    /// on the struct. A stateful backend would need re-review before sharing.
    crypto: std::sync::Mutex<Option<Arc<dyn CryptoBackend>>>,
    /// The storage backend (git today; `ext:` extensions via the registry).
    /// Lazily resolved post-unlock — the backend type + root live in sealed
    /// `repo.json`, unreadable until app unlock — so `None` until
    /// [`resolve_storage`](Self::resolve_storage) or a setup path calls
    /// [`resolve_and_set`](Self::resolve_and_set). `std::sync::Mutex` (not
    /// `tokio::sync`) because the guard is dropped before any `.await`:
    /// [`storage`](Self::storage)() clones the `Arc` out and releases.
    storage: std::sync::Mutex<Option<Arc<dyn StorageBackend>>>,
    /// The most recent hard resolve failure (a tampered config, an unregistered
    /// `ext:` backend, …). Stashed by [`resolve_storage`](Self::resolve_storage)
    /// so [`storage`](Self::storage)() surfaces the specific reason instead of a
    /// generic `BackendNotAvailable`. Cleared on a successful
    /// [`set_storage_backend`](Self::set_storage_backend) /
    /// [`clear_storage_backend`](Self::clear_storage_backend).
    resolve_err: std::sync::Mutex<Option<Error>>,
    /// The most recent hard crypto-resolve failure (an unknown crypto kind in
    /// `repo.json`). Stashed by [`resolve_crypto`](Self::resolve_crypto) so
    /// [`crypto`](Self::crypto)() surfaces the specific reason instead of a
    /// generic `BackendNotAvailable`. Cleared on a successful
    /// [`resolve_crypto`](Self::resolve_crypto) /
    /// [`clear_crypto_backend`](Self::clear_crypto_backend).
    crypto_resolve_err: std::sync::Mutex<Option<Error>>,
    /// The backend registry (built-ins + `ext:` extensions). Injected by
    /// [`StoreBuilder::build`](crate::storage::StoreBuilder::build) and consulted
    /// at resolve time. Immutable after construction.
    registry: Arc<StorageRegistry>,
    config: Config,
    /// Cached decrypted identity (populated after unlock).
    cached_identity: RwLock<Option<Zeroizing<Vec<u8>>>>,
    /// Serializes all repo-mutating operations (writes via [`autosync_write`],
    /// pull, push, divergence resolution) so two in-flight mutations can't race
    /// the git index or let a reviewed divergence go stale vs local HEAD
    /// mid-resolution. Public mutation entry points acquire it; the orchestrator
    /// acquires it once and composes the lock-free `*_locked` inners.
    write_mu: Mutex<()>,
    /// Cached app-scoped `autosync` flag — the only app-scoped pref `rustpass`
    /// still consumes (`autosync_write` reads it). Owned by the app shell; seeded
    /// on startup and re-pushed on every mutation via [`Store::set_autosync`],
    /// mirroring [`Store::set_master_key`]. Defaults to `true` (a caller that
    /// never seeds gets today's fresh-repo behavior, not a silent regression).
    autosync: AtomicBool,
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

/// Owned per-op RCS context bundle. `Store` builds one from `RepoConfig` at the
/// start of an RCS op and lends a borrowing [`StorageCtx`] (via [`RcsCtx::ctx`])
/// to each storage-backend call. Owning the fields here lets the borrowed ctx
/// stay alive across the op's `await`s.
struct RcsCtx {
    /// Repo working-tree root.
    repo_path: PathBuf,
    /// Git remote credentials.
    auth: GitAuth,
    /// Repository authenticity policy.
    policy: AuthenticityConfig,
    /// Commit author name (app default if `None`).
    commit_name: Option<String>,
    /// Commit author email (app default if `None`).
    commit_email: Option<String>,
}

impl RcsCtx {
    /// The borrowing view the storage-backend trait methods take.
    fn ctx(&self) -> StorageCtx<'_> {
        StorageCtx {
            repo_path: &self.repo_path,
            auth: &self.auth,
            policy: &self.policy,
            commit_name: self.commit_name.as_deref(),
            commit_email: self.commit_email.as_deref(),
        }
    }
}

impl Store {
    /// Create a new `Store` backed by the given config directory, with only the
    /// built-in (git) storage backend. Equivalent to
    /// [`StoreBuilder::new().build(config_dir, master_key)`](crate::storage::StoreBuilder::build)
    /// — use [`StoreBuilder`](crate::storage::StoreBuilder) directly to register
    /// `ext:` extension backends.
    ///
    /// **Behavior note:** the storage backend is NOT constructed here (it lives
    /// in sealed `repo.json`, unreadable until app unlock). It is resolved
    /// lazily post-unlock via [`resolve_storage`](Self::resolve_storage), or by a
    /// setup path via [`resolve_and_set`](Self::resolve_and_set). Before that,
    /// [`storage`](Self::storage)() returns [`ErrorCode::BackendNotAvailable`].
    #[must_use]
    pub fn new(config_dir: PathBuf, master_key: Option<[u8; 32]>) -> Self {
        crate::storage::StoreBuilder::new().build(config_dir, master_key)
    }

    /// Construct a `Store` with an injected backend registry. The crate-private
    /// construction path used by
    /// [`StoreBuilder::build`](crate::storage::StoreBuilder::build); not public
    /// because extensions register through the builder, not here.
    #[must_use]
    pub(crate) fn with_registry(
        config_dir: PathBuf,
        master_key: Option<[u8; 32]>,
        registry: Arc<StorageRegistry>,
    ) -> Self {
        Self {
            crypto: std::sync::Mutex::new(None),
            storage: std::sync::Mutex::new(None),
            resolve_err: std::sync::Mutex::new(None),
            crypto_resolve_err: std::sync::Mutex::new(None),
            registry,
            config: Config::new(config_dir, master_key),
            cached_identity: RwLock::new(None),
            write_mu: Mutex::new(()),
            autosync: AtomicBool::new(true),
        }
    }

    /// Borrow the resolved storage backend, cloning its `Arc` out so the
    /// `std::sync::Mutex` guard is dropped before any caller `.await`.
    ///
    /// Returns [`ErrorCode::BackendNotAvailable`] when the backend hasn't been
    /// resolved yet (pre-unlock, or after a resolve failure — `resolve_storage`
    /// stashes the specific error so the app can surface it).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BackendNotAvailable`] when `storage` is `None`;
    /// [`ErrorCode::StoreError`] on a poisoned lock (a panic mid-set).
    fn storage(&self) -> Result<Arc<dyn StorageBackend>, Error> {
        let backend = self
            .storage
            .lock()
            .map_err(|_| Error::new(ErrorCode::StoreError, "storage backend lock poisoned"))?
            .clone();
        match backend {
            Some(b) => Ok(b),
            None => {
                // No backend — surface the stashed resolve error if any (the
                // specific reason: unregistered ext:, tampered config, …),
                // else a generic "not resolved".
                Err(self
                    .resolve_err
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or_else(|| {
                        Error::new(
                            ErrorCode::BackendNotAvailable,
                            "storage backend not resolved (awaiting app unlock)",
                        )
                    }))
            }
        }
    }

    /// Swap in a resolved backend. Used by [`resolve_and_set`](Self::resolve_and_set),
    /// which the setup paths call to pin the git built-in.
    pub(crate) fn set_storage_backend(&self, backend: Arc<dyn StorageBackend>) {
        if let Ok(mut slot) = self.storage.lock() {
            *slot = Some(backend);
        }
        // A fresh, working backend supersedes any prior resolve error.
        Self::clear_err(&self.resolve_err);
    }

    /// Drop the resolved backend (set `storage` to `None`). Called first in
    /// [`Store::reset`] so post-reset ops get a clear `BackendNotAvailable`
    /// instead of operating against a torn-down repo. Marginal: `reset` does not
    /// hold `write_mu`, so an in-flight op that already cloned the `Arc` may
    /// still touch the old backend.
    pub(crate) fn clear_storage_backend(&self) {
        if let Ok(mut slot) = self.storage.lock() {
            *slot = None;
        }
        Self::clear_err(&self.resolve_err);
    }

    /// Resolve a backend of `backend` type rooted at `root` and swap it in.
    /// The single construction path for both the post-unlock resolve (which
    /// reads the type from `repo.json`) and the setup paths (which know the
    /// type they're configuring).
    fn resolve_and_set(&self, backend: Option<&str>, root: &str) -> Result<(), Error> {
        let resolved = self.registry.resolve(backend, root)?;
        self.set_storage_backend(Arc::from(resolved));
        Ok(())
    }

    /// Resolve the storage backend from the persisted `repo.json` config.
    /// Intended to be called post-unlock (once the master key is injected and
    /// `repo.json` is readable) — soft-skips when the config isn't readable yet
    /// (`NoRepo` pre-setup; `SealKeyUnavailable` under app-lock), mirroring
    /// [`Config::migrate_seal`].
    ///
    /// # Errors
    ///
    /// Soft-skips (`Ok`) on `NoRepo`/`SealKeyUnavailable`; otherwise propagates
    /// `load_repo_config`/`resolve` errors, stashing them internally (via
    /// `stash_resolve_err`) so [`storage`](Self::storage)() can surface the
    /// specific reason.
    pub async fn resolve_storage(&self) -> Result<(), Error> {
        let rc = match self.config.load_repo_config().await {
            Ok(rc) => rc,
            Err(e) if e.code == "NO_REPO" || e.code == "SEAL_KEY_UNAVAILABLE" => {
                // Not resolvable yet: pre-setup (no repo.json) or app-lock
                // (key withheld). Retry later — not an error. A soft-skip
                // carries no specific failure, so drop any error stashed by a
                // prior hard resolve (it's stale for this state).
                Self::clear_err(&self.resolve_err);
                return Ok(());
            }
            Err(e) => {
                Self::stash_err(&self.resolve_err, e.clone());
                return Err(e);
            }
        };
        match self.resolve_and_set(rc.backend.as_deref(), &rc.local_path) {
            Ok(()) => Ok(()),
            Err(e) => {
                Self::stash_err(&self.resolve_err, e.clone());
                Err(e)
            }
        }
    }

    /// Stash a hard resolve failure so the matching accessor surfaces the
    /// specific reason instead of a generic `BackendNotAvailable`. Shared by the
    /// storage and crypto resolve paths — pass the slot (`resolve_err` /
    /// `crypto_resolve_err`).
    fn stash_err(slot: &std::sync::Mutex<Option<Error>>, err: Error) {
        if let Ok(mut s) = slot.lock() {
            *s = Some(err);
        }
    }

    /// Clear the stashed resolve error for `slot` (a working backend supersedes
    /// it, or `reset` tears everything down).
    fn clear_err(slot: &std::sync::Mutex<Option<Error>>) {
        if let Ok(mut s) = slot.lock() {
            *s = None;
        }
    }

    /// Borrow the resolved crypto backend, cloning its `Arc` out so the
    /// `std::sync::Mutex` guard is dropped before any caller `.await`.
    ///
    /// Returns [`ErrorCode::BackendNotAvailable`] when the backend hasn't been
    /// resolved yet (pre-unlock, or after a resolve failure — `resolve_crypto`
    /// stashes the specific error so the app can surface it).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BackendNotAvailable`] when `crypto` is `None`;
    /// [`ErrorCode::StoreError`] on a poisoned lock (a panic mid-set).
    fn crypto(&self) -> Result<Arc<dyn CryptoBackend>, Error> {
        let backend = self
            .crypto
            .lock()
            .map_err(|_| Error::new(ErrorCode::StoreError, "crypto backend lock poisoned"))?
            .clone();
        match backend {
            Some(b) => Ok(b),
            None => Err(self
                .crypto_resolve_err
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| {
                    Error::new(
                        ErrorCode::BackendNotAvailable,
                        "crypto backend not resolved (awaiting app unlock)",
                    )
                })),
        }
    }

    /// Drop the resolved crypto backend (set the slot to `None`). Called in
    /// [`Store::reset`] so post-reset ops get a clear `BackendNotAvailable`
    /// instead of operating against a torn-down repo. Marginal: `reset` does not
    /// hold `write_mu`, so an in-flight op that already cloned the `Arc` may still
    /// touch the old backend — the same pre-existing race as
    /// `clear_storage_backend`.
    pub(crate) fn clear_crypto_backend(&self) {
        if let Ok(mut slot) = self.crypto.lock() {
            *slot = None;
        }
        Self::clear_err(&self.crypto_resolve_err);
    }

    /// Resolve the crypto backend from the persisted `repo.json` config — a typed
    /// match on [`RepoConfig::crypto`] (`None`/`"age"` → `AgeBackend`, `"gpg"` →
    /// `GpgBackend`). Intended post-unlock (sealed `repo.json` is readable once
    /// the master key is injected); soft-skips when the config isn't readable yet
    /// (`NoRepo` pre-setup; `SealKeyUnavailable` under app-lock), mirroring
    /// [`resolve_storage`](Self::resolve_storage). There is no `ext:` crypto
    /// namespace: both backends are rustpass-internal pure-Rust, so selection is a
    /// typed match, not a registry lookup.
    ///
    /// # Errors
    ///
    /// Soft-skips (`Ok`) on `NoRepo`/`SealKeyUnavailable`; otherwise propagates
    /// `load_repo_config`/resolve errors, stashing them internally so
    /// [`crypto`](Self::crypto)() can surface the specific reason.
    pub async fn resolve_crypto(&self) -> Result<(), Error> {
        let rc = match self.config.load_repo_config().await {
            Ok(rc) => rc,
            Err(e) if e.code == "NO_REPO" || e.code == "SEAL_KEY_UNAVAILABLE" => {
                // Not resolvable yet: pre-setup or app-lock. Retry later — not
                // an error. Drop any error stashed by a prior hard resolve.
                Self::clear_err(&self.crypto_resolve_err);
                return Ok(());
            }
            Err(e) => {
                Self::stash_err(&self.crypto_resolve_err, e.clone());
                return Err(e);
            }
        };
        match self.resolve_and_set_crypto(rc.crypto.as_deref()) {
            Ok(()) => Ok(()),
            Err(e) => {
                Self::stash_err(&self.crypto_resolve_err, e.clone());
                Err(e)
            }
        }
    }

    /// Construct the typed crypto backend for `kind` and swap it in.
    /// `None`/`"age"` → the age built-in; `"gpg"` → the GPG built-in; anything
    /// else → [`ErrorCode::BackendNotAvailable`] (an unknown crypto kind in
    /// `repo.json`).
    fn resolve_and_set_crypto(&self, kind: Option<&str>) -> Result<(), Error> {
        let backend: Arc<dyn CryptoBackend> = match kind {
            None | Some("age") => Arc::new(AgeBackend),
            Some("gpg") => Arc::new(GpgBackend),
            Some(other) => {
                // Clear any prior backend so crypto() surfaces THIS error
                // instead of a stale backend from a previous resolve.
                if let Ok(mut slot) = self.crypto.lock() {
                    *slot = None;
                }
                return Err(Error::new(
                    ErrorCode::BackendNotAvailable,
                    format!("unknown crypto backend {other:?} (expected \"age\" or \"gpg\")"),
                ));
            }
        };
        if let Ok(mut slot) = self.crypto.lock() {
            *slot = Some(backend);
        }
        Self::clear_err(&self.crypto_resolve_err);
        Ok(())
    }

    /// The crypto backend's typed secret-file extension (`.age` today; `.gpg`
    /// once the GPG backend lands). Returned as [`SecretExt`] so a bare string
    /// can't be typo'd at a storage call site. `Store` threads this into `list`
    /// and builds passfile paths with it; `get`/`set`/`delete` take the built
    /// passfile, so they never name an extension.
    fn secret_ext(&self) -> Result<SecretExt, Error> {
        Ok(self.crypto()?.profile().secret_extension)
    }

    /// The crypto backend's recipients-index filename (`.age-recipients` today).
    fn recipients_file(&self) -> Result<&'static str, Error> {
        Ok(self.crypto()?.profile().recipients_filename)
    }

    /// Read + parse the recipients index at `repo_path`, delegating the liveness
    /// guard + read + parse to the crypto backend through a [`RepoFiles`] view.
    /// Returns empty for a genuinely-missing file (an uninitialized store); every
    /// other failure (tampered index, missing checkout, non-UTF-8, I/O error) is a
    /// hard error — see [`crate::storage::validate_recipients_index_liveness`]
    /// for why "empty" is unsafe for a tampered/escaping index.
    async fn read_recipients_raw(&self, repo_path: &Path) -> Result<Vec<Recipient>, Error> {
        let storage = self.storage()?;
        let crypto = self.crypto()?;
        let view = RepoFiles::new(&*storage, repo_path);
        crypto.list_recipients(&view).await
    }

    /// Replace the seal master key at runtime. The app-launch biometric lock
    /// builds the store without the key (so `repo.json` is unreadable until the
    /// unlock prompt), injects it via this call after a successful biometric
    /// unlock, and wipes it (`None`) when the process is backgrounded. See
    /// [`Config::set_master_key`].
    pub fn set_master_key(&self, master_key: Option<[u8; 32]>) {
        self.config.set_master_key(master_key);
    }

    /// One-time migration: wrap any plaintext config files in the seal
    /// envelope. No-op on desktop (no master key) and for already-wrapped
    /// files. Safe to call on every startup.
    ///
    /// # Errors
    ///
    /// Returns an error if a file cannot be read, sealed/unsealed, or written.
    pub async fn migrate_seal(&self) -> Result<(), Error> {
        self.config.migrate_seal().await
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
    /// Returns true for age-encrypted identities, passphrase-protected SSH keys,
    /// and S2K-protected GPG keys. Returns false for plaintext x25519 keys and
    /// unprotected SSH/GPG keys. Fails closed (returns true) if the crypto
    /// backend isn't resolved, so the app prompts rather than skips.
    pub async fn is_identity_encrypted(&self) -> bool {
        let Ok(bytes) = self.config.load_identity().await else {
            return false;
        };
        let itype = classify_identity(&bytes);

        if itype == IdentityType::AgeEncrypted {
            return true;
        }

        if matches!(
            itype,
            IdentityType::SshEd25519 | IdentityType::SshRsa | IdentityType::PgpSecretKey
        ) {
            // Whether an SSH or GPG key needs a passphrase is a question for the
            // resolved crypto backend. Fail CLOSED on a missing backend: assume
            // encrypted so the app prompts for a passphrase rather than skipping
            // it. (Production resolves crypto at startup in init_state; this
            // guards the window after an unlock whose resolve_crypto failed.)
            return match self.crypto() {
                Ok(c) => c.identity_requires_passphrase(&bytes),
                Err(_) => true,
            };
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

        // Only encrypted identities populate the cache. Plaintext / unencrypted
        // identities decrypt straight from disk per-op (see `get_identity_bytes`),
        // so they report `is_unlocked() == false` — the unlock-status signal the
        // app's lock UI depends on. `unlock_identity` classifies again internally
        // and produces the operational bytes; the cache gate here preserves the
        // plaintext-never-cached invariant.
        if matches!(
            itype,
            IdentityType::AgeEncrypted
                | IdentityType::SshEd25519
                | IdentityType::SshRsa
                | IdentityType::PgpSecretKey
        ) {
            let zeroizing = self
                .crypto()?
                .unlock_identity(&encrypted_bytes, passphrase)
                .await?;
            let mut cache = self
                .cached_identity
                .write()
                .map_err(|_| Error::new(ErrorCode::StoreError, "Cache lock poisoned"))?;
            *cache = Some(zeroizing);
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

        // Prove the passphrase decrypts WITHOUT materializing key bytes where a
        // light validator exists. SSH keys go through `validate_ssh_key_passphrase`,
        // which decrypts in place and discards — it never serializes the PEM, so
        // the decrypted private key isn't left in a non-zeroized heap buffer (this
        // is the biometric-enable gate, a common flow). An age-encrypted identity
        // has no light validator, so `unlock_identity` scrypt-decrypts to the
        // operational key, returned as `Zeroizing` and dropped (wiped on drop).
        // Plaintext / unencrypted: nothing to validate.
        let crypto = self.crypto()?;
        match itype {
            IdentityType::AgeEncrypted => {
                crypto.unlock_identity(&bytes, passphrase).await?;
            }
            IdentityType::SshEd25519 | IdentityType::SshRsa => {
                crypto
                    .validate_identity_passphrase(&bytes, passphrase)
                    .await?;
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
    /// by the storage backend); `progress` receives transfer stats. Both are `None`
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
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        let auth = match (ssh_key, pat) {
            (Some(key), _) => GitAuth::Ssh {
                username: "git".to_string(),
                private_key: key.to_string(),
                passphrase: ssh_passphrase.map(String::from),
            },
            (_, Some(token)) => GitAuth::Pat(token.to_string()),
            _ => GitAuth::None,
        };

        let repo_dir = self.config.config_dir().join("repo");
        self.config.clear_all().await?;

        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }

        self.resolve_and_set(Some("git"), &repo_dir.to_string_lossy())?;
        self.resolve_and_set_crypto(None)?;
        self.storage()?
            .clone_repo(&auth, repo_url, &repo_dir, cancel, progress)
            .await?;

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
            self.resolve_and_set(Some("git"), &repo_dir.to_string_lossy())?;
            self.resolve_and_set_crypto(None)?;
            self.storage()?.init_repo(&repo_dir).await?;

            let recipients_bytes = serialize_recipients(&[recipient.to_string()]);
            self.storage()?
                .write_file_atomic(&repo_dir, self.recipients_file()?, &recipients_bytes)
                .await?;

            let message = format!("Initialized Store for {recipient}");
            let rel_paths = vec![self.recipients_file()?.to_string()];
            self.storage()?
                .commit_initial(&repo_dir, &rel_paths, &message)
                .await?;

            if has_url {
                self.storage()?.remote_add(&repo_dir, "origin", url).await?;
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
            // not leave the store looking initialized. Cleanup failures are
            // swallowed (the bootstrap error `e` is what we return) — log them.
            if let Err(cleanup) = fs::remove_dir_all(&repo_dir).await {
                log::warn!("create-store: partial-repo cleanup failed: {cleanup}");
            }
            if let Err(cleanup) = self.config.clear_all().await {
                log::warn!("create-store: config clear-all cleanup failed: {cleanup}");
            }
            return Err(e);
        }
        Ok(())
    }

    /// Read recipients from the cloned repository.
    ///
    /// Returns an empty list when the recipients index is absent (an
    /// uninitialized store) — matching gopass, so setup can proceed.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo is not configured or the recipients file
    /// exists but cannot be read.
    pub async fn list_recipients(&self) -> Result<Vec<Recipient>, Error> {
        let repo_config = self.config.load_repo_config().await?;
        let repo_path = Path::new(&repo_config.local_path);
        self.read_recipients_raw(repo_path).await
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
        let derived_recipient = self
            .crypto()?
            .identity_recipient(identity, recipient_passphrase)?;

        // Read the recipients to match the identity against. A tampered/corrupt
        // index on a configured repo (symlink, non-UTF-8, I/O error) must FAIL
        // here — the old `unwrap_or_default()` swallowed it to empty, skipping
        // the match and accepting any pasted identity against a store whose
        // recipients we could not actually read. The only tolerated case is
        // NO_REPO (no store configured yet — nothing to match against); a
        // genuine fresh store also reads as `Ok(empty)` (missing index).
        let known_recipients = match self.list_recipients().await {
            Ok(r) => r,
            Err(e) if e.code == "NO_REPO" => Vec::new(),
            Err(e) => return Err(e),
        };
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

        // Only native x25519 keys support optional seal encryption; SSH keys
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
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        // age-keygen writes # comment lines before the key; keep only the key
        // so it is parsed and stored consistently with the paste path.
        let identity = crate::identity::normalize_identity_text(identity);
        let identity_bytes = identity.as_bytes();
        validate_identity_format(identity_bytes)?;

        // A fresh/cloned store uses the age built-in; pin it before the identity
        // validation below touches the crypto backend. (A GPG store has its own
        // setup path; the post-unlock resolve corrects this default.)
        self.resolve_and_set_crypto(None)?;

        // Validate identity can derive a recipient (verifies key is usable)
        let _ = self
            .crypto()?
            .identity_recipient(identity, identity_passphrase)?;

        let auth = match (ssh_key, pat) {
            (Some(key), _) => GitAuth::Ssh {
                username: "git".to_string(),
                private_key: key.to_string(),
                passphrase: ssh_passphrase.map(String::from),
            },
            (_, Some(token)) => GitAuth::Pat(token.to_string()),
            _ => GitAuth::None,
        };

        let repo_dir = self.config.config_dir().join("repo");
        self.config.clear_all().await?;

        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }

        self.config.save_identity(identity_bytes, None).await?;

        self.resolve_and_set(Some("git"), &repo_dir.to_string_lossy())?;
        self.storage()?
            .clone_repo(&auth, repo_url, &repo_dir, cancel, progress)
            .await?;

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
        self.storage()?.list(repo_path, self.secret_ext()?).await
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
        let entries = self.storage()?.list(&repo_path, self.secret_ext()?).await?;
        let q = query.to_string();
        Ok(spawn_blocking(move || rank_entries(entries, &q)).await?)
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
        let entries = self.storage()?.list(&repo_path, self.secret_ext()?).await?;
        let q = query.to_string();
        Ok(spawn_blocking(move || slice_page(rank_entries(entries, &q), offset, limit)).await?)
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

        let encrypted = self
            .storage()?
            .get(repo_path, &passfile_rel(name, self.secret_ext()?))
            .await?;
        let identity_bytes = self.get_identity_bytes().await?;
        let crypto = self.crypto()?;
        let decrypted = crypto.decrypt(&encrypted, &identity_bytes).await?;
        Secret::parse(&decrypted)
    }

    /// Encrypt and write a secret to the store, then commit **locally** (no
    /// sync, no push).
    ///
    /// This is gopass's `set` (write) command, local-only. The plaintext is
    /// encrypted to every recipient in the store's `.age-recipients`, with our
    /// own key guaranteed to be among the encryption
    /// targets (mirroring gopass's `ensureOurKeyID`, so we can always read back
    /// what we wrote), written to `<name>.age`, and committed on the current
    /// branch. It does **not** pull or push — publishing is the caller's job.
    /// Production callers go through [`Store::autosync_write`], which wraps this
    /// in a pull → write → push and routes a rejected push to the sync-time
    /// divergence surface; calling `set` directly skips that serialization, so
    /// it is for tests and the orchestrator only.
    ///
    /// **Limitation:**
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
        let rcs = self.rcs_ctx().await?;
        let passfile = self
            .encrypt_and_write(name, plaintext, &rcs.repo_path)
            .await?;
        let head = self
            .commit_local(
                &rcs,
                CommitKind::Add,
                passfile,
                format!("Save secret: {name}"),
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
        let passfile = passfile_rel(name, self.secret_ext()?);
        let rcs = self.rcs_ctx().await?;

        // Existence + within-repo guard + remove the worktree file. The index
        // removal is staged in the commit below.
        self.storage()?.delete(&rcs.repo_path, &passfile).await?;

        let head = self
            .commit_local(
                &rcs,
                CommitKind::Remove,
                passfile,
                format!("Delete secret: {name}"),
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
    ///   The push is **not** cancellable today;
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
    /// Non-terminal outcomes are returned as [`WriteOutcome`] variants, not
    /// `Err`: [`WriteOutcome::AuthenticityBlocked`] when Enforce blocks the
    /// pre-write pull (HEAD unchanged), [`WriteOutcome::NeedsDivergenceResolve`]
    /// when the push is rejected (real divergence — the UI resolves via
    /// [`Self::resolve_sync_divergence`]). `Err` is a pull/push network error
    /// (the local commit survives, syncs later) or whatever `local_write`
    /// returns. [`WriteOutcome::Written`] is the normal success.
    pub async fn autosync_write<F, Fut>(
        &self,
        cancel: Option<CancelToken>,
        local_write: F,
    ) -> Result<WriteOutcome, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<WriteResult, Error>>,
    {
        // One critical section across pull → write → push. `set`/`delete` (the
        // local-only primitives the closure calls) do NOT re-acquire this guard.
        let _guard = self.write_mu.lock().await;

        let autosync = self.autosync.load(Ordering::Relaxed);
        if !autosync {
            return local_write().await.map(WriteOutcome::Written);
        }

        // Pull (cancellable). Divergence is benign — proceed and let the push
        // decide. Only an Enforce block aborts, before the write touches anything.
        match self.sync_with_locked(cancel, None).await? {
            SyncOutcome::FastForwarded(result) if result.authenticity.blocked => {
                return Ok(WriteOutcome::AuthenticityBlocked(result.authenticity));
            }
            _ => {}
        }

        // Local write (encrypt + commit), inside the critical section.
        let result = local_write().await?;

        // Push. Not cancellable today (RFC 0032). A PUSH_REJECTED is a real
        // divergence — surface it as NeedsDivergenceResolve with a fresh preview
        // so the UI can show the resolve modal without a second round-trip. A
        // network error leaves the local commit to sync later.
        match self.push_locked().await {
            Ok(()) => Ok(WriteOutcome::Written(result)),
            Err(e) if e.code == "PUSH_REJECTED" => {
                log::warn!("autosync: push rejected, surfacing divergence");
                Ok(WriteOutcome::NeedsDivergenceResolve(
                    self.sync_divergence_preview().await?,
                ))
            }
            Err(e) => Err(e),
        }
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
        resolve_entry_path(&repo_path, &passfile_rel(name, self.secret_ext()?))?;
        // Raw write primitive (no template), local-only via `set`.
        self.set(name, plaintext).await
    }

    /// Resolve a [`SyncOutcome::Diverged`] with the user's [`DivergenceChoice`].
    ///
    /// - [`DivergenceChoice::AdoptRemote`] adopts the reviewed remote tip exactly
    ///   (delegating to the storage backend).
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
                let rcs = self.rcs_ctx().await?;
                let expected = expected_remote_oid.to_string();
                self.storage()?.adopt_remote(&rcs.ctx(), &expected).await
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
        let rcs = self.rcs_ctx().await?;
        let expected = expected_remote_oid.to_string();

        // 1. Plan: fetch once, stale-guard, authenticity-verify, compute the
        //    replay set + conflict detection. Does NOT move HEAD.
        let plan = match self
            .storage()?
            .keep_local_plan(&rcs.ctx(), &expected)
            .await?
        {
            KeepLocalOutcome::Blocked(result) => return Ok(result),
            KeepLocalOutcome::Plan(p) => p,
        };
        let KeepLocalPlan {
            fetched_oid,
            replays,
            deletes,
            authenticity,
        } = plan;

        // 2. Decrypt each local blob to plaintext (identity). An undecryptable
        //    local entry can't be re-encrypted → refuse (adopt or cancel rather
        //    than silently drop it). `get_identity_bytes` returns the cached
        //    *unlocked* identity, so this works for passphrase-protected SSH keys
        //    (the PEM is already decrypted); the re-encrypt step (4) reuses it.
        let identity = self.get_identity_bytes().await?;
        let crypto = self.crypto()?;
        let mut decrypted: Vec<(String, Zeroizing<Vec<u8>>)> = Vec::with_capacity(replays.len());
        for r in replays {
            let plaintext = crypto.decrypt(&r.blob, &identity).await.map_err(|_| {
                Error::new(
                    ErrorCode::PushRejected,
                    format!(
                        "Can't keep mine: \"{}\" can't be decrypted to re-encrypt. \
                             Adopt the remote or cancel.",
                        r.rel_path.trim_end_matches(".age")
                    ),
                )
            })?;
            decrypted.push((r.rel_path, Zeroizing::new(plaintext)));
        }

        // 3. Advance to the reviewed remote tip — reuses the plan's fetched oid
        //    (objects still in the DB), so no second fetch can race past the
        //    reviewed tip and bypass the authenticity check under Enforce.
        let fetched = fetched_oid.clone();
        self.storage()?
            .keep_local_advance(&rcs.repo_path, &fetched)
            .await?;

        // 4. Re-encrypt to the CURRENT (remote-tip) recipients + our own key
        //    (ensureOurKeyID) via the backend. It re-reads the recipients index
        //    and re-derives our recipient per entry — cheap for age, and the
        //    replay set is small. The view binds to the advanced working tree.
        let storage = self.storage()?;
        let view = RepoFiles::new(&*storage, &rcs.repo_path);
        let mut ciphertexts: Vec<(String, Vec<u8>)> = Vec::with_capacity(decrypted.len());
        for (rel, plaintext) in decrypted {
            let ct = crypto.encrypt(&plaintext, &identity, &view).await?;
            ciphertexts.push((rel, ct));
        }

        // 5. Write the re-encrypted entries, apply local deletes, commit, push.
        let deletes = deletes.clone();
        let head = self
            .storage()?
            .keep_local_finalize(&rcs.ctx(), &ciphertexts, &deletes)
            .await?;

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
        let rcs = self.rcs_ctx().await?;
        self.storage()?.preview_divergence(&rcs.ctx()).await
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
        self.storage()?.lookup_template(&repo_path, name).await
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

    /// Commit `passfile` (the caller has already mutated the worktree) locally,
    /// with **no push**. `kind` is `Add` for a save or `Remove` for a delete.
    /// This is the local-only commit half shared by the local-only write
    /// primitives ([`Store::set`] / [`Store::delete`]).
    async fn commit_local(
        &self,
        rcs: &RcsCtx,
        kind: CommitKind,
        passfile: String,
        message: String,
    ) -> Result<String, Error> {
        self.storage()?
            .commit(&rcs.ctx(), kind, &[passfile], &message)
            .await
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
        let passfile = passfile_rel(name, self.secret_ext()?);

        // Encrypt to the store's recipients plus our own key (ensureOurKeyID),
        // reading the index through a view — the backend owns recipient
        // resolution + the encrypt step now.
        let identity_bytes = self.get_identity_bytes().await?;
        let storage = self.storage()?;
        let view = RepoFiles::new(&*storage, repo_path);
        let ciphertext = self
            .crypto()?
            .encrypt(plaintext, &identity_bytes, &view)
            .await?;

        storage.set(repo_path, &passfile, &ciphertext).await?;
        Ok(passfile)
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

        if matches!(
            classify_identity(&raw_bytes),
            IdentityType::AgeEncrypted | IdentityType::PgpSecretKey
        ) {
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
    /// Only native x25519 keys support seal encryption; SSH keys are
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

        // scrypt is intentionally slow (~100 ms+); the backend runs it on a
        // blocking thread. `unlock_identity` returns the decrypted key as
        // `Zeroizing`, so it's wiped after the re-encrypt instead of lingering
        // in the heap.
        let plaintext = self
            .crypto()?
            .unlock_identity(&encrypted_bytes, old_passphrase)
            .await?;
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
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
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
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let rcs = self.rcs_ctx().await?;
        self.storage()?.pull(&rcs.ctx(), cancel, progress).await
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
        let rcs = self.rcs_ctx().await?;
        self.storage()?.push(&rcs.ctx()).await
    }

    /// Manual sync (pull → push) — the publish path when autosync is off, and the
    /// "reconcile both directions" action behind the Sync button.
    ///
    /// Acquires [`write_mu`] for the whole pull → push. The pull phase is
    /// cancellable and surfaces [`SyncOutcome::Diverged`] (pull-side divergence)
    /// or an Enforce block (`FastForwarded` with `authenticity.blocked`, HEAD
    /// unchanged) without pushing. If the pull is clean, the push runs; a push
    /// rejection (someone pushed between our pull and our push — a race) is
    /// surfaced as [`SyncOutcome::Diverged`] with a fresh preview. On success the
    /// returned [`SyncResult`] reflects the pull (the push doesn't move local
    /// HEAD); a missing `origin` is a no-op at both phases (local-only store).
    ///
    /// # Errors
    ///
    /// Returns a network error from the pull or push (any local commit survives
    /// to sync later), or whatever [`sync_with_locked`] returns.
    pub async fn sync_repo(
        &self,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let _guard = self.write_mu.lock().await;

        // Pull (cancellable, progress-reporting). Hand back Diverged / an Enforce
        // block unchanged for the UI to resolve; otherwise keep the pull result
        // for the success return.
        let pull_result = match self.sync_with_locked(cancel, progress).await? {
            SyncOutcome::Diverged(d) => return Ok(SyncOutcome::Diverged(d)),
            SyncOutcome::FastForwarded(r) if r.authenticity.blocked => {
                return Ok(SyncOutcome::FastForwarded(r));
            }
            SyncOutcome::FastForwarded(r) => r,
        };

        // Push. A PUSH_REJECTED is a real divergence — surface it as Diverged
        // with a fresh preview. A network error leaves any local commits to sync
        // later. Push doesn't move local HEAD, so the pull result still reflects
        // the post-sync state.
        match self.push_locked().await {
            Ok(()) => Ok(SyncOutcome::FastForwarded(pull_result)),
            Err(e) if e.code == "PUSH_REJECTED" => {
                log::warn!("sync: push rejected, surfacing divergence");
                Ok(SyncOutcome::Diverged(self.sync_divergence_preview().await?))
            }
            Err(e) => Err(e),
        }
    }

    // ── Repository authenticity ───────────────────────────────────────────

    /// The configured repo path, or an error if not configured.
    async fn repo_path(&self) -> Result<PathBuf, Error> {
        let repo_config = self.config.load_repo_config().await?;
        Ok(Path::new(&repo_config.local_path).to_path_buf())
    }

    /// Load the current `RepoConfig` and build the per-op RCS context (repo
    /// path, auth, policy, commit identity). `RepoConfig` is stable for the op's
    /// duration — every caller runs under `write_mu` (or is a setup path with no
    /// concurrency).
    async fn rcs_ctx(&self) -> Result<RcsCtx, Error> {
        let repo_config = self.config.load_repo_config().await?;
        Ok(RcsCtx {
            repo_path: Path::new(&repo_config.local_path).to_path_buf(),
            auth: repo_config.to_git_auth(),
            policy: repo_config.authenticity,
            commit_name: repo_config.commit_user_name,
            commit_email: repo_config.commit_user_email,
        })
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
    /// trusted key (SSH **or** GPG) is recorded yet (Enforce with zero keys
    /// would block every pull). Returns the effective stored mode.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ConfigError`] if Enforce is requested with no
    /// trusted keys, or the config cannot be persisted.
    pub async fn set_verification_mode(&self, mode: VerifyMode) -> Result<VerifyMode, Error> {
        let mut rc = self.config.load_repo_config().await?;
        if mode == VerifyMode::Enforce && !rc.authenticity.has_any_trusted_key() {
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

    /// Push the app-scoped `autosync` flag into the [`Store`]'s cache — the
    /// value [`autosync_write`](Store::autosync_write) reads. The app shell owns
    /// the authoritative copy in `app.json`; this keeps the cached injection in
    /// sync. Call on startup, on the `set_autosync` command, and after the
    /// config-scope migration (the three mutation points).
    pub fn set_autosync(&self, enabled: bool) {
        self.autosync.store(enabled, Ordering::Relaxed);
    }

    /// The cached app-scoped `autosync` flag (the value [`autosync_write`] reads).
    /// Read accessor for tests/diagnostics — production reads it via
    /// [`autosync_write`](Store::autosync_write).
    #[must_use]
    pub fn autosync(&self) -> bool {
        self.autosync.load(Ordering::Relaxed)
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

    /// Seal the identity passphrase under the seal master key, for the
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

    /// Remove a trusted signing key by fingerprint. Removing the last trusted
    /// key of either kind (SSH or GPG) while in Enforce downgrades to Audit
    /// (Enforce with zero keys would block everything).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be persisted.
    pub async fn remove_trusted_key(&self, fingerprint: &str) -> Result<(), Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.authenticity
            .trusted_keys
            .retain(|k| k.fingerprint != fingerprint);
        if !rc.authenticity.has_any_trusted_key() && rc.authenticity.mode == VerifyMode::Enforce {
            rc.authenticity.mode = VerifyMode::Audit;
        }
        self.config.save_repo_config_full(&rc).await
    }

    /// Add a trusted GPG/OpenPGP public key (RFC 0009). Parses the armored
    /// block, derives the primary-key fingerprint, and dedupes — if a key with
    /// the same primary fingerprint is already trusted, the existing entry is
    /// returned unchanged (idempotent).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::SshKeyInvalid`] if the armor is unparseable or its
    /// self-signatures do not validate, or an error if the config cannot be
    /// persisted.
    pub async fn add_trusted_gpg_key(
        &self,
        armored_public_key: &str,
        label: &str,
    ) -> Result<TrustedGpgKey, Error> {
        // Bound the input before rpgp parses it — a mis-pasted/mis-picked
        // multi-MB blob is rejected with the same "not a usable GPG key" error
        // whether it arrives via paste or file import.
        if armored_public_key.len() > crate::MAX_GPG_KEY_FILE_BYTES {
            return Err(Error::new(
                ErrorCode::SshKeyInvalid,
                format!(
                    "GPG public key too large ({} bytes; limit {} bytes) — not an armored public key.",
                    armored_public_key.len(),
                    crate::MAX_GPG_KEY_FILE_BYTES
                ),
            ));
        }
        let key = crate::crypto::openpgp::parse_armored_public_key(armored_public_key)?;
        let fingerprint = crate::crypto::openpgp::primary_fingerprint(&key);

        let mut rc = self.config.load_repo_config().await?;
        if let Some(existing) = rc
            .authenticity
            .trusted_gpg_keys
            .iter()
            .find(|k| k.fingerprint == fingerprint)
            .cloned()
        {
            return Ok(existing);
        }

        let head = self.current_head_hash().await.unwrap_or_default();
        let entry = TrustedGpgKey {
            armored_public_key: armored_public_key.trim().to_string(),
            fingerprint,
            label: label.to_string(),
            added_at_commit: head,
        };
        rc.authenticity.trusted_gpg_keys.push(entry.clone());
        self.config.save_repo_config_full(&rc).await?;
        Ok(entry)
    }

    /// Remove a trusted GPG key by primary fingerprint. Removing the last
    /// trusted key of either kind (SSH or GPG) while in Enforce downgrades to
    /// Audit (Enforce with zero keys would block everything).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be persisted.
    pub async fn remove_trusted_gpg_key(&self, fingerprint: &str) -> Result<(), Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.authenticity
            .trusted_gpg_keys
            .retain(|k| k.fingerprint != fingerprint);
        if !rc.authenticity.has_any_trusted_key() && rc.authenticity.mode == VerifyMode::Enforce {
            rc.authenticity.mode = VerifyMode::Audit;
        }
        self.config.save_repo_config_full(&rc).await
    }

    /// The per-key parse warnings for the persisted trusted GPG keys — one
    /// human-readable string per entry that failed to re-parse. A trusted key
    /// that later breaks must not silently downgrade commits to
    /// `UnverifiedSignature`; the Settings card surfaces these so the user can
    /// re-add or remove the broken entry. Settings-load frequency only — the
    /// per-commit verifier path uses `TrustSet::from_config` (separate).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be read.
    pub async fn gpg_key_parse_warnings(&self) -> Result<Vec<String>, Error> {
        let rc = self.config.load_repo_config().await?;
        let armored = rc
            .authenticity
            .trusted_gpg_keys
            .iter()
            .map(|k| k.armored_public_key.as_str());
        let (_keys, warnings) = crate::crypto::openpgp::parse_trusted_keys(armored);
        Ok(warnings)
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
    pub async fn ignore_commit_issue(&self, commit: &str) -> Result<CommitSigInfo, Error> {
        let repo_path = self.repo_path().await?;
        let mut rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();

        // Derive the full CommitSigInfo once (a single signature verify). Its
        // status drives the is-issue check, and its metadata is returned to the
        // caller so the UI can refresh the row in place without a second IPC
        // (no write-then-re-read window).
        let commit_owned = commit.to_string();
        let repo_path_for_info = repo_path.clone();
        let info = spawn_blocking(move || {
            signing::commit_sig_info_at(&repo_path_for_info, &commit_owned, &trusted, &ignored)
        })
        .await??;

        // Record the ignore for a real issue (idempotent). A newly-written entry
        // means this commit is now ignored, so flip the returned flag.
        if info.status.is_issue() {
            let already = rc
                .authenticity
                .ignored
                .iter()
                .any(|i| i.commit == info.hash && i.status == info.status);
            if !already {
                let head = self.current_head_hash().await.unwrap_or_default();
                // Store the full resolved hash (`info.hash`), not the raw caller
                // input — `is_ignored` matches on the full OID, so a short hash or
                // revspec input would otherwise persist an entry that never matches
                // future verification.
                rc.authenticity.ignored.push(signing::IgnoredIssue {
                    commit: info.hash.clone(),
                    status: info.status.clone(),
                    ignored_at_commit: head,
                });
                self.config.save_repo_config_full(&rc).await?;
                return Ok(CommitSigInfo {
                    ignored: true,
                    ..info
                });
            }
        }
        Ok(info)
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
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        spawn_blocking(move || signing::head_status_at(&repo_path, &trusted)).await?
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
        spawn_blocking(move || signing::head_signer_public_key_at(&repo_path)).await?
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
        let public_key =
            spawn_blocking(move || signing::signer_public_key_at(&repo_path, &hash_owned))
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
        self.storage()?.current_head(&repo_path).await
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
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        let from_owned = from.to_string();
        let to_owned = to.to_string();
        spawn_blocking(move || {
            signing::verify_range_at(&repo_path, &from_owned, &to_owned, &trusted, &ignored)
        })
        .await?
    }

    /// The `limit` most recent commits (HEAD and ancestors, newest first) with
    /// per-commit verification status. Used by the `/history` screen.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn list_commit_signatures(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<signing::CommitSigPage, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        spawn_blocking(move || {
            signing::list_commit_signatures_at(&repo_path, offset, limit, &trusted, &ignored)
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
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        let hash_owned = commit_hash.to_string();
        spawn_blocking(move || {
            signing::commit_sig_info_at(&repo_path, &hash_owned, &trusted, &ignored)
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
        // Drop the resolved backends first so post-reset ops get a clear
        // `BackendNotAvailable` instead of touching a torn-down repo. Marginal:
        // `reset` doesn't hold `write_mu`, so an in-flight op that already cloned
        // an `Arc` may still hit the old backend (pre-existing destructive-reset
        // behavior; applies to storage and crypto alike).
        self.clear_storage_backend();
        self.clear_crypto_backend();

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

    /// Read + unseal `repo.json` and deserialize into `T` (see
    /// [`Config::load_repo_config_as`]). The config-scope migration uses this to
    /// read the legacy field shape.
    ///
    /// # Errors
    ///
    /// See [`Config::load_repo_config_as`].
    pub async fn load_repo_config_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        self.config.load_repo_config_as().await
    }
}

// ---------------------------------------------------------------------------
// Low-level functions (used by Store, also publicly accessible)
// ---------------------------------------------------------------------------

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
pub fn search_entries_in(
    repo_path: &Path,
    ext: SecretExt,
    query: &str,
) -> Result<Vec<Entry>, Error> {
    Ok(rank_entries(list_entries(repo_path, ext)?, query))
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

/// Build an `InvalidEntryName` error (keeps call sites terse).
fn invalid_name(message: &str) -> Error {
    Error::new(ErrorCode::InvalidEntryName, message)
}

/// Normalize a view/clipboard auto-clear override: `None` stays (default),
/// `Some(0)` stays (Never), any other `Some(n)` is clamped to
/// [`CLEAR_SECS_MIN`]..[`CLEAR_SECS_MAX`]. Infallible — out-of-range clamps
/// rather than erroring, since the UI sends only preset values. `pub` so the
/// app shell (which owns the app-scoped clear-timer setters post-scope-split)
/// applies the same rule.
#[must_use]
pub fn normalize_clear_secs(secs: Option<u64>) -> Option<u64> {
    match secs {
        None => None,
        Some(0) => Some(0),
        Some(n) => Some(n.clamp(CLEAR_SECS_MIN, CLEAR_SECS_MAX)),
    }
}

/// Clamp a [`LockMode::Idle`] timeout into
/// [`LOCK_IDLE_SECS_MIN`]..[`LOCK_IDLE_SECS_MAX`]; `Immediate` and `Never` pass
/// through. `pub` so the app shell's `lock_mode` setter applies the same rule
/// the old in-`Store` setter did.
#[must_use]
pub fn clamp_lock_mode(mode: LockMode) -> LockMode {
    match mode {
        LockMode::Idle(secs) => LockMode::Idle(secs.clamp(LOCK_IDLE_SECS_MIN, LOCK_IDLE_SECS_MAX)),
        other => other,
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

    /// A symlink planted at the recipients index (dangling, or pointing outside
    /// the repo) must be rejected as tampering — not read as "uninitialized" →
    /// empty → silent recipient-set shrink on the next encrypt. The `lstat` guard
    /// in `read_recipients_raw` catches both symlink shapes without following
    /// them. Hits the private method directly (same module), so no `repo.json`
    /// setup is needed.
    #[tokio::test]
    #[cfg(unix)]
    async fn read_recipients_raw_rejects_symlinked_index() {
        // Dangling symlink: lstat sees the symlink itself (not its missing
        // target) → not a regular file → hard error.
        let repo_dir = tempfile::tempdir().unwrap();
        let store = Store::new(repo_dir.path().to_path_buf(), None);
        store
            .resolve_and_set(Some("git"), &repo_dir.path().to_string_lossy())
            .unwrap();
        store.resolve_and_set_crypto(None).unwrap();
        symlink(
            "/nonexistent/gpm-dangling",
            repo_dir.path().join(".age-recipients"),
        )
        .unwrap();
        let err = store
            .read_recipients_raw(repo_dir.path())
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "dangling symlink must be tampering, not an empty set"
        );

        // Out-of-repo symlink: lstat does not follow, so the regular-file check
        // rejects it before `read_file` could resolve + read the victim.
        let repo_dir2 = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let external_file = external.path().join("victim");
        fs::write(&external_file, b"age1stolen\n").unwrap();
        symlink(&external_file, repo_dir2.path().join(".age-recipients")).unwrap();
        let store2 = Store::new(repo_dir2.path().to_path_buf(), None);
        store2
            .resolve_and_set(Some("git"), &repo_dir2.path().to_string_lossy())
            .unwrap();
        store2.resolve_and_set_crypto(None).unwrap();
        let err = store2
            .read_recipients_raw(repo_dir2.path())
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "escaping symlink must be tampering, not adopted"
        );

        // Sanity: a regular recipients index still reads (the lstat guard must
        // not reject a normal file).
        let repo_dir3 = tempfile::tempdir().unwrap();
        fs::write(repo_dir3.path().join(".age-recipients"), b"age1abc\n").unwrap();
        let store3 = Store::new(repo_dir3.path().to_path_buf(), None);
        store3
            .resolve_and_set(Some("git"), &repo_dir3.path().to_string_lossy())
            .unwrap();
        store3.resolve_and_set_crypto(None).unwrap();
        let got = store3.read_recipients_raw(repo_dir3.path()).await.unwrap();
        assert_eq!(got.len(), 1, "regular index still parses");

        // Missing index → empty (uninitialized store), unchanged.
        let repo_dir4 = tempfile::tempdir().unwrap();
        let store4 = Store::new(repo_dir4.path().to_path_buf(), None);
        store4
            .resolve_and_set(Some("git"), &repo_dir4.path().to_string_lossy())
            .unwrap();
        store4.resolve_and_set_crypto(None).unwrap();
        assert!(
            store4
                .read_recipients_raw(repo_dir4.path())
                .await
                .unwrap()
                .is_empty(),
            "missing index is an uninitialized store, not an error"
        );

        // Configured-but-missing checkout (repo.json pointed at a dir that's
        // gone): a bare "index absent" must NOT read as empty here — that would
        // let save_identity accept any identity against a store whose checkout
        // it can't see. Hard error instead.
        let missing_checkout = PathBuf::from("/tmp/gpm_no_such_checkout_12345_age_recipients");
        assert!(!missing_checkout.exists());
        let store5 = Store::new(missing_checkout.clone(), None);
        store5
            .resolve_and_set(Some("git"), &missing_checkout.to_string_lossy())
            .unwrap();
        store5.resolve_and_set_crypto(None).unwrap();
        assert_eq!(
            store5
                .read_recipients_raw(&missing_checkout)
                .await
                .unwrap_err()
                .code,
            "STORE_ERROR",
            "a missing configured checkout is an anomaly, not an empty store"
        );
    }

    #[test]
    fn list_entries_nonexistent_dir() {
        let missing = PathBuf::from("/tmp/gpm_no_such_dir_12345");
        assert!(!missing.exists());
        let result = list_entries(&missing, SecretExt::AGE);
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

    /// An unresolved Store (no `resolve_and_set` / configure) surfaces
    /// `BackendNotAvailable` from `storage()` — not a panic, not a wrong backend.
    #[tokio::test]
    async fn unresolved_storage_returns_backend_not_available() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        // read_recipients_raw calls storage() directly (repo_path passed
        // explicitly — no repo_config load that would mask the error).
        let err = store.read_recipients_raw(dir.path()).await.unwrap_err();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
    }

    /// A hard resolve failure (unregistered `ext:`) stashes the specific error
    /// so `storage()` surfaces the offending name, not a generic message.
    #[tokio::test]
    async fn resolve_storage_stashes_unregistered_backend_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        // Seed a repo.json pointing at an unregistered ext: backend.
        let rc = RepoConfig {
            url: String::new(),
            local_path: "/tmp".to_string(),
            backend: Some("ext:unregistered".to_string()),
            ..Default::default()
        };
        store.config.save_repo_config_full(&rc).await.unwrap();
        // Resolve fails (unregistered) and stashes the error.
        let err = store.resolve_storage().await.unwrap_err();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
        // storage() surfaces the stashed error, including the offending name.
        let stashed = store.storage().err().unwrap();
        assert_eq!(stashed.code, "BACKEND_NOT_AVAILABLE");
        assert!(
            stashed.message.contains("ext:unregistered"),
            "stashed error should name the unregistered backend: {stashed}"
        );
    }

    /// `resolve_storage` soft-skips (Ok) when there's no `repo.json` yet
    /// (pre-setup) — not an error. `storage()` stays unresolved.
    #[tokio::test]
    async fn resolve_storage_soft_skips_when_no_repo() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        // No repo.json — soft-skip, not an error.
        store.resolve_storage().await.unwrap();
        let err = store.storage().err().unwrap();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
    }

    #[tokio::test]
    async fn crypto_returns_backend_not_available_before_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        // Unresolved (no resolve_crypto / setup path yet) → a clear error, not a
        // panic or a silently-wrong default backend.
        let err = store.crypto().err().unwrap();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
    }

    #[test]
    fn resolve_and_set_crypto_picks_age_for_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(None).unwrap();
        let crypto = store.crypto().unwrap();
        assert_eq!(
            crypto.profile().backend_kind,
            crate::crypto::BackendKind::Age
        );
        assert_eq!(crypto.profile().secret_extension.as_str(), ".age");
    }

    #[test]
    fn resolve_and_set_crypto_picks_gpg_for_gpg() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(Some("gpg")).unwrap();
        let crypto = store.crypto().unwrap();
        assert_eq!(
            crypto.profile().backend_kind,
            crate::crypto::BackendKind::Gpg
        );
        assert_eq!(crypto.profile().secret_extension.as_str(), ".gpg");
    }

    #[test]
    fn resolve_and_set_crypto_rejects_unknown_kind() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        let err = store.resolve_and_set_crypto(Some("quux")).unwrap_err();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
        // A failed resolve leaves no backend — crypto() still errors.
        assert!(store.crypto().is_err());
    }

    #[tokio::test]
    async fn resolve_crypto_soft_skips_when_no_repo() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        // No repo.json — soft-skip, not an error (mirrors resolve_storage).
        store.resolve_crypto().await.unwrap();
        let err = store.crypto().err().unwrap();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
    }

    #[tokio::test]
    async fn reset_clears_crypto_backend() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(Some("gpg")).unwrap();
        assert!(store.crypto().is_ok(), "gpg backend resolved");
        store.reset().await.unwrap();
        let err = store.crypto().err().unwrap();
        assert_eq!(
            err.code, "BACKEND_NOT_AVAILABLE",
            "reset tears down the crypto slot"
        );
    }

    #[test]
    fn resolve_and_set_crypto_picks_age_for_explicit_age_string() {
        // The Some("age") arm is a documented input (mirrors None); cover it so a
        // refactor that dropped it (matching only None) would fail.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(Some("age")).unwrap();
        let crypto = store.crypto().unwrap();
        assert_eq!(
            crypto.profile().backend_kind,
            crate::crypto::BackendKind::Age
        );
        assert_eq!(crypto.profile().secret_extension.as_str(), ".age");
    }

    #[tokio::test]
    async fn resolve_crypto_surfaces_unknown_kind_via_crypto() {
        // Driving the full resolve_crypto path with an unknown kind must (a)
        // hard-fail and (b) leave crypto() surfacing the stashed unknown-kind
        // error — not a stale backend from a prior resolve.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(None).unwrap(); // seed a backend first
        assert!(store.crypto().is_ok());

        Config::new(dir.path().to_path_buf(), None)
            .save_repo_config_full(&RepoConfig {
                local_path: dir.path().join("repo").to_string_lossy().to_string(),
                crypto: Some("quux".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        store.resolve_crypto().await.unwrap_err();
        let err = store.crypto().err().unwrap();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
        assert!(
            err.message.contains("unknown crypto backend"),
            "crypto() must surface the stashed unknown-kind error, not a stale backend: {err}"
        );
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
        store.resolve_and_set_crypto(None).unwrap();
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
            !crate::crypto::is_ssh_identity_encrypted(pem.as_bytes()),
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
        store.resolve_and_set_crypto(None).unwrap();
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
        store.resolve_and_set_crypto(None).unwrap();
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
        let _crypto = crate::test_crypto_gate::crypto_permit().await;
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
        store.resolve_and_set_crypto(None).unwrap();
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
        store.resolve_and_set_crypto(None).unwrap();
        assert!(!store.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn save_identity_stores_ssh_key_as_plaintext_even_with_passphrase() {
        let unencrypted_ssh_key = b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(None).unwrap();

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
        let _crypto = crate::test_crypto_gate::crypto_permit().await;
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
        let _crypto = crate::test_crypto_gate::crypto_permit().await;
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
        store.resolve_and_set_crypto(None).unwrap();
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
        store.resolve_and_set_crypto(None).unwrap();
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
        let _crypto = crate::test_crypto_gate::crypto_permit().await;
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        // Save an age-encrypted identity (uses a fixed test recipient).
        config
            .save_identity(b"AGE-SECRET-KEY-1TEST", Some("correct-pw"))
            .await
            .unwrap();

        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(None).unwrap();
        let err = store.validate_passphrase("nope").await.unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }

    #[test]
    fn clamp_lock_mode_clamps_idle_and_passes_others() {
        // Idle secs below the minimum clamp up.
        assert_eq!(
            clamp_lock_mode(LockMode::Idle(1)),
            LockMode::Idle(LOCK_IDLE_SECS_MIN)
        );
        // Idle secs above the maximum clamp down.
        assert_eq!(
            clamp_lock_mode(LockMode::Idle(99_999)),
            LockMode::Idle(LOCK_IDLE_SECS_MAX)
        );
        // Never + Immediate pass through unchanged.
        assert_eq!(clamp_lock_mode(LockMode::Never), LockMode::Never);
        assert_eq!(clamp_lock_mode(LockMode::Immediate), LockMode::Immediate);
    }

    #[test]
    fn normalize_clear_secs_clamps_keeps_never_and_none() {
        // A nonzero value below the minimum clamps up; Never (0) is preserved.
        assert_eq!(normalize_clear_secs(Some(1)), Some(CLEAR_SECS_MIN));
        assert_eq!(
            normalize_clear_secs(Some(0)),
            Some(0),
            "Some(0) (Never) must be kept"
        );
        // None stays None (resolves to the default at read time).
        assert_eq!(normalize_clear_secs(None), None);
        // Values above the maximum clamp down.
        assert_eq!(normalize_clear_secs(Some(999_999)), Some(CLEAR_SECS_MAX));
    }

    #[test]
    fn autosync_cache_default_true_and_set_round_trips() {
        // Proves the injected `autosync` cache is what set_autosync writes and
        // autosync_write reads — the plumbing the app shell pushes into. Default
        // is true (a caller that never seeds gets today's fresh-repo behavior).
        let store = Store::new(std::env::temp_dir(), None);
        assert!(store.autosync(), "default is true");
        store.set_autosync(false);
        assert!(
            !store.autosync(),
            "set_autosync(false) must reach the cache autosync_write reads"
        );
        store.set_autosync(true);
        assert!(store.autosync());
    }
}
