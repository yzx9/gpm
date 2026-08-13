// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;
use std::str;
use std::sync::{Arc, Mutex};

use crate::crypto::{AgeBackend, BackendKind, CryptoBackend, GpgBackend, SecretExt};
use crate::error::{Error, ErrorCode};
use crate::recipient::Recipient;
use crate::storage::{RepoFiles, StorageBackend};

// Impl-split submodule: mod.rs is the shared scope for Store's split impl, so a
// super-glob is the idiomatic import (pedantic flags it; scoped allow).
#[allow(clippy::wildcard_imports)]
use super::*;

impl Store {
    /// Borrow the resolved storage backend, cloning its `Arc` out so the
    /// `Mutex` guard is dropped before any caller `.await`.
    ///
    /// Returns [`ErrorCode::BackendNotAvailable`] when the backend hasn't been
    /// resolved yet (pre-unlock, or after a resolve failure — `resolve_storage`
    /// stashes the specific error so the app can surface it).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BackendNotAvailable`] when `storage` is `None`;
    /// [`ErrorCode::StoreError`] on a poisoned lock (a panic mid-set).
    pub(super) fn storage(&self) -> Result<Arc<dyn StorageBackend>, Error> {
        let backend = self
            .storage
            .lock()
            .map_err(|_| Error::new(ErrorCode::StoreError, "storage backend lock poisoned"))?
            .clone();
        match backend {
            Some(b) => Ok(b),
            // No backend — surface the stashed resolve error if any (the
            // specific reason: unregistered ext:, tampered config, …),
            // else a generic "not resolved".
            None => Err(self
                .resolve_err
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| {
                    Error::new(
                        ErrorCode::BackendNotAvailable,
                        "storage backend not resolved (awaiting app unlock)",
                    )
                })),
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
    pub(super) fn resolve_and_set(&self, backend: Option<&str>, root: &str) -> Result<(), Error> {
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
    fn stash_err(slot: &Mutex<Option<Error>>, err: Error) {
        if let Ok(mut s) = slot.lock() {
            *s = Some(err);
        }
    }

    /// Clear the stashed resolve error for `slot` (a working backend supersedes
    /// it, or `reset` tears everything down).
    fn clear_err(slot: &Mutex<Option<Error>>) {
        if let Ok(mut s) = slot.lock() {
            *s = None;
        }
    }

    /// Borrow the resolved crypto backend, cloning its `Arc` out so the
    /// `Mutex` guard is dropped before any caller `.await`.
    ///
    /// Returns [`ErrorCode::BackendNotAvailable`] when the backend hasn't been
    /// resolved yet (pre-unlock, or after a resolve failure — `resolve_crypto`
    /// stashes the specific error so the app can surface it).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BackendNotAvailable`] when `crypto` is `None`;
    /// [`ErrorCode::StoreError`] on a poisoned lock (a panic mid-set).
    pub(super) fn crypto(&self) -> Result<Arc<dyn CryptoBackend>, Error> {
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
    /// match on [`RepoConfig::crypto`] (`BackendKind::Age` → `AgeBackend`,
    /// `BackendKind::Gpg` → `GpgBackend`). Intended post-unlock (sealed
    /// `repo.json` is readable once
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
        self.resolve_and_set_crypto(rc.crypto);
        Ok(())
    }

    /// Construct the typed crypto backend for `kind` and swap it in.
    /// `Age` → the age built-in; `Gpg` → the GPG built-in. Infallible: an invalid
    /// on-disk value can no longer reach here (the `BackendKind` deserializer
    /// rejects it at `repo.json` load — a `CONFIG_ERROR` at the first
    /// `load_repo_config`, rather than the prior runtime
    /// [`ErrorCode::BackendNotAvailable`] from this site), and a poisoned
    /// `crypto` lock is swallowed (leaving the slot unset, which `crypto()`
    /// then surfaces). Returns `()` rather than `Result`; if a future `ext:`
    /// crypto backend needs fallible construction, widen the signature then.
    pub(super) fn resolve_and_set_crypto(&self, kind: BackendKind) {
        let backend: Arc<dyn CryptoBackend> = match kind {
            BackendKind::Age => Arc::new(AgeBackend),
            BackendKind::Gpg => Arc::new(GpgBackend),
        };
        if let Ok(mut slot) = self.crypto.lock() {
            *slot = Some(backend);
        }
        Self::clear_err(&self.crypto_resolve_err);
    }

    /// The crypto backend's typed secret-file extension (`.age` today; `.gpg`
    /// once the GPG backend lands). Returned as [`SecretExt`] so a bare string
    /// can't be typo'd at a storage call site. `Store` threads this into `list`
    /// and builds passfile paths with it; `get`/`set`/`delete` take the built
    /// passfile, so they never name an extension.
    pub(super) fn secret_ext(&self) -> Result<SecretExt, Error> {
        Ok(self.crypto()?.profile().secret_extension)
    }

    /// Read + parse the recipients index, delegating the liveness guard and
    /// read+parse to the crypto backend through a [`RepoFiles`] view bound to
    /// the resolved storage backend.
    ///
    /// Returns empty for a genuinely-missing file (an uninitialized store).
    /// Every other failure mode is a hard error: tampered index, missing
    /// checkout, non-UTF-8, or an I/O error. Treating a tampered/escaping index
    /// as empty would silently shrink the recipient set on the next encrypt.
    ///
    /// The view (and therefore the guard) is bound to the backend's owned root,
    /// so the liveness check and the actual read source the SAME root — no
    /// per-op `local_path` vs owned-root gap.
    pub(super) async fn read_recipients_raw(&self) -> Result<Vec<Recipient>, Error> {
        let storage = self.storage()?;
        let crypto = self.crypto()?;
        let view = RepoFiles::new(&*storage);
        crypto.list_recipients(&view).await
    }

    /// The configured repo path, or an error if not configured.
    pub(super) async fn repo_path(&self) -> Result<PathBuf, Error> {
        let repo_config = self.config.load_repo_config().await?;
        Ok(Path::new(&repo_config.local_path).to_path_buf())
    }

    /// The full hash of the current HEAD commit, for provenance fields.
    pub(super) async fn current_head_hash(&self) -> Result<String, Error> {
        self.storage()?.current_head().await
    }

    /// Write a full-history Git bundle of the active repository to `out_path`
    /// — the export payload (R078). Packing objects never decrypts, so this
    /// runs under App Lock like [`list`](StorageBackend::list). Takes the
    /// cross-process [`repo_lock`](Self::repo_lock) so a separate-process
    /// `SyncWorker` pull can't advance refs mid-export; it is a read-only op,
    /// so no in-process `write_mu` is taken.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BackendNotAvailable`] before the storage backend resolves;
    /// [`ErrorCode::RepoBusy`] on cross-process lock contention; otherwise the
    /// storage backend's bundle error.
    pub async fn create_bundle(&self, out_path: &Path) -> Result<(), Error> {
        let _repo_lock = self.repo_lock()?;
        self.storage()?.create_bundle(out_path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::BackendKind;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    /// A symlink planted at the recipients index (dangling, or pointing outside
    /// the repo) must be rejected as tampering — not read as "uninitialized" →
    /// empty → silent recipient-set shrink on the next encrypt. The `lstat`
    /// guard behind `read_recipients_raw` (in `StorageBackend::file_liveness`)
    /// catches both symlink shapes without following them. Hits the private
    /// method directly (same module), so no `repo.json` setup is needed; the
    /// guard's root is the backend's owned root (pinned at `resolve_and_set`).
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
        store.resolve_and_set_crypto(BackendKind::Age);
        symlink(
            "/nonexistent/gpm-dangling",
            repo_dir.path().join(".age-recipients"),
        )
        .unwrap();
        let err = store.read_recipients_raw().await.unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "dangling symlink must be tampering, not an empty set"
        );

        // Out-of-repo symlink: lstat does not follow, so the regular-file check
        // rejects it before `read_file` could resolve + read the victim.
        let repo_dir2 = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let external_file = external.path().join("victim");
        std::fs::write(&external_file, b"age1stolen\n").unwrap();
        symlink(&external_file, repo_dir2.path().join(".age-recipients")).unwrap();
        let store2 = Store::new(repo_dir2.path().to_path_buf(), None);
        store2
            .resolve_and_set(Some("git"), &repo_dir2.path().to_string_lossy())
            .unwrap();
        store2.resolve_and_set_crypto(BackendKind::Age);
        let err = store2.read_recipients_raw().await.unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "escaping symlink must be tampering, not adopted"
        );

        // Sanity: a regular recipients index still reads (the lstat guard must
        // not reject a normal file).
        let repo_dir3 = tempfile::tempdir().unwrap();
        std::fs::write(repo_dir3.path().join(".age-recipients"), b"age1abc\n").unwrap();
        let store3 = Store::new(repo_dir3.path().to_path_buf(), None);
        store3
            .resolve_and_set(Some("git"), &repo_dir3.path().to_string_lossy())
            .unwrap();
        store3.resolve_and_set_crypto(BackendKind::Age);
        let got = store3.read_recipients_raw().await.unwrap();
        assert_eq!(got.len(), 1, "regular index still parses");

        // Missing index → empty (uninitialized store), unchanged.
        let repo_dir4 = tempfile::tempdir().unwrap();
        let store4 = Store::new(repo_dir4.path().to_path_buf(), None);
        store4
            .resolve_and_set(Some("git"), &repo_dir4.path().to_string_lossy())
            .unwrap();
        store4.resolve_and_set_crypto(BackendKind::Age);
        assert!(
            store4.read_recipients_raw().await.unwrap().is_empty(),
            "missing index is an uninitialized store, not an error"
        );

        // Configured-but-missing checkout (the backend's owned root is gone): a
        // bare "index absent" must NOT read as empty here — that would let
        // save_identity accept any identity against a store whose checkout it
        // can't see. Hard error instead.
        let missing_checkout = PathBuf::from("/tmp/gpm_no_such_checkout_12345_age_recipients");
        assert!(!missing_checkout.exists());
        let store5 = Store::new(missing_checkout.clone(), None);
        store5
            .resolve_and_set(Some("git"), &missing_checkout.to_string_lossy())
            .unwrap();
        store5.resolve_and_set_crypto(BackendKind::Age);
        assert_eq!(
            store5.read_recipients_raw().await.unwrap_err().code,
            "STORE_ERROR",
            "a missing configured checkout is an anomaly, not an empty store"
        );
    }

    /// An unresolved Store (no `resolve_and_set` / configure) surfaces
    /// `BackendNotAvailable` from `storage()` — not a panic, not a wrong backend.
    #[tokio::test]
    async fn unresolved_storage_returns_backend_not_available() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        // read_recipients_raw calls storage() directly (no repo_config load that
        // would mask the error).
        let err = store.read_recipients_raw().await.unwrap_err();
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
    fn resolve_and_set_crypto_picks_age() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(BackendKind::Age);
        let crypto = store.crypto().unwrap();
        assert_eq!(crypto.profile().backend_kind, BackendKind::Age);
        assert_eq!(crypto.profile().secret_extension.as_str(), ".age");
    }

