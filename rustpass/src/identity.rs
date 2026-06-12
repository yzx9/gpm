// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Identity type classification for age identities.
//!
//! Provides a single [`classify_identity`] function that detects the type of
//! an age identity (x25519, SSH ed25519, SSH RSA, or age-encrypted) from its
//! byte content. This eliminates prefix-check duplication across call sites.

use serde::Serialize;

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
}
