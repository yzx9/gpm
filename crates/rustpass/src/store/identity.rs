// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;
use std::str;
use std::sync::Arc;

use crate::crypto::{
    AgeBackend, CryptoBackend, GPG_RECIPIENTS_FILE, GpgBackend, RECIPIENTS_FILE, SecretExt,
};
use crate::error::{Error, ErrorCode};
use crate::identity::{self, IdentityType, classify_identity, validate_identity_format};
use crate::recipient::Recipient;
use crate::storage::RepoFiles;

// Impl-split submodule: mod.rs is the shared scope for Store's split impl, so a
// super-glob is the idiomatic import (pedantic flags it; scoped allow).
#[allow(clippy::wildcard_imports)]
use super::*;

impl Store {
    /// Delegate for [`Config::rekey_identity_to_vault`] (App-Lock enable / m0007).
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Config::rekey_identity_to_vault`].
    pub async fn rekey_identity_to_vault(&self) -> Result<(), Error> {
        self.config.rekey_identity_to_vault().await
    }

    /// Delegate for [`Config::rekey_identity_to_master`] (App-Lock disable).
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Config::rekey_identity_to_master`].
    pub async fn rekey_identity_to_master(&self) -> Result<(), Error> {
        self.config.rekey_identity_to_master().await
    }

    /// Delegate for [`Config::is_identity_under_master`] — the R064 crash-safety
    /// + m0007 idempotency probe. See [`Config::is_identity_under_master`].
    #[must_use]
    pub async fn is_identity_under_master(&self) -> bool {
        self.config.is_identity_under_master().await
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

    /// Delegate for [`Config::migrate_repo_seal`] — the headless background
    /// worker's repo-only seal migration (wraps `repo_config` only, skips the
    /// vault-tier files the pull-only worker must not touch).
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Config::migrate_repo_seal`].
    pub async fn migrate_repo_seal(&self) -> Result<(), Error> {
        self.config.migrate_repo_seal().await
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
    /// for encrypted SSH keys and S2K-protected GPG keys it validates without
    /// producing output; for plaintext or unencrypted identities it is a no-op
    /// success.
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
            IdentityType::SshEd25519 | IdentityType::SshRsa | IdentityType::PgpSecretKey => {
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
        // Touching repo_config here keeps the "repo not configured" surface —
        // the read itself no longer needs local_path (the backend owns the root
        // and the liveness guard runs behind it), but we still want a missing
        // repo.json to surface as `NoRepo` rather than reaching the backend.
        let _repo_config = self.config.load_repo_config().await?;
        self.read_recipients_raw().await
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
        let identity = identity::normalize_identity_text(identity);
        let identity_bytes = identity.as_bytes();
        validate_identity_format(identity_bytes)?;

        let itype = classify_identity(identity_bytes);

        // The backend kind this identity selects (R079 locked decision #1: the
        // identity TYPE drives backend selection): `"gpg"` for an OpenPGP secret
        // key, the age built-in (`None`) otherwise. There is no separate
        // "confirm backend" step.
        let kind: Option<&str> = match itype {
            IdentityType::PgpSecretKey => Some("gpg"),
            _ => None,
        };

        // Load the repo config once. `None` means no store is configured yet (a
        // fresh create / clone-first flow) — both the membership gate and the
        // crypto-kind persist below are skipped in that case.
        let repo = match self.config.load_repo_config().await {
            Ok(rc) => Some(rc),
            Err(e) if e.code == "NO_REPO" => None,
            Err(e) => return Err(e),
        };

        if let Some(rc) = &repo {
            let repo_path = Path::new(&rc.local_path).to_path_buf();

            // Flip-guard: refuse a backend switch that would orphan the store's
            // existing secrets of the other extension (they'd vanish from `list`
            // and from keep-mine replay) OR its recipients marker — a store
            // defined by its marker but with zero secrets of that extension yet
            // (a freshly-initialized store) must still not be silently flipped, or
            // the marker becomes a relic and future secrets of the orphaned
            // extension are invisible. The backend self-defends so a UI bug or
            // future re-setup flow can't silently brick the store.
            self.assert_no_conflicting_secrets(kind, &repo_path).await?;

            // Membership gate on a TRANSIENT backend (shared with the setup pick
            // step's preview probe via [`Self::probe_membership`]). x25519/GPG
            // derive their recipient from public-packet data (no passphrase);
            // only an encrypted SSH key needs its passphrase. `None` (no
            // recipients to match against) and `Some(true)` pass; `Some(false)`
            // rejects; `Err` (incomplete `.public-keys` pool, tampered index)
            // propagates rather than silently allowing.
            let recipient_passphrase = match itype {
                IdentityType::SshEd25519 | IdentityType::SshRsa => passphrase,
                _ => None,
            };
            if let Some(false) = self
                .probe_membership(identity, kind, recipient_passphrase)
                .await?
            {
                return Err(Error::new(
                    ErrorCode::InvalidIdentity,
                    "Identity does not match any recipient in the repository",
                ));
            }
        }

        // Only native x25519 keys support optional seal encryption; SSH and GPG
        // keys are stored as-is (they rely on their own native passphrase
        // protection), matching age's design. GPG must NOT get an age-encryption
        // layer here — that would scrypt-encrypt the S2K armor and brick the
        // identity at rest.
        let storage_passphrase = match itype {
            IdentityType::SshEd25519 | IdentityType::SshRsa | IdentityType::PgpSecretKey => None,
            _ => passphrase,
        };

        // Save the identity FIRST (the durable fact), then correct `repo.json`'s
        // crypto kind to match it. `save_identity` is the crypto-persistence
        // authority: it sets `crypto` SYMMETRICALLY — age → `None`, gpg →
        // `Some("gpg")` — writing only when the value changes (a fresh age setup
        // already has `crypto: None`, so no churn), so a stale kind left by an
        // earlier partial save self-heals on the next save. Saving the identity
        // before the kind persist means a persist failure leaves identity +
        // stale-kind (recoverable by re-saving) — never `crypto=gpg` stranded
        // with no identity. The kind persist + slot swap run only when a repo is
        // configured; the create/clone-first flow has no `repo.json` yet (its
        // later clone writes `crypto: None`).
        self.config
            .save_identity(identity_bytes, storage_passphrase)
            .await?;
        if let Some(mut rc) = repo {
            if rc.crypto.as_deref() != kind {
                rc.crypto = kind.map(str::to_string);
                self.config.save_repo_config_full(&rc).await?;
            }
            self.resolve_and_set_crypto(kind)?;
        }
        Ok(())
    }

    /// Refuse a crypto-backend flip that would orphan the store's existing
    /// secrets OR recipients marker of the other extension. If the identity
    /// being saved selects backend `kind` (`Some("gpg")` → `.gpg`, else `.age`),
    /// switching backends makes any secrets of the OTHER extension unreachable —
    /// they'd vanish from `list` and from keep-mine replay. The guard refuses
    /// when EITHER the orphaned extension has secret files already, OR its root
    /// recipients marker is present: a store defined by its marker but with zero
    /// secrets of that extension yet (a freshly-initialized store) must still not
    /// be silently flipped, or the marker becomes a relic and future secrets of
    /// the orphaned extension are invisible.
    async fn assert_no_conflicting_secrets(
        &self,
        kind: Option<&str>,
        repo_path: &Path,
    ) -> Result<(), Error> {
        // The backend orphaned by this identity + its root recipients marker.
        let (orphaned_ext, orphaned_marker) = match kind {
            Some("gpg") => (SecretExt::AGE, RECIPIENTS_FILE),
            _ => (SecretExt::GPG, GPG_RECIPIENTS_FILE),
        };
        let repo_path = repo_path.to_path_buf();
        let (count, marker_present) = tokio::task::spawn_blocking(move || {
            let count = list_entries(&repo_path, orphaned_ext).map_or(0, |v| v.len());
            // `metadata` (not `try_exists`) for MSRV-friendliness; a marker file
            // present is the signal. An unreadable marker degrades to "absent"
            // here, matching the secret-count's `map_or(0)` fail-open — the
            // marker check is additional defense, the secret count still applies.
            let marker_present = std::fs::metadata(repo_path.join(orphaned_marker)).is_ok();
            (count, marker_present)
        })
        .await
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("conflict scan: {e}")))?;
        if count > 0 || marker_present {
            let chosen = kind.unwrap_or("age");
            let orphaned = orphaned_ext.as_str().trim_start_matches('.');
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                format!(
                    "Can't save this {chosen} identity: the store is already set up for \
                     {orphaned} ({count} secret(s) and its {orphaned_marker} recipients file \
                     present). Saving it would switch the backend and make the store's \
                     {orphaned} secrets unreachable. Use an identity that matches the store's \
                     existing secrets.",
                ),
            ));
        }
        Ok(())
    }

    /// Membership probe on a TRANSIENT backend for `kind` — no shared-slot
    /// mutation. Returns:
    /// - `Ok(None)` — the store has no recipients to match against (an
    ///   uninitialized store), so there is nothing to gate on;
    /// - `Ok(Some(true))` / `Ok(Some(false))` — the identity is / is not a
    ///   recipient, definitively;
    /// - `Err(_)` — the recipient pool is incomplete or unreadable (the GPG
    ///   "repo looks incomplete" case).
    ///
    /// Shared by [`Self::save_identity`] (the authority gate) and
    /// [`Self::gpg_identity_preview`] (the setup pick probe). `passphrase` is the
    /// recipient-derivation passphrase (only an encrypted SSH key needs it; GPG
    /// ignores it — the fingerprint is public-packet data).
    pub(super) async fn probe_membership(
        &self,
        identity: &str,
        kind: Option<&str>,
        passphrase: Option<&str>,
    ) -> Result<Option<bool>, Error> {
        let storage = self.storage()?;
        let view = RepoFiles::new(&*storage);
        let backend: Arc<dyn CryptoBackend> = match kind {
            Some("gpg") => Arc::new(GpgBackend),
            _ => Arc::new(AgeBackend),
        };
        if backend.list_recipients(&view).await?.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            backend
                .identity_is_recipient(identity, passphrase, &view)
                .await?,
        ))
    }

    /// Get identity bytes for decryption.
    ///
    /// Checks cache first (for encrypted identities that have been unlocked),
    /// then falls back to loading from disk (for plaintext identities).
    pub(super) async fn get_identity_bytes(&self) -> Result<Vec<u8>, Error> {
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
            IdentityType::SshEd25519 | IdentityType::SshRsa | IdentityType::PgpSecretKey => {
                return Err(Error::new(
                    ErrorCode::IdentityNotEncrypted,
                    "SSH and GPG keys are not re-encrypted by gpm; use the key's native passphrase",
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
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::crypto;

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
            !crypto::is_ssh_identity_encrypted(pem.as_bytes()),
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

    /// B4b: a GPG key must NOT be re-encrypted by gpm's age seal — `set_passphrase`
    /// rejects it (same brick bug class as the SSH rejection), reachable from
    /// Settings once a GPG store exists.
    #[tokio::test]
    async fn set_passphrase_rejects_gpg_key() {
        let (sk, _pk) =
            crypto::openpgp::generate_keypair("gpm test <test@gpm.local>", None).expect("keygen");
        let armor = crypto::openpgp::armor_secret_key(&sk).expect("armor");
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(armor.as_bytes(), None).await.unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        let err = store.set_passphrase("new").await.unwrap_err();
        assert_eq!(
            err.code, "IDENTITY_NOT_ENCRYPTED",
            "a GPG key must not be re-encrypted by gpm's age seal"
        );
    }

    /// B5: `validate_passphrase` checks a GPG key's S2K passphrase (the gate
    /// `enable_biometric_unlock` uses before sealing the passphrase). A wrong
    /// passphrase surfaces as `WRONG_PASSPHRASE`, not a silent success.
    #[tokio::test]
    async fn validate_passphrase_checks_gpg_s2k() {
        let passphrase = "test-gpg-passphrase";
        let (sk, _pk) =
            crypto::openpgp::generate_keypair("gpm test <test@gpm.local>", Some(passphrase))
                .expect("keygen");
        let armor = crypto::openpgp::armor_secret_key(&sk).expect("armor");
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        config.save_identity(armor.as_bytes(), None).await.unwrap();
        let store = Store::new(dir.path().to_path_buf(), None);
        store.resolve_and_set_crypto(Some("gpg")).unwrap();
        store
            .validate_passphrase(passphrase)
            .await
            .expect("the right GPG passphrase validates");
        let err = store.validate_passphrase("wrong").await.unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }
}
