// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use tokio::fs;
use zeroize::Zeroizing;

use crate::StoreBuilder;
use crate::config::{Config, RepoConfig};
use crate::crypto::CryptoBackend;
use crate::error::Error;
use crate::signing::AuthenticityConfig;
use crate::storage::{
    CancelSlot, CancelToken, GitAuth, StorageBackend, StorageCtx, StorageRegistry,
};

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
    /// [`resolve_crypto`](Self::resolve_crypto) runs. `Mutex` (not
    /// `tokio::sync`) because the guard is dropped before any `.await`:
    /// [`crypto`](Self::crypto)() clones the `Arc` out and releases.
    ///
    /// `Arc<dyn>` (not `Box`) so a cloned handle survives across the async
    /// encrypt/decrypt `.await`s without holding the mutex guard. Safe to share:
    /// every backend is a stateless unit struct (`AgeBackend`, `GpgBackend`) —
    /// `GpgBackend`'s keyring is read through `RepoFileView` per call, never held
    /// on the struct. A stateful backend would need re-review before sharing.
    crypto: Mutex<Option<Arc<dyn CryptoBackend>>>,
    /// The storage backend (git today; `ext:` extensions via the registry).
    /// Lazily resolved post-unlock — the backend type + root live in sealed
    /// `repo.json`, unreadable until app unlock — so `None` until
    /// [`resolve_storage`](Self::resolve_storage) or a setup path calls
    /// [`resolve_and_set`](Self::resolve_and_set). `Mutex` (not
    /// `tokio::sync`) because the guard is dropped before any `.await`:
    /// [`storage`](Self::storage)() clones the `Arc` out and releases.
    storage: Mutex<Option<Arc<dyn StorageBackend>>>,
    /// The most recent hard resolve failure (a tampered config, an unregistered
    /// `ext:` backend, …). Stashed by [`resolve_storage`](Self::resolve_storage)
    /// so [`storage`](Self::storage)() surfaces the specific reason instead of a
    /// generic `BackendNotAvailable`. Cleared on a successful
    /// [`set_storage_backend`](Self::set_storage_backend) /
    /// [`clear_storage_backend`](Self::clear_storage_backend).
    resolve_err: Mutex<Option<Error>>,
    /// The most recent hard crypto-resolve failure (an unknown crypto kind in
    /// `repo.json`). Stashed by [`resolve_crypto`](Self::resolve_crypto) so
    /// [`crypto`](Self::crypto)() surfaces the specific reason instead of a
    /// generic `BackendNotAvailable`. Cleared on a successful
    /// [`resolve_crypto`](Self::resolve_crypto) /
    /// [`clear_crypto_backend`](Self::clear_crypto_backend).
    crypto_resolve_err: Mutex<Option<Error>>,
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
    write_mu: tokio::sync::Mutex<()>,
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

/// RAII guard that arms a [`CancelSlot`] with a token on construction and clears
/// it on drop. Constructed INSIDE a `write_mu` critical section so the running
/// op's token — not a queued op's — is what `cancel_git` targets. The guard
/// outlives the network phases and disarms before the `write_mu`
/// guard drops.
struct ArmedSlot {
    slot: CancelSlot,
}

impl ArmedSlot {
    fn arm(slot: CancelSlot, token: CancelToken) -> Self {
        *slot.lock().expect("cancel slot poisoned") = Some(token);
        Self { slot }
    }
}

impl Drop for ArmedSlot {
    fn drop(&mut self) {
        *self.slot.lock().expect("cancel slot poisoned") = None;
    }
}

mod app_state;
mod authenticity;
mod backend;
mod entries;
mod identity;
mod paging;
mod setup;
mod sync;

// Shared sync / write / commit-identity result types live in `crate::storage`
// (the future `StorageBackend` trait home) so the upcoming `Store` → trait edge
// doesn't form a `store ↔ storage` module cycle. Re-exported here for callers
// that still reach them via `rustpass::store::`.
// `list_entries` / `resolve_entry_path` were relocated to `storage::git`
// and are re-exported here so existing integration-test call sites
// (`store::list_entries`, `store::resolve_entry_path`) keep compiling unchanged.
pub use crate::storage::git::{list_entries, resolve_entry_path};
pub use crate::storage::{
    AuthenticityResult, CommitIdentity, DivergenceChoice, EntryConflictChoice, ExpectedEntry,
    ExpectedKind, SyncDivergence, SyncOutcome, SyncResult, WriteOutcome, WriteResult,
};
pub use app_state::{clamp_lock_mode, normalize_clear_secs};
pub use entries::RevisionContent;
pub use paging::{RankedPage, rank_entries, slice_page};

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
        StoreBuilder::new().build(config_dir, master_key)
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
            crypto: Mutex::new(None),
            storage: Mutex::new(None),
            resolve_err: Mutex::new(None),
            crypto_resolve_err: Mutex::new(None),
            registry,
            config: Config::new(config_dir, master_key),
            cached_identity: RwLock::new(None),
            write_mu: tokio::sync::Mutex::new(()),
            autosync: AtomicBool::new(true),
        }
    }

    /// Replace the **auth-free** master seal key at runtime (R064): gates
    /// `repo.json` + `app.json`, NOT the identity. The master is permanent and
    /// auth-free — `startup_master_key` loads it silently at launch (or defers
    /// it under App Lock until `app_unlock`), and it is **never** wiped on
    /// background. The identity lives under the separate biometric-gated vault
    /// key — see [`set_vault_key`](Self::set_vault_key) /
    /// [`Config::set_vault_key`].
    pub fn set_master_key(&self, master_key: Option<[u8; 32]>) {
        self.config.set_master_key(master_key);
    }

    /// Delegate for [`Config::set_vault_key`].
    pub fn set_vault_key(&self, vault_key: Option<[u8; 32]>) {
        self.config.set_vault_key(vault_key);
    }

    /// Delegate for [`Config::has_identity`] — whether an `identity` file exists.
    /// m0007 gates the legacy-alias delete on this. See [`Config::has_identity`].
    #[must_use]
    pub fn has_identity(&self) -> bool {
        self.config.has_identity()
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

    /// Whether a master key is currently in memory (a real envelope can be
    /// produced right now). See [`Config::has_master_key`]. The `m0004` app-config
    /// split and the behavior setters gate on this.
    #[must_use]
    pub fn has_master_key(&self) -> bool {
        self.config.has_master_key()
    }

    /// Whether a vault key is in memory. See [`Config::has_vault_key`].
    #[must_use]
    pub fn has_vault_key(&self) -> bool {
        self.config.has_vault_key()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armed_slot_arms_on_construct_clears_on_drop() {
        // The load-bearing contract for the lock-scoped arming: the slot holds
        // the running op's token only while the guard lives, then clears — so a
        // queued op arming under the next critical section isn't clobbered.
        let slot: CancelSlot = Arc::new(Mutex::new(None));
        let token: CancelToken = Arc::new(AtomicBool::new(false));
        {
            let _armed = ArmedSlot::arm(slot.clone(), token.clone());
            assert!(
                slot.lock().unwrap().is_some(),
                "ArmedSlot::arm must publish the token into the slot"
            );
        }
        assert!(
            slot.lock().unwrap().is_none(),
            "ArmedSlot::drop must clear the slot"
        );
    }
}
