// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Crypto backend abstraction.
//!
//! Home of the [`CryptoBackend`] trait — the swappable encryption-backend
//! interface, mirroring gopass's `internal/backend/crypto.go`. The sole
//! implementation today is [`AgeBackend`] (in [`age`]); `Store` holds a
//! `Box<dyn CryptoBackend>` and never touches the age library directly.
//!
//! Recipients *file* I/O is generic file I/O on
//! [`StorageBackend`](crate::storage::StorageBackend) (`read_file` /
//! `write_file_atomic`), not specific recipient methods — crypto owns the
//! recipient *format* (serialize/parse), storage owns the file. The parsed
//! recipients view is [`Store::list_recipients`].
//!
//! For now the age functions are also re-exported here so existing `crypto::`
//! callers (the Tauri command layer's `generate_age_identity`, the seal
//! `Config` layer, integration tests) keep resolving unchanged.

use async_trait::async_trait;
use tokio::task::spawn_blocking;
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};
use crate::identity::{IdentityType, classify_identity};
use crate::recipient::{Recipient, parse_recipients};
use crate::storage::{RecipientsIndexPresence, RepoFileView, validate_recipients_index_liveness};

/// The age encryption backend (the sole `CryptoBackend` implementation today).
pub mod age;

/// Low-level OpenPGP (rpgp) wrapper — the shared seam owning the `pgp` dep.
/// Holds GPG commit-signature verification (RFC 0009, live) and the crypto
/// primitives (RFC 0036, `#[allow(dead_code)]` until `GpgBackend` lands).
pub mod openpgp;

#[allow(unused_imports)]
// re-export brings the age impl surface to `crypto::` for existing callers
// (src-tauri's `generate_age_identity`, integration tests, the seal
// `Config` layer). `Store` itself routes through `AgeBackend`, not these.
pub use age::*;

// ── Per-backend profile ───────────────────────────────────────────────────

/// Which crypto backend a store uses. Phase 0-1 has only [`BackendKind::Age`]
/// (hardwired in `Store::new`); Phase 3 persists this in `repo.json` for
/// construction-time selection — which requires late binding, because the
/// master key is withheld until app unlock so sealed `repo.json` is unreadable
/// at `Store::new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// The age (X25519 / SSH) crypto backend — the sole implementation today.
    Age,
    /// The GPG/OpenPGP crypto backend (RFC 0036; lands once the trait reshape
    /// is in). `#[allow(dead_code)]` until `GpgBackend` exists.
    #[allow(dead_code)]
    Gpg,
}

/// A crypto backend's secret-file extension — a typed wrapper so a bare
/// `".age"` string can't be typo'd at a storage call site. Constructed from a
/// [`CryptoProfile`] (`profile.secret_extension`) or a well-known const
/// ([`SecretExt::AGE`]); there is no public string constructor, so the on-disk
/// value stays the backend's single source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretExt(&'static str);

impl SecretExt {
    /// The age secret extension (`.age`). The canonical age value —
    /// [`AgeBackend::profile`] reuses this so the on-disk extension has one
    /// source. Tests reference this const instead of a raw string.
    pub const AGE: Self = Self(".age");

    /// The dotted extension, e.g. `.age` / `.gpg`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Per-backend storage-facing naming properties (gopass `crypto` profile).
///
/// Grouped behind [`CryptoBackend::profile`] so storage consumes one struct
/// rather than N accessors, and so a second backend's naming (`.gpg`,
/// `.gpg-id`, `.public-keys/`) lands in one place (RFC 0036). Runtime state
/// (held identity, keyring) lives on the backend struct, never here.
#[derive(Debug, Clone, Copy)]
pub struct CryptoProfile {
    /// Which backend this is.
    pub backend_kind: BackendKind,
    /// Secret-file extension (`.age` / `.gpg`), typed so a bare string can't be
    /// typo'd at a storage call site.
    pub secret_extension: SecretExt,
    /// Recipients-index filename — gopass `crypto.IDFile()`
    /// (`.age-recipients` / `.gpg-id`).
    pub recipients_filename: &'static str,
    /// Armored-recipient pubkey directory (GPG's `.public-keys/`); `None` for
    /// backends whose recipient strings are self-describing (age).
    pub public_keys_dir: Option<&'static str>,
}

/// The age backend's recipients-index filename — gopass's canonical name.
/// Referenced by [`AgeBackend::profile`] so it is the single source of truth
/// (production code + in-crate tests cannot drift).
pub(crate) const RECIPIENTS_FILE: &str = ".age-recipients";

/// Swappable crypto backend (gopass `internal/backend/crypto.go` analogue).
///
/// Owns everything age-specific: encrypt/decrypt, recipient derivation, and
/// identity management. The trait is `Send + Sync` so `Box<dyn CryptoBackend>`
/// stays `Send + Sync` — required because [`crate::store::Store`] is held in a
/// Tauri `AppState` shared across async commands.
///
/// Blocking work (age encrypt/decrypt, scrypt, the SSH KDF) is the impl's
/// responsibility: each method wraps its CPU-bound step in `spawn_blocking`
/// internally, so callers await a plain `Result` with no double-`?` on the
/// `JoinError`. Pure-CPU helpers ([`CryptoBackend::identity_to_recipient`] and
/// [`CryptoBackend::is_ssh_identity_encrypted`]) stay synchronous.
#[async_trait]
pub trait CryptoBackend: Send + Sync {
    /// Per-backend storage-facing naming (extension, recipients filename,
    /// `.public-keys/` dir, kind). Storage consumes this to map entry names to
    /// `<name><ext>` and to locate the recipients index — kept on the backend
    /// (not the facade) so each backend owns its on-disk format.
    fn profile(&self) -> CryptoProfile;

