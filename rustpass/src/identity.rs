// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Identity type classification for age identities.
//!
//! Provides a single [`classify_identity`] function that detects the type of
//! an age identity (x25519, SSH ed25519, SSH RSA, or age-encrypted) from its
//! byte content. This eliminates prefix-check duplication across call sites.

use std::str;

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
    /// Post-quantum MLKEM768-X25519 key (`AGE-SECRET-KEY-PQ-1...`), recognized
    /// but unsupported.
    PostQuantum,
    /// Unknown / unrecognized identity format.
    Unknown,
}

/// Classify the type of an age identity from its byte content.
///
/// Detects the identity type by examining the leading bytes (prefix-based).
/// This is a cheap, non-validating check — it does not parse the full content.
#[must_use]
pub fn classify_identity(bytes: &[u8]) -> IdentityType {
    let Ok(text) = str::from_utf8(bytes) else {
        return IdentityType::Unknown;
    };
    let trimmed = text.trim();

    if trimmed.starts_with("AGE-SECRET-KEY-PQ-1") {
        IdentityType::PostQuantum
    } else if trimmed.starts_with("AGE-SECRET-KEY-") {
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

/// Strip `age-keygen`-style `#` comment lines and return the bare identity.
///
/// `age-keygen` writes an identity file with `# created:` / `# public key:`
/// comment lines before the `AGE-SECRET-KEY-1...` line. For a native age
/// identity this returns just that key line; for SSH private keys and
/// age-armored blobs (which never contain such a line) the trimmed input is
/// returned unchanged.
#[must_use]
pub fn normalize_identity_text(text: &str) -> &str {
    let trimmed = text.trim();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.starts_with("AGE-SECRET-KEY-") {
            return line;
        }
    }
    trimmed
}

/// Validate that `identity_bytes` contains a recognized private key format.
///
/// Delegates to [`classify_identity`] for the actual prefix detection, keeping
/// it as the single source of truth. Accepts native x25519, OpenSSH, RSA, and
/// PKCS#8 private keys. Rejects age-encrypted blobs (not a private key),
/// unrecognized formats, and post-quantum keys (recognized but unsupported).
///
/// # Errors
///
/// Returns `InvalidIdentity` if the format is not recognized.
/// Returns `PostQuantumNotSupported` for post-quantum (`AGE-SECRET-KEY-PQ-1`)
/// keys.
pub fn validate_identity_format(identity_bytes: &[u8]) -> Result<(), Error> {
    match classify_identity(identity_bytes) {
        IdentityType::Unknown | IdentityType::AgeEncrypted => Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key",
        )),
        IdentityType::PostQuantum => Err(Error::new(
            ErrorCode::PostQuantumNotSupported,
            "Post-quantum (ML-KEM-768 / X-Wing) age keys aren't supported yet",
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
    fn classify_post_quantum() {
        // PQ identity prefix must be matched before the generic AGE-SECRET-KEY-.
        let identity = "AGE-SECRET-KEY-PQ-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        assert_eq!(
            classify_identity(identity.as_bytes()),
            IdentityType::PostQuantum,
        );
    }

    #[test]
    fn classify_x25519_is_not_swallowed_by_pq_prefix() {
        // Regression: a plain x25519 key (no PQ- segment) must stay X25519.
        let identity = "AGE-SECRET-KEY-1TEST1234567890ABCDEF";
        assert_eq!(classify_identity(identity.as_bytes()), IdentityType::X25519);
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

    // --- normalize_identity_text tests ---

    #[test]
    fn normalize_strips_age_keygen_comments() {
        let file = "# created: 2026-06-15T21:00:00+08:00\n\
                    # public key: age1q9jzl3a...\n\
                    AGE-SECRET-KEY-1SHQZY5UXJD4SVFMG9VKKK5P27H2K4726362NDYGVHRVNN29V5T3SUTKE7L\n";
        assert_eq!(
            normalize_identity_text(file),
            "AGE-SECRET-KEY-1SHQZY5UXJD4SVFMG9VKKK5P27H2K4726362NDYGVHRVNN29V5T3SUTKE7L",
        );
    }

    #[test]
    fn normalize_passes_through_bare_key() {
        let key = "AGE-SECRET-KEY-1TEST1234567890ABCDEF";
        assert_eq!(normalize_identity_text(key), key);
    }

    #[test]
    fn normalize_passes_through_ssh_key() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\ndata\n-----END OPENSSH PRIVATE KEY-----";
        // SSH keys have no AGE-SECRET-KEY- line → returned trimmed.
        assert_eq!(normalize_identity_text(key), key.trim());
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
    fn validate_rejects_post_quantum() {
        let identity = b"AGE-SECRET-KEY-PQ-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        let result = validate_identity_format(identity);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code,
            "POST_QUANTUM_NOT_SUPPORTED",
            "PQ identity must be rejected as unsupported, not as invalid format"
        );
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_identity_format(b"").is_err());
    }
}
