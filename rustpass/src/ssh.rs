// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! SSH key generation, public key derivation, and private key export.
//!
//! Uses the `ssh-key` crate to generate ed25519 keypairs compatible with
//! standard OpenSSH tools and git2's SSH authentication.

use zeroize::Zeroizing;

use crate::error::{Error, ErrorCode};

/// A generated SSH keypair.
#[derive(Debug)]
pub struct SshKeyPair {
    /// OpenSSH PEM-encoded private key (wiped on drop).
    pub private_key: Zeroizing<String>,
    /// OpenSSH public key string (`ssh-ed25519 AAAA...`).
    pub public_key: String,
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
    use ssh_key::{Algorithm, LineEnding, PrivateKey, rand_core::OsRng};

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
    use ssh_key::PrivateKey;

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
    use ssh_key::{LineEnding, PrivateKey};

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

#[cfg(test)]
mod tests {
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
}