    /// Encrypt `plaintext` to every recipient in `recipients`, returning binary
    /// (unarmored) age ciphertext — the on-disk gopass secret format.
    ///
    /// # Errors
    ///
    /// See [`age::encrypt_to_recipients`] — `InvalidIdentity` for an empty
    /// recipient list or an unparseable recipient, `PostQuantumNotSupported`
    /// for a post-quantum recipient, `PluginUnavailable` if a required
    /// `age-plugin-<name>` binary is missing, `DecryptFailed` on the rare
    /// internal age failure.
    async fn encrypt_to_recipients(
        &self,
        plaintext: &[u8],
        recipients: &[String],
    ) -> Result<Vec<u8>, Error>;

    /// Decrypt age `encrypted` bytes with `identity_bytes` (native x25519 or
    /// SSH private key; encrypted SSH keys need `passphrase`).
    ///
    /// # Errors
    ///
    /// See [`age::decrypt_bytes`] — `InvalidIdentity` for a malformed identity,
    /// `IdentityEncrypted` for an encrypted SSH key with no passphrase,
    /// `DecryptFailed` for a wrong identity / corrupted ciphertext,
    /// `PostQuantumNotSupported` for a PQ identity.
    async fn decrypt_bytes(
        &self,
        encrypted: &[u8],
        identity_bytes: &[u8],
        passphrase: Option<&str>,
    ) -> Result<Vec<u8>, Error>;

    /// Decrypt a passphrase-encrypted (scrypt) identity blob. The scrypt KDF is
    /// intentionally slow (~100 ms), so this runs on a blocking thread.
    ///
    /// # Errors
    ///
    /// `IdentityNotEncrypted` for an empty passphrase, `WrongPassphrase` for a
    /// bad passphrase, `DecryptFailed` for corrupted data.
    async fn decrypt_identity(&self, passphrase: &str, encrypted: &[u8]) -> Result<Vec<u8>, Error>;

    /// Produce the operational identity bytes from an at-rest identity under
    /// `passphrase`. Classifies `at_rest` and returns what the encrypt/decrypt
    /// primitives consume: an age-encrypted identity is scrypt-decrypted, an
    /// encrypted SSH key is decrypted to an unencrypted PEM, and anything else
    /// (plaintext x25519, unencrypted SSH) is returned as-is. The caller decides
    /// whether to cache the result — the backend holds no identity state.
    ///
    /// # Errors
    ///
    /// `WrongPassphrase` for an incorrect passphrase on an age-encrypted
    /// identity or SSH key; `InvalidIdentity` for a non-UTF-8 SSH identity;
    /// `SshKeyInvalid` for an unparseable SSH key; `DecryptFailed` for a
    /// corrupt age-encrypted blob.
    async fn unlock_identity(&self, at_rest: &[u8], passphrase: &str) -> Result<Vec<u8>, Error>;

    /// Resolve the parsed recipients the backend can encrypt to, reading the
    /// backend's recipients index through `view`. The view carries the absolute
    /// repo root (for the liveness guard) and reads repo-relative files, so the
    /// backend needs nothing else to locate + parse its index. A future GPG
    /// backend resolves fingerprint recipients through its own keyring here.
    ///
    /// Returns an empty list for a genuinely-missing index (an uninitialized
    /// store) — matching gopass, so setup can proceed.
    ///
    /// # Errors
    ///
    /// `StoreError` for a tampered index (non-regular file) or a missing
    /// configured checkout; `IoError` on a metadata failure; `StoreError` for a
    /// non-UTF-8 index.
    async fn list_recipients(&self, view: &dyn RepoFileView) -> Result<Vec<Recipient>, Error>;