    #[test]
    fn resolve_and_set_crypto_picks_gpg() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(BackendKind::Gpg);
        let crypto = store.crypto().unwrap();
        assert_eq!(crypto.profile().backend_kind, BackendKind::Gpg);
        assert_eq!(crypto.profile().secret_extension.as_str(), ".gpg");
    }

    // The prior `rejects_unknown_kind` / `surfaces_unknown_kind_via_crypto` tests
    // are gone: an invalid crypto value can no longer reach resolve_and_set_crypto
    // — BackendKind rejects it at repo.json deserialization (a CONFIG_ERROR at the
    // first load). That serde-level behavior is covered in config.rs
    // (repo_config_crypto_field_*).

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
        store.resolve_and_set_crypto(BackendKind::Gpg);
        assert!(store.crypto().is_ok(), "gpg backend resolved");
        store.reset().await.unwrap();
        let err = store.crypto().err().unwrap();
        assert_eq!(
            err.code, "BACKEND_NOT_AVAILABLE",
            "reset tears down the crypto slot"
        );
    }

    // resolve_crypto's soft-skip path (NoRepo/SealKeyUnavailable → Ok, with
    // crypto() then surfacing BackendNotAvailable) is covered by
    // resolve_crypto_soft_skips_when_no_repo above. The hard-error stash path
    // (a load error → stashed + returned) is a trivial branch not exercised
    // here; the invalid-crypto-value case is now a repo.json deserialization
    // error, covered at the serde level in config.rs
    // (repo_config_crypto_field_*), not here.
}
