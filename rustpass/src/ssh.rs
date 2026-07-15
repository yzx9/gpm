// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! SSH key generation, public key derivation, and private key export.
//!
//! Uses the `ssh-key` crate to generate ed25519 keypairs compatible with
//! standard OpenSSH tools and git2's SSH authentication.

use std::fmt;

use ssh_key::{Algorithm, LineEnding, PrivateKey, rand_core::OsRng};
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// A generated SSH keypair.
pub struct SshKeyPair {
    /// OpenSSH PEM-encoded private key (wiped on drop).
    pub private_key: Zeroizing<String>,
    /// OpenSSH public key string (`ssh-ed25519 AAAA...`).
    pub public_key: String,
}

/// Redacts `private_key` — mirrors `rustpass::Secret` so `Debug` never leaks
/// the PEM body (the derived `Debug` would print `Zeroizing<String>` verbatim).
impl fmt::Debug for SshKeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshKeyPair")
            .field("private_key", &"[REDACTED]")
            .field("public_key", &self.public_key)
            .finish()
    }
}

/// Generate a new ed25519 SSH keypair.
///
/// If `passphrase` is provided, the private key is encrypted using
/// bcrypt-pbkdf (standard OpenSSH encryption).
///
/// # Errors
///
/// Returns `SSH_KEY_INVALID` if key generation or encryption fails.
pub fn generate_keypair(passphrase: Option<&str>) -> Result<SshKeyPair, Error> {
    let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Key generation failed: {e}"),
        )
    })?;

    let final_key = match passphrase {
        Some(pw) if !pw.is_empty() => key.encrypt(&mut OsRng, pw).map_err(|e| {
            Error::new(
                ErrorCode::SshKeyInvalid,
                format!("Key encryption failed: {e}"),
            )
        })?,
        _ => key,
    };

    // PrivateKey::to_openssh returns Zeroizing<String> — wrap in our own Zeroizing
    let private_key = final_key.to_openssh(LineEnding::default()).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Private key serialization failed: {e}"),
        )
    })?;

    let public_key = final_key.public_key().to_openssh().map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Public key serialization failed: {e}"),
        )
    })?;

    Ok(SshKeyPair {
        private_key,
        public_key,
    })
}

/// Derive the public key from a private key PEM string.
///
/// The public key is always readable, even from encrypted private keys
/// (no passphrase needed to extract it).
///
/// # Errors
///
/// Returns `SSH_KEY_INVALID` if the private key cannot be parsed.
pub fn get_public_key(private_key_pem: &str) -> Result<String, Error> {
    let key = PrivateKey::from_openssh(private_key_pem).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Invalid private key: {e}"),
        )
    })?;

    key.public_key().to_openssh().map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Public key extraction failed: {e}"),
        )
    })
}

/// Validate and return a private key PEM string.
///
/// This is a security gate that confirms the key is valid before
/// returning it across IPC for export.
///
/// # Errors
///
/// Returns `SSH_KEY_INVALID` if the private key cannot be parsed.
pub fn export_private_key(private_key_pem: &str) -> Result<Zeroizing<String>, Error> {
    // Validate the key parses
    let key = PrivateKey::from_openssh(private_key_pem).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Invalid private key: {e}"),
        )
    })?;

    // Re-serialize to ensure consistent format — to_openssh returns Zeroizing<String>
    key.to_openssh(LineEnding::default()).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Private key serialization failed: {e}"),
        )
    })
}