    /// Encrypt `plaintext` to every recipient in the store's index plus the
    /// identity's own recipient (gopass `ensureOurKeyID`), reading the index
    /// through `view`. `identity` is the operational (already-unlocked) identity
    /// bytes — the caller supplies the cached/plaintext form, so no passphrase.
    ///
    /// # Errors
    ///
    /// See [`encrypt_to_recipients`] and [`Self::list_recipients`].
    async fn encrypt(
        &self,
        plaintext: &[u8],
        identity: &[u8],
        view: &dyn RepoFileView,
    ) -> Result<Vec<u8>, Error>;

    /// Decrypt `ciphertext` with `identity` (the operational, unlocked identity
    /// bytes — native x25519, an unencrypted SSH PEM, or a plaintext key). No
    /// passphrase: the caller unlocks encrypted identities ahead of time.
    ///
    /// # Errors
    ///
    /// See [`decrypt_bytes`] — `InvalidIdentity`, `DecryptFailed`.
    async fn decrypt(&self, ciphertext: &[u8], identity: &[u8]) -> Result<Vec<u8>, Error>;

    /// Validate `passphrase` against an SSH identity without producing output.
    /// Used by the biometric-enable flow to reject a wrong passphrase before
    /// sealing it. Unencrypted keys succeed with any passphrase. The SSH KDF is
    /// blocking work, so this runs on a blocking thread.
    ///
    /// # Errors
    ///
    /// `WrongPassphrase` if the key is encrypted and `passphrase` is wrong,
    /// `InvalidIdentity` if the key can't be parsed.
    async fn validate_ssh_key_passphrase(
        &self,
        identity_bytes: &[u8],
        passphrase: &str,
    ) -> Result<(), Error>;

    /// Derive the public recipient string from an identity (native x25519 or
    /// SSH). Pure CPU op — synchronous.
    ///
    /// # Errors
    ///
    /// `InvalidIdentity` for an unparseable / unsupported identity,
    /// `IdentityEncrypted` for an encrypted SSH key with no passphrase,
    /// `PostQuantumNotSupported` / `PluginIdentityNotSupported` for the
    /// recognized-but-unsupported variants.
    fn identity_to_recipient(
        &self,
        identity: &str,
        passphrase: Option<&str>,
    ) -> Result<String, Error>;

    /// True iff `identity_bytes` is an SSH key whose private body is
    /// passphrase-encrypted. Pure CPU op — synchronous. See
    /// [`age::is_ssh_identity_encrypted`].
    fn is_ssh_identity_encrypted(&self, identity_bytes: &[u8]) -> bool;
}

/// The age crypto backend — the sole [`CryptoBackend`] implementation.
///
/// Stateless: every operation is a pure function of its arguments, so the unit
/// struct carries no configuration. The loaded identity and recipient set are
/// held by [`crate::store::Store`], not the backend; bytes flow through each
/// call. All blocking work (encrypt/decrypt, scrypt, the SSH KDF) is wrapped in
/// `spawn_blocking` inside the impl, so callers see a plain `Result`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AgeBackend;

#[async_trait]
impl CryptoBackend for AgeBackend {
    // Each method delegates to the age free fn of the same name (re-exported
    // into this module by `pub use age::*`). A bare call resolves to the free
    // fn, NOT this trait method — methods need a `self.` receiver — so these
    // are plain delegation, not recursion. Sync CPU work is wrapped in
    // `spawn_blocking`.

    fn profile(&self) -> CryptoProfile {
        CryptoProfile {
            backend_kind: BackendKind::Age,
            secret_extension: SecretExt::AGE,
            recipients_filename: RECIPIENTS_FILE,
            public_keys_dir: None,
        }
    }

    async fn encrypt_to_recipients(
        &self,
        plaintext: &[u8],
        recipients: &[String],
    ) -> Result<Vec<u8>, Error> {
        // Wrap secret copies in `Zeroizing` so they're scrubbed on drop after the
        // blocking op (CLAUDE.md: "All decrypted content uses Zeroizing and is
        // wiped after use"). Deref-coercion hands `&[u8]` / `&str` to the free fn.
        let plaintext = Zeroizing::new(plaintext.to_vec());
        let recipients = recipients.to_vec();
        spawn_blocking(move || encrypt_to_recipients(&plaintext, &recipients)).await?
    }

    async fn decrypt_bytes(
        &self,
        encrypted: &[u8],
        identity_bytes: &[u8],
        passphrase: Option<&str>,
    ) -> Result<Vec<u8>, Error> {
        let encrypted = encrypted.to_vec();
        let identity_bytes = Zeroizing::new(identity_bytes.to_vec());
        let passphrase = passphrase.map(|p| Zeroizing::new(p.to_string()));
        spawn_blocking(move || {
            decrypt_bytes(
                &encrypted,
                &identity_bytes,
                passphrase.as_deref().map(String::as_str),
            )
        })
        .await?
    }

