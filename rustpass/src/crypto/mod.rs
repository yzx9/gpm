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
//! Recipients *file* I/O (`list_recipients` / `write_recipients`) lives on the
//! [`StorageBackend`](crate::storage::StorageBackend), not here — crypto owns
//! recipient *semantics*, storage owns the recipients file.
//!
//! For now the age functions are also re-exported here so existing `crypto::`
//! callers (the Tauri command layer's `generate_age_identity`, the at-rest
//! `Config` layer, integration tests) keep resolving unchanged.

use async_trait::async_trait;
use tokio::task::spawn_blocking;
use zeroize::Zeroizing;

use crate::error::Error;

/// The age encryption backend (the sole `CryptoBackend` implementation today).
pub mod age;

#[allow(unused_imports)]
// re-export brings the age impl surface to `crypto::` for existing callers
// (src-tauri's `generate_age_identity`, integration tests, the at-rest
// `Config` layer). `Store` itself routes through `AgeBackend`, not these.
pub use age::*;

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