/// Decrypt an SSH private key (if encrypted) and return it as an UNENCRYPTED
/// OpenSSH PEM, for caching after `Store::unlock`. The cached bytes let
/// `crypto::decrypt_bytes(.., None)` take age's no-KDF `Unencrypted` path,
/// collapsing the per-entry bcrypt KDF to a one-time unlock cost.
///
/// Handles **OpenSSH** format (`-----BEGIN OPENSSH PRIVATE KEY-----`) via
/// `ssh_key::PrivateKey::from_openssh`. This is the only format routed here:
/// `Store::is_identity_encrypted` returns `true` only when age classifies the
/// key as `Encrypted(_)`, which happens exclusively for OpenSSH-encrypted keys.
/// Legacy `-----BEGIN RSA PRIVATE KEY-----` PEM is never encrypted-classified
/// (age reads unencrypted PEM as `Unencrypted`, encrypted PEM as
/// `Unsupported`), so it never reaches `unlock()` — and unencrypted legacy RSA
/// still decrypts entries via the normal `get()` path without unlocking.
///
/// # Errors
///
/// Returns `WrongPassphrase` if the key is encrypted and `passphrase` is
/// incorrect (mirrors `crypto::validate_ssh_key_passphrase`). Returns
/// `SshKeyInvalid` if the key cannot be parsed or serialized.
pub fn to_unencrypted_pem(pem: &str, passphrase: &str) -> Result<Zeroizing<String>, Error> {
    let key = PrivateKey::from_openssh(pem.trim()).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Cannot parse SSH private key: {e}"),
        )
    })?;

    let key = if key.is_encrypted() {
        key.decrypt(passphrase)
            .map_err(|_| Error::new(ErrorCode::WrongPassphrase, "Wrong passphrase for SSH key"))?
    } else {
        key
    };

    key.to_openssh(LineEnding::default()).map_err(|e| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            format!("Private key serialization failed: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use age::{Decryptor, Encryptor, ssh};

    use super::*;

    #[test]
    fn generate_keypair_unencrypted() {
        let pair = generate_keypair(None).expect("generation should succeed");
        assert!(
            pair.private_key
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"),
            "private key should be OpenSSH PEM format"
        );
        assert!(
            pair.public_key.starts_with("ssh-ed25519 "),
            "public key should start with ssh-ed25519"
        );
    }

    #[test]
    fn generate_keypair_encrypted() {
        let pair =
            generate_keypair(Some("test-password")).expect("encrypted generation should succeed");
        assert!(
            pair.private_key
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"),
            "encrypted private key should be OpenSSH PEM format"
        );
        assert!(
            pair.public_key.starts_with("ssh-ed25519 "),
            "public key should be extractable from encrypted key"
        );
    }

    #[test]
    fn get_public_key_from_unencrypted() {
        let pair = generate_keypair(None).expect("generation should succeed");
        let pub_key =
            get_public_key(&pair.private_key).expect("public key extraction should succeed");
        assert_eq!(pub_key, pair.public_key, "derived public key should match");
    }

    #[test]
    fn get_public_key_from_encrypted() {
        let pair = generate_keypair(Some("passphrase")).expect("generation should succeed");
        let pub_key =
            get_public_key(&pair.private_key).expect("public key extraction should succeed");
        assert_eq!(pub_key, pair.public_key, "derived public key should match");
    }

    #[test]
    fn generated_keys_are_unique() {
        let pair1 = generate_keypair(None).expect("generation should succeed");
        let pair2 = generate_keypair(None).expect("generation should succeed");
        assert_ne!(
            *pair1.private_key, *pair2.private_key,
            "two generated keys should differ"
        );
        assert_ne!(
            pair1.public_key, pair2.public_key,
            "two generated public keys should differ"
        );
    }

    #[test]
    fn export_private_key_validates() {
        let pair = generate_keypair(None).expect("generation should succeed");
        let exported = export_private_key(&pair.private_key).expect("export should succeed");
        assert_eq!(
            *exported, *pair.private_key,
            "exported key should match original"
        );
    }

    #[test]
    fn get_public_key_rejects_garbage() {
        let result = get_public_key("not a valid key");
        assert!(result.is_err(), "should reject invalid key");
    }

    #[test]
    fn export_private_key_rejects_garbage() {
        let result = export_private_key("garbage");
        assert!(result.is_err(), "should reject invalid key");
    }

    #[test]
    fn debug_redacts_private_key() {
        let pair = generate_keypair(None).expect("generation should succeed");
        let debug_output = format!("{pair:?}");
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug should redact the private key, got: {debug_output}"
        );
        assert!(
            !debug_output.contains("BEGIN OPENSSH PRIVATE KEY"),
            "Debug must not contain the PEM body, got: {debug_output}"
        );
        // The public key is safe to surface (it is public).
        assert!(
            debug_output.contains(&pair.public_key),
            "Debug should still show the public key, got: {debug_output}"
        );
    }

    // ── to_unencrypted_pem ──────────────────────────────────────────────

    /// Encrypt `plaintext` to an SSH recipient string, returning ciphertext.
    fn encrypt_to_ssh_recipient(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
        let recipient: ssh::Recipient = recipient_str.parse().unwrap();
        let recipients: Vec<Box<dyn age::Recipient>> = vec![Box::new(recipient)];
        let encryptor = Encryptor::with_recipients(recipients.iter().map(AsRef::as_ref)).unwrap();
        let mut encrypted = Vec::new();
        let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
        writer.write_all(plaintext).unwrap();
        writer.finish().unwrap();
        encrypted
    }

    /// Decrypt `ciphertext` with an UNENCRYPTED SSH PEM (the cached form),
    /// proving the serialized PEM lands on age's no-KDF Unencrypted path.
    fn decrypt_with_unencrypted_pem(ciphertext: &[u8], unencrypted_pem: &str) -> Vec<u8> {
        let identity = ssh::Identity::from_buffer(unencrypted_pem.as_bytes(), None).unwrap();
        let identities: Vec<Box<dyn age::Identity>> = vec![Box::new(identity)];
        let decryptor = Decryptor::new(ciphertext).unwrap();
        let mut reader = decryptor
            .decrypt(identities.iter().map(AsRef::as_ref))
            .unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        out
    }

    /// An encrypted ed25519 key decrypts to an unencrypted PEM that age parses
    /// onto the no-KDF Unencrypted path and can decrypt with.
    #[test]
    fn to_unencrypted_pem_round_trips_ed25519() {
        let pair = generate_keypair(Some("gate-passphrase")).unwrap();
        let unenc = to_unencrypted_pem(&pair.private_key, "gate-passphrase").unwrap();

        // Serialized as OpenSSH PEM.
        assert!(
            unenc.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"),
            "expected OpenSSH PEM, got: {}",
            unenc.as_str()
        );
        // age must classify the cached PEM as Unencrypted (no bcrypt KDF) — the
        // whole point of caching it. (The cipher is "none", but it lives inside
        // the base64 payload, so we assert the parsed variant, not a substring.)
        let parsed = ssh::Identity::from_buffer(unenc.as_bytes(), None).unwrap();
        assert!(
            matches!(parsed, ssh::Identity::Unencrypted(_)),
            "cached PEM must parse as Unencrypted"
        );

        // age re-parses onto the Unencrypted (no-KDF) path and decrypts a stanza.
        let plaintext = b"gate-secret";
        let ciphertext = encrypt_to_ssh_recipient(plaintext, &pair.public_key);
        let decrypted = decrypt_with_unencrypted_pem(&ciphertext, &unenc);
        assert_eq!(decrypted.as_slice(), plaintext);
    }

    /// A wrong passphrase for an encrypted key returns `WrongPassphrase`.
    #[test]
    fn to_unencrypted_pem_wrong_passphrase() {
        let pair = generate_keypair(Some("correct")).unwrap();
        let err = to_unencrypted_pem(&pair.private_key, "wrong").unwrap_err();
        assert_eq!(err.code, "WRONG_PASSPHRASE");
    }
}