    async fn decrypt_identity(&self, passphrase: &str, encrypted: &[u8]) -> Result<Vec<u8>, Error> {
        let passphrase = Zeroizing::new(passphrase.to_string());
        let encrypted = encrypted.to_vec();
        spawn_blocking(move || decrypt_identity(&passphrase, &encrypted)).await?
    }

    async fn unlock_identity(&self, at_rest: &[u8], passphrase: &str) -> Result<Vec<u8>, Error> {
        let itype = classify_identity(at_rest);
        if itype == IdentityType::AgeEncrypted {
            // scrypt unwrap runs on a blocking thread inside decrypt_identity.
            self.decrypt_identity(passphrase, at_rest).await
        } else if matches!(itype, IdentityType::SshEd25519 | IdentityType::SshRsa) {
            // Decrypt the SSH key to an unencrypted PEM; the bcrypt KDF is
            // blocking work. This cached form is what the decrypt path consumes
            // via age's no-KDF `Unencrypted` variant.
            let pw = Zeroizing::new(passphrase.to_string());
            let at_rest = at_rest.to_vec();
            let pem = spawn_blocking(move || {
                let raw = str::from_utf8(&at_rest).map_err(|_| {
                    Error::new(
                        ErrorCode::InvalidIdentity,
                        "SSH identity is not valid UTF-8",
                    )
                })?;
                crate::ssh::to_unencrypted_pem(raw, &pw)
            })
            .await??;
            Ok(pem.as_str().as_bytes().to_vec())
        } else {
            // Plaintext / unencrypted — already operational.
            Ok(at_rest.to_vec())
        }
    }

    async fn list_recipients(&self, view: &dyn RepoFileView) -> Result<Vec<Recipient>, Error> {
        let recipients_filename = self.profile().recipients_filename;
        let repo_path = view.repo_path();
        // Absent index → empty (uninitialized store); every other guard failure
        // (tampered index, missing checkout, I/O error) surfaces as a hard error.
        if let RecipientsIndexPresence::Present =
            validate_recipients_index_liveness(repo_path, recipients_filename).await?
        {
            let bytes = view.read(recipients_filename).await?;
            // Propagate a non-UTF-8 index as a hard error: parsing `""` → empty
            // set would `ensureOurKeyID` to only our key and silently drop every
            // other recipient on the next encrypt.
            let content = str::from_utf8(&bytes).map_err(|e| {
                Error::new(
                    ErrorCode::StoreError,
                    format!("recipients index is not valid UTF-8: {e}"),
                )
            })?;
            Ok(parse_recipients(content))
        } else {
            Ok(Vec::new())
        }
    }

    async fn encrypt(
        &self,
        plaintext: &[u8],
        identity: &[u8],
        view: &dyn RepoFileView,
    ) -> Result<Vec<u8>, Error> {
        // Recipients: everyone in the index, plus our own key (ensureOurKeyID).
        let mut recipients: Vec<String> = self
            .list_recipients(view)
            .await?
            .into_iter()
            .map(|r| r.public_key)
            .collect();
        let identity_str = str::from_utf8(identity)
            .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Identity is not valid UTF-8"))?;
        let our_recipient = self.identity_to_recipient(identity_str, None)?;
        if !recipients.iter().any(|r| r == &our_recipient) {
            recipients.push(our_recipient);
        }
        self.encrypt_to_recipients(plaintext, &recipients).await
    }

    async fn decrypt(&self, ciphertext: &[u8], identity: &[u8]) -> Result<Vec<u8>, Error> {
        // The caller (Store::get_identity_bytes) supplies the unlocked identity,
        // so no passphrase — same as the existing decrypt_bytes(.., None) path.
        self.decrypt_bytes(ciphertext, identity, None).await
    }

    async fn validate_ssh_key_passphrase(
        &self,
        identity_bytes: &[u8],
        passphrase: &str,
    ) -> Result<(), Error> {
        let identity_bytes = Zeroizing::new(identity_bytes.to_vec());
        let passphrase = Zeroizing::new(passphrase.to_string());
        spawn_blocking(move || validate_ssh_key_passphrase(&identity_bytes, &passphrase)).await?
    }

    fn identity_to_recipient(
        &self,
        identity: &str,
        passphrase: Option<&str>,
    ) -> Result<String, Error> {
        crate::recipient::identity_to_recipient(identity, passphrase)
    }

    fn is_ssh_identity_encrypted(&self, identity_bytes: &[u8]) -> bool {
        is_ssh_identity_encrypted(identity_bytes)
    }
}
