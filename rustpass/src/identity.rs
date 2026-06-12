// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Identity type classification for age identities.
//!
//! Provides a single [`classify_identity`] function that detects the type of
//! an age identity (x25519, SSH ed25519, SSH RSA, or age-encrypted) from its
//! byte content. This eliminates prefix-check duplication across call sites.

use serde::Serialize;

use crate::error::{Error, ErrorCode};

/// The type of an age identity, detected from its byte content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityType {
    /// Native x25519 age key (`AGE-SECRET-KEY-...`).
    X25519,
    /// SSH ed25519 private key (OpenSSH format).
    SshEd25519,
    /// SSH RSA private key (PEM or OpenSSH format).
    SshRsa,
    /// Age passphrase-encrypted identity file (armored age encrypted blob).
    AgeEncrypted,
    /// Unknown / unrecognized identity format.
    Unknown,
}

/// Classify the type of an age identity from its byte content.
///
/// Detects the identity type by examining the leading bytes (prefix-based).
/// This is a cheap, non-validating check — it does not parse the full content.
#[must_use]
pub fn classify_identity(bytes: &[u8]) -> IdentityType {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return IdentityType::Unknown;
    };
    let trimmed = text.trim();

    if trimmed.starts_with("AGE-SECRET-KEY-") {
        IdentityType::X25519
    } else if trimmed.starts_with("-----BEGIN AGE ENCRYPTED FILE-----") {
        IdentityType::AgeEncrypted
    } else if trimmed.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----") {
        // Distinguish ed25519 from RSA within OpenSSH format
        if trimmed.contains("ssh-ed25519") || trimmed.contains("nistp256") {
            IdentityType::SshEd25519
        } else {
            // Default OpenSSH key to SshEd25519 (most common)
            // RSA keys in OpenSSH format are rare
            IdentityType::SshEd25519
        }
    } else if trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----") {
        IdentityType::SshRsa
    } else if trimmed.starts_with("-----BEGIN PRIVATE KEY-----") {
        // PKCS#8 format — could be any key type, default to SshEd25519
        IdentityType::SshEd25519
    } else {
        IdentityType::Unknown
    }
}

/// Validate that `identity_bytes` contains a recognized private key format.
///
/// Delegates to [`classify_identity`] for the actual prefix detection, keeping
/// it as the single source of truth. Accepts native x25519, OpenSSH, RSA, and
/// PKCS#8 private keys. Rejects age-encrypted blobs (not a private key) and
/// unrecognized formats.
///
/// # Errors
///
/// Returns `InvalidIdentity` if the format is not recognized.
pub fn validate_identity_format(identity_bytes: &[u8]) -> Result<(), Error> {
    match classify_identity(identity_bytes) {
        IdentityType::Unknown | IdentityType::AgeEncrypted => Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key",
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_x25519() {
        let identity = "AGE-SECRET-KEY-1TEST1234567890ABCDEF";
        assert_eq!(classify_identity(identity.as_bytes()), IdentityType::X25519);
    }

    #[test]
    fn classify_x25519_with_whitespace() {
        let identity = "  \n AGE-SECRET-KEY-1TEST1234567890ABCDEF \n ";
        assert_eq!(classify_identity(identity.as_bytes()), IdentityType::X25519);
    }

    #[test]
    fn classify_age_encrypted() {
        let encrypted = "-----BEGIN AGE ENCRYPTED FILE-----\nyWdlLWVuY3J5cHRpb24...\n-----END AGE ENCRYPTED FILE-----";
        assert_eq!(
            classify_identity(encrypted.as_bytes()),
            IdentityType::AgeEncrypted,
        );
    }

    #[test]
    fn classify_ssh_ed25519() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----";
        assert_eq!(classify_identity(key.as_bytes()), IdentityType::SshEd25519,);
    }

    #[test]
    fn classify_ssh_rsa() {
        let key =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEogIBAAKCAQEA\n-----END RSA PRIVATE KEY-----";
        assert_eq!(classify_identity(key.as_bytes()), IdentityType::SshRsa);
    }

    #[test]
    fn classify_unknown() {
        assert_eq!(classify_identity(b"not-a-valid-key"), IdentityType::Unknown);
    }

    #[test]
    fn classify_non_utf8() {
        assert_eq!(
            classify_identity(&[0xFF, 0xFE, 0x00]),
            IdentityType::Unknown,
        );
    }

    #[test]
    fn classify_empty() {
        assert_eq!(classify_identity(b""), IdentityType::Unknown);
    }

    // --- validate_identity_format tests ---

    #[test]
    fn validate_accepts_x25519() {
        assert!(validate_identity_format(b"AGE-SECRET-KEY-1TEST1234567890ABCDEF").is_ok());
    }

    #[test]
    fn validate_accepts_ssh_ed25519() {
        let key = b"-----BEGIN OPENSSH PRIVATE KEY-----\ndata\n-----END OPENSSH PRIVATE KEY-----";
        assert!(validate_identity_format(key).is_ok());
    }

    #[test]
    fn validate_accepts_ssh_rsa() {
        let key = b"-----BEGIN RSA PRIVATE KEY-----\ndata\n-----END RSA PRIVATE KEY-----";
        assert!(validate_identity_format(key).is_ok());
    }

    #[test]
    fn validate_rejects_unknown() {
        let result = validate_identity_format(b"not-a-key");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "INVALID_IDENTITY");
    }

    #[test]
    fn validate_rejects_age_encrypted() {
        let encrypted =
            b"-----BEGIN AGE ENCRYPTED FILE-----\ndata\n-----END AGE ENCRYPTED FILE-----";
        let result = validate_identity_format(encrypted);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "INVALID_IDENTITY");
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_identity_format(b"").is_err());
    }
}
