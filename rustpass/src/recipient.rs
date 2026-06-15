// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Recipient discovery and identity validation for age-encrypted stores.
//!
//! Reads `.gopass-recipients` or `.age-recipients` files from a cloned
//! repository and provides utilities to validate that an age identity
//! matches a known recipient.
//!
//! Supports both native x25519 age keys (`age1...` / `AGE-SECRET-KEY-...`)
//! and SSH keys (`ssh-ed25519` / `ssh-rsa` as recipients, OpenSSH private
//! keys as identities).

use std::io::BufReader;
use std::path::Path;
use std::str::FromStr;

use age::{ssh, x25519};
use serde::Serialize;
use tokio::fs;

use crate::error::{Error, ErrorCode};

/// The type of an age identity/recipient key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyType {
    /// Native x25519 age key (age1... / AGE-SECRET-KEY-...).
    X25519,
    /// SSH ed25519 key (ssh-ed25519 ...).
    SshEd25519,
    /// SSH RSA key (ssh-rsa ...).
    SshRsa,
    /// Post-quantum MLKEM768-X25519 key (age1pq1...), recognized but unsupported.
    PostQuantum,
}

impl KeyType {
    /// Returns the default key type (X25519) for serde default.
    #[allow(dead_code)]
    fn default_value() -> Self {
        Self::X25519
    }
}

/// A recipient (public key) discovered in the store.
#[derive(Debug, Clone, Serialize)]
pub struct Recipient {
    /// Public key string as it appears in the recipients file.
    pub public_key: String,
    /// Optional comment from the recipients file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub comment: Option<String>,
    /// The type of this recipient key.
    #[serde(default = "KeyType::default_value")]
    pub key_type: KeyType,
}

/// Read recipients from a cloned gopass repository.
///
/// Looks for `.gopass-recipients` first, then falls back to `.age-recipients`.
/// Returns an empty list if neither file exists (the user can still proceed).
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read.
pub async fn list_recipients(repo_path: &Path) -> Result<Vec<Recipient>, Error> {
    let gopass_path = repo_path.join(".gopass-recipients");
    let age_path = repo_path.join(".age-recipients");

    let file_path = if gopass_path.exists() {
        &gopass_path
    } else if age_path.exists() {
        &age_path
    } else {
        return Ok(Vec::new());
    };

    let content = fs::read_to_string(file_path).await?;
    Ok(parse_recipients(&content))
}

/// Parse recipients from file content.
///
/// Each line can be:
/// - An age public key (`age1...`), optionally followed by `# comment`
/// - An SSH public key (`ssh-ed25519 ...` or `ssh-rsa ...`), optionally
///   followed by `# comment` or with an inline comment after the key data
/// - A comment line starting with `#`
/// - Empty (skipped)
fn parse_recipients(content: &str) -> Vec<Recipient> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }

            // Split on first '#' to extract comment
            let (key_part, hash_comment) = if let Some(idx) = trimmed.find('#') {
                let (k, c) = trimmed.split_at(idx);
                (k.trim(), Some(c[1..].trim().to_string()))
            } else {
                (trimmed, None)
            };

            if key_part.starts_with("age1pq1") {
                Some(Recipient {
                    public_key: key_part.to_string(),
                    comment: hash_comment,
                    key_type: KeyType::PostQuantum,
                })
            } else if key_part.starts_with("age1") {
                Some(Recipient {
                    public_key: key_part.to_string(),
                    comment: hash_comment,
                    key_type: KeyType::X25519,
                })
            } else {
                parse_ssh_recipient_line(key_part, hash_comment)
            }
        })
        .collect()
}

/// Parse an SSH recipient line like `ssh-ed25519 AAAA... user@host`.
///
/// The inline comment (e.g. `user@host`) is extracted if no hash comment
/// was already found.
fn parse_ssh_recipient_line(key_part: &str, hash_comment: Option<String>) -> Option<Recipient> {
    let key_type = if key_part.starts_with("ssh-ed25519 ") {
        KeyType::SshEd25519
    } else if key_part.starts_with("ssh-rsa ") {
        KeyType::SshRsa
    } else {
        return None;
    };

    // SSH public key format: `key_type base64_data [inline_comment]`
    // The full key portion is `key_type base64_data`.
    // Validate by parsing with the age crate.
    let parts: Vec<&str> = key_part.splitn(3, ' ').collect();
    let key_type_str = parts.first()?;
    let base64_data = parts.get(1)?;

    // Full key without inline comment: "key_type base64_data"
    let full_key = format!("{key_type_str} {base64_data}");

    // Validate that the age crate can parse this recipient
    if ssh::Recipient::from_str(&full_key).is_err() {
        return None;
    }

    // Use inline comment if no hash comment
    let comment = hash_comment.or_else(|| parts.get(2).map(ToString::to_string));

    Some(Recipient {
        public_key: full_key,
        comment,
        key_type,
    })
}

/// Derive the recipient (public key) from an age identity (private key).
///
/// Supports both native x25519 identities (`AGE-SECRET-KEY-...`) and SSH
/// private keys (OpenSSH or PEM format). For encrypted SSH keys, provide
/// the passphrase via the `passphrase` parameter.
///
/// # Errors
///
/// Returns an error if the identity format is invalid, cannot be parsed,
/// uses an unsupported key type, or an encrypted SSH key is provided
/// without a passphrase.
pub fn identity_to_recipient(identity: &str, passphrase: Option<&str>) -> Result<String, Error> {
    // age-keygen files include # comment lines before the key; use the bare key.
    let trimmed = crate::identity::normalize_identity_text(identity);

    if trimmed.starts_with("AGE-SECRET-KEY-PQ-1") {
        Err(Error::new(
            ErrorCode::PostQuantumNotSupported,
            "Post-quantum (ML-KEM-768 / X-Wing) age keys aren't supported yet",
        ))
    } else if trimmed.starts_with("AGE-SECRET-KEY-") {
        // x25519 path
        let sk = x25519::Identity::from_str(trimmed)
            .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Cannot parse age identity key"))?;
        Ok(sk.to_public().to_string())
    } else if trimmed.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        || trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----")
    {
        // SSH path
        let buf = BufReader::new(trimmed.as_bytes());
        let ssh_identity =
            ssh::Identity::from_buffer(buf, passphrase.map(String::from)).map_err(|e| {
                Error::new(
                    ErrorCode::InvalidIdentity,
                    format!("Cannot parse SSH private key: {e}"),
                )
            })?;

        match &ssh_identity {
            ssh::Identity::Encrypted(_) if passphrase.is_none() => Err(Error::new(
                ErrorCode::IdentityEncrypted,
                "Encrypted SSH key requires a passphrase",
            )),
            ssh::Identity::Encrypted(_) | ssh::Identity::Unencrypted(_) => {
                let recipient = ssh::Recipient::try_from(ssh_identity).map_err(|e| {
                    Error::new(
                        ErrorCode::InvalidIdentity,
                        format!("Cannot derive recipient from SSH key: {e:?}"),
                    )
                })?;
                Ok(recipient.to_string())
            }
            ssh::Identity::Unsupported(u) => Err(Error::new(
                ErrorCode::InvalidIdentity,
                format!("Unsupported SSH key type: {u:?}"),
            )),
        }
    } else {
        Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key",
        ))
    }
}

/// Detect the key type of an identity string.
///
/// Returns `KeyType::X25519` for age native keys and the appropriate SSH
/// variant for SSH private keys.
#[must_use]
pub fn detect_identity_type(identity: &str) -> KeyType {
    let trimmed = identity.trim();
    if trimmed.starts_with("AGE-SECRET-KEY-PQ-1") {
        KeyType::PostQuantum
    } else if trimmed.starts_with("AGE-SECRET-KEY-") {
        KeyType::X25519
    } else if trimmed.contains("ssh-ed25519") {
        KeyType::SshEd25519
    } else {
        KeyType::SshRsa
    }
}

/// Information about an age identity, returned by [`validate_identity`].
#[derive(Debug, Clone, Copy, Serialize)]
pub struct IdentityInfo {
    /// The type of the identity key.
    pub key_type: KeyType,
    /// True if the identity requires a passphrase (encrypted SSH key).
    pub encrypted: bool,
}

/// Validate an age identity and return its type and encryption status.
///
/// Parses the identity to determine its key type and whether it requires
/// a passphrase. This is used during setup to detect encrypted SSH keys
/// and prompt the user for a passphrase.
///
/// # Errors
///
/// Returns an error if the identity format is invalid or cannot be parsed.
pub fn validate_identity(identity: &str) -> Result<IdentityInfo, Error> {
    // age-keygen files include # comment lines before the key; use the bare key.
    let trimmed = crate::identity::normalize_identity_text(identity);

    if trimmed.starts_with("AGE-SECRET-KEY-PQ-1") {
        return Err(Error::new(
            ErrorCode::PostQuantumNotSupported,
            "Post-quantum (ML-KEM-768 / X-Wing) age keys aren't supported yet",
        ));
    }

    if trimmed.starts_with("AGE-SECRET-KEY-") {
        // Validate x25519 key can be parsed
        x25519::Identity::from_str(trimmed)
            .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Cannot parse age identity key"))?;
        Ok(IdentityInfo {
            key_type: KeyType::X25519,
            encrypted: false,
        })
    } else if trimmed.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        || trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----")
    {
        // Parse SSH key without passphrase to detect encryption
        let buf = BufReader::new(trimmed.as_bytes());
        let ssh_identity = ssh::Identity::from_buffer(buf, None).map_err(|e| {
            Error::new(
                ErrorCode::InvalidIdentity,
                format!("Cannot parse SSH private key: {e}"),
            )
        })?;

        let encrypted = matches!(ssh_identity, ssh::Identity::Encrypted(_));

        // Use classify_identity for key type detection — detect_identity_type
        // checks for literal "ssh-ed25519" which is only present in SSH public
        // keys, not in private key payloads.
        let key_type = if trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----") {
            KeyType::SshRsa
        } else {
            KeyType::SshEd25519
        };

        Ok(IdentityInfo {
            key_type,
            encrypted,
        })
    } else {
        Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recipients_basic() {
        let content = "age1ycefkjae3lkfue8sd9afkje3lkjfs9akjehr98sdf\nage1abcdef1234567890abcdef1234567890abcdef\n";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 2);
        assert_eq!(
            recipients.first().unwrap().public_key,
            "age1ycefkjae3lkfue8sd9afkje3lkjfs9akjehr98sdf"
        );
        assert_eq!(recipients.first().unwrap().comment, None);
        assert_eq!(recipients.first().unwrap().key_type, KeyType::X25519);
    }

    #[test]
    fn parse_recipients_with_comments() {
        let content = "# Team keys\nage1abc123... # Alice\nage1def456... # Bob\n";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 2);
        assert_eq!(recipients.first().unwrap().public_key, "age1abc123...");
        assert_eq!(
            recipients.first().unwrap().comment,
            Some("Alice".to_string())
        );
        assert_eq!(recipients.get(1).unwrap().comment, Some("Bob".to_string()));
    }

    #[test]
    fn parse_recipients_skip_comments_and_empty() {
        let content = "# This is a comment\n\nage1key1\n# Another comment\nage1key2\n";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 2);
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn parse_recipients_ssh_ed25519() {
        let content = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHsKLqeplhpW+uObz5dvMgjz1OxfM/XXUB+VHtZ6isGN alice@rust\n";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 1);
        let r = &recipients[0];
        assert_eq!(r.key_type, KeyType::SshEd25519);
        assert!(r.public_key.starts_with("ssh-ed25519 "));
        assert_eq!(r.comment, Some("alice@rust".to_string()));
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn parse_recipients_ssh_rsa() {
        let content = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDE7nIXTGNuaRBN9toI/wNALuQec8mvlt0iJ7o3OaD2UvoKHJ7S8rmIn4FiQDUed/Vac3OhUibei1k+TBmm16u2Rj3klgWZOIDgi8d4vXKI5N3YBhxr3jsQ+kz1c+iZ4z/tTtz306+4K46XViVMWwyyg9j82Jn41mOAy9vdeDIfQ5fLeaGqn5KwlT61GNkZ+ozWK/ZNlQIlNCcoXxhJULIs9XrtczWyVBAea1nlDo0WHODePxoJjmsNHrpQXn5mf9O83xs10qfTUjnRUt48jRmedFy4tcra3QGmSTQ3KZne+wXXSb0cIpXLGvZjQSPHgG1hc4r3uBpiSzvesGLv79XL alice@rust\n";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 1);
        let r = &recipients[0];
        assert_eq!(r.key_type, KeyType::SshRsa);
        assert!(r.public_key.starts_with("ssh-rsa "));
        assert_eq!(r.comment, Some("alice@rust".to_string()));
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn parse_recipients_mixed_types() {
        let content = "\
# Mixed recipients
age1abc123...
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHsKLqeplhpW+uObz5dvMgjz1OxfM/XXUB+VHtZ6isGN alice@rust
some-random-text
ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDE7nIXTGNuaRBN9toI/wNALuQec8mvlt0iJ7o3OaD2UvoKHJ7S8rmIn4FiQDUed/Vac3OhUibei1k+TBmm16u2Rj3klgWZOIDgi8d4vXKI5N3YBhxr3jsQ+kz1c+iZ4z/tTtz306+4K46XViVMWwyyg9j82Jn41mOAy9vdeDIfQ5fLeaGqn5KwlT61GNkZ+ozWK/ZNlQIlNCcoXxhJULIs9XrtczWyVBAea1nlDo0WHODePxoJjmsNHrpQXn5mf9O83xs10qfTUjnRUt48jRmedFy4tcra3QGmSTQ3KZne+wXXSb0cIpXLGvZjQSPHgG1hc4r3uBpiSzvesGLv79XL bob
";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 3);
        assert_eq!(recipients[0].key_type, KeyType::X25519);
        assert_eq!(recipients[1].key_type, KeyType::SshEd25519);
        assert_eq!(recipients[2].key_type, KeyType::SshRsa);
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn parse_recipients_post_quantum() {
        let content = "age1pq1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq\n";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients[0].key_type, KeyType::PostQuantum);
        assert!(recipients[0].public_key.starts_with("age1pq1"));
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn parse_recipients_pq_not_swallowed_as_x25519() {
        // Regression: a PQ recipient must not be tagged X25519 despite the
        // shared age1 prefix.
        let content = "\
age1abc123...
age1pq1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq
";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 2);
        assert_eq!(recipients[0].key_type, KeyType::X25519);
        assert_eq!(recipients[1].key_type, KeyType::PostQuantum);
    }

    #[test]
    fn parse_recipients_skip_non_recipient_lines() {
        let content = "some-random-text\nnope\n";
        let recipients = parse_recipients(content);
        assert!(recipients.is_empty());
    }

    #[test]
    fn parse_recipients_empty_content() {
        let recipients = parse_recipients("");
        assert!(recipients.is_empty());
    }

    #[tokio::test]
    async fn list_recipients_from_gopass_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".gopass-recipients"),
            "age1key1 # Alice\nage1key2\n",
        )
        .unwrap();

        let recipients = list_recipients(dir.path()).await.unwrap();
        assert_eq!(recipients.len(), 2);
    }

    #[tokio::test]
    async fn list_recipients_fallback_to_age_file() {
        let dir = tempfile::tempdir().unwrap();
        // Only .age-recipients exists
        std::fs::write(dir.path().join(".age-recipients"), "age1key1\n").unwrap();

        let recipients = list_recipients(dir.path()).await.unwrap();
        assert_eq!(recipients.len(), 1);
    }

    #[tokio::test]
    async fn list_recipients_prefers_gopass_over_age() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gopass-recipients"), "age1gopass\n").unwrap();
        std::fs::write(dir.path().join(".age-recipients"), "age1age\n").unwrap();

        let recipients = list_recipients(dir.path()).await.unwrap();
        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients.first().unwrap().public_key, "age1gopass");
    }

    #[tokio::test]
    async fn list_recipients_no_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let recipients = list_recipients(dir.path()).await.unwrap();
        assert!(recipients.is_empty());
    }

    #[test]
    fn identity_to_recipient_derives_correct_x25519_key() {
        use age::secrecy::ExposeSecret;
        use age::x25519::Identity;
        let sk = Identity::generate();
        let pk = sk.to_public();
        let identity_str = sk.to_string().expose_secret().to_string();
        let expected_recipient = pk.to_string();

        let derived = identity_to_recipient(&identity_str, None).unwrap();
        assert_eq!(derived, expected_recipient);
    }

    #[test]
    fn identity_to_recipient_derives_ssh_ed25519_key() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML
agAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ
AAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz
1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=
-----END OPENSSH PRIVATE KEY-----";
        let derived = identity_to_recipient(sk, None).unwrap();
        assert!(derived.starts_with("ssh-ed25519 "));
    }

    #[test]
    fn identity_to_recipient_derives_ssh_rsa_key() {
        let sk = "-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAxO5yF0xjbmkQTfbaCP8DQC7kHnPJr5bdIie6Nzmg9lL6Chye
0vK5iJ+BYkA1Hnf1WnNzoVIm3otZPkwZptertkY95JYFmTiA4IvHeL1yiOTd2AYc
a947EPpM9XPomeM/7U7c99OvuCuOl1YlTFsMsoPY/NiZ+NZjgMvb3XgyH0OXy3mh
qp+SsJU+tRjZGfqM1iv2TZUCJTQnKF8YSVCyLPV67XM1slQQHmtZ5Q6NFhzg3j8a
CY5rDR66UF5+Zn/TvN8bNdKn01I50VLePI0ZnnRcuLXK2t0Bpkk0NymZ3vsF10m9
HCKVyxr2Y0Ejx4BtYXOK97gaYks73rBi7+/VywIDAQABAoIBADGsf8TWtOH9yGoS
ES9hu90ttsbjqAUNhdv+r18Mv0hC5+UzEPDe3uPScB1rWrrDwXS+WHVhtoI+HhWz
tmi6UArbLvOA0Aq1EPUS7Q7Mop5bNIYwDG09EiMXL+BeC1b91nsygFRW5iULf502
0pOvB8XjshEdRcFZuqGbSmtTzTjLLxYS/aboBtZLHrH4cRlFMpHWCSuJng8Psahp
SnJbkjL7fHG81dlH+M3qm5EwdDJ1UmNkBfoSfGRs2pupk2cSJaL+SPkvNX+6Xyoy
yvfnbJzKUTcV6rf+0S0P0yrWK3zRK9maPJ1N60lFui9LvFsunCLkSAluGKiMwEjb
fm40F4kCgYEA+QzIeIGMwnaOQdAW4oc7hX5MgRPXJ836iALy56BCkZpZMjZ+VKpk
8P4E1HrEywpgqHMox08hfCTGX3Ph6fFIlS1/mkLojcgkrqmg1IrRvh8vvaZqzaAf
GKEhxxRta9Pvm44E2nUY97iCKzE3Vfh+FIyQLRuc+0COu49Me4HPtBUCgYEAym1T
vNZKPfC/eTMh+MbWMsQArOePdoHQyRC38zeWrLaDFOUVzwzEvCQ0IzSs0PnLWkZ4
xx60wBg5ZdU4iH4cnOYgjavQrbRFrCmZ1KDUm2+NAMw3avcLQqu41jqzyAlkktUL
fZzyqHIBmKYLqut5GslkGnQVg6hB4psutHhiel8CgYA3yy9WH9/C6QBxqgaWdSlW
fLby69j1p+WKdu6oCXUgXW3CHActPIckniPC3kYcHpUM58+o5wdfYnW2iKWB3XYf
RXQiwP6MVNwy7PmE5Byc9Sui1xdyPX75648/pEnnMDGrraNUtYsEZCd1Oa9l6SeF
vv/Fuzvt5caUKkQ+HxTDCQKBgFhqUiXr7zeIvQkiFVeE+a/ovmbHKXlYkCoSPFZm
VFCR00VAHjt2V0PaCE/MRSNtx61hlIVcWxSAQCnDbNLpSnQZa+SVRCtqzve4n/Eo
YlSV75+GkzoMN4XiXXRs5XOc7qnXlhJCiBac3Segdv4rpZTWm/uV8oOz7TseDtNS
tai/AoGAC0CiIJAzmmXscXNS/stLrL9bb3Yb+VZi9zN7Cb/w7B0IJ35N5UOFmKWA
QIGpMU4gh6p52S1eLttpIf2+39rEDzo8pY6BVmEp3fKN3jWmGS4mJQ31tWefupC+
fGNu+wyKxPnSU3svsuvrOdwwDKvfqCNyYK878qKAAaBqbGT1NJ8=
-----END RSA PRIVATE KEY-----";
        let derived = identity_to_recipient(sk, None).unwrap();
        assert!(derived.starts_with("ssh-rsa "));
    }

    #[test]
    fn identity_to_recipient_rejects_encrypted_ssh_key() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABC0OgNmiw
QW/kJ8kCmmTA2TAAAAEAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHsKLqeplhpW+uOb
z5dvMgjz1OxfM/XXUB+VHtZ6isGNAAAAkPhBKsZoNmaeuWYJQxOl+ofEmue/sFJnW+4IOt
oTrS/orMBJ4b/phQcv/ejWYJ4RYYVhSLiI6hf0KwNGefxI90E8iG/yDOKcrxb34tqDEYrY
FARDaJVRd9QtWLEqoP7pgdBR2BTP7aK1y6Mx3eFDgiQI9f/0Sjxd8V0apOPXv4i4kuQ1Nt
LF7kNlDznn/nyZlg==
-----END OPENSSH PRIVATE KEY-----";
        let result = identity_to_recipient(sk, None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "IDENTITY_ENCRYPTED");
    }

    #[test]
    fn identity_to_recipient_invalid_format() {
        let result = identity_to_recipient("not-a-valid-key", None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "INVALID_IDENTITY");
    }

    #[test]
    fn identity_to_recipient_rejects_post_quantum() {
        let identity = "AGE-SECRET-KEY-PQ-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        let result = identity_to_recipient(identity, None);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code,
            "POST_QUANTUM_NOT_SUPPORTED",
            "PQ identity must be rejected as unsupported, not routed to the x25519 parser"
        );
    }

    #[test]
    fn detect_identity_type_post_quantum() {
        let identity = "AGE-SECRET-KEY-PQ-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        assert_eq!(detect_identity_type(identity), KeyType::PostQuantum);
    }

    #[test]
    fn detect_identity_type_x25519_not_swallowed_by_pq_prefix() {
        let identity = "AGE-SECRET-KEY-1TEST1234567890ABCDEF";
        assert_eq!(detect_identity_type(identity), KeyType::X25519);
    }

    #[test]
    fn recipient_struct_debug_format() {
        let r = Recipient {
            public_key: "age1test".to_string(),
            comment: Some("Alice".to_string()),
            key_type: KeyType::X25519,
        };
        let debug = format!("{r:?}");
        assert!(debug.contains("age1test"));
    }

    // ── Encrypted SSH key tests ──────────────────────────────────────────

    #[test]
    fn identity_to_recipient_encrypted_ssh_key_with_passphrase() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let derived = identity_to_recipient(sk, Some("test-passphrase")).unwrap();
        assert!(
            derived.starts_with("ssh-ed25519 "),
            "expected ssh-ed25519 recipient, got: {derived}"
        );
    }

    #[test]
    fn validate_identity_x25519_not_encrypted() {
        use age::secrecy::ExposeSecret;
        use age::x25519::Identity;
        let sk = Identity::generate();
        let identity_str = sk.to_string().expose_secret().to_string();

        let info = validate_identity(&identity_str).unwrap();
        assert_eq!(info.key_type, KeyType::X25519);
        assert!(!info.encrypted);
    }

    #[test]
    fn validate_identity_unencrypted_ssh_key() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let info = validate_identity(sk).unwrap();
        assert_eq!(info.key_type, KeyType::SshEd25519);
        assert!(!info.encrypted);
    }

    #[test]
    fn validate_identity_encrypted_ssh_key() {
        let sk = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jYmMAAAAGYmNyeXB0AAAAGAAAABAO4u+xEG\nc7/4ChBhyKfc5AAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHuEHuK5j/S6zW08\nlcpk06Ast8Z7z7CjjvwJHMnKMjH7AAAAkEGCPxwe5eiPxyho1gM64dg5Upve28LioOvMhW\n2YUSDTCswCAqw6RRLa9ZSJ7IsiqMYblwP1UEyz4vbLM0BqqgpXtlfdnSwiZU6hRr+OU3r1\nAAjj0UXSjYEAglHKALANMwgiHENIsmye/YOH2fCJ8DjB3bvfdUKqBND56NON/MRY+8vujj\nIJjptSbFpDh+zfEg==\n-----END OPENSSH PRIVATE KEY-----";
        let info = validate_identity(sk).unwrap();
        assert_eq!(info.key_type, KeyType::SshEd25519);
        assert!(info.encrypted);
    }

    #[test]
    fn validate_identity_rejects_post_quantum() {
        let identity = "AGE-SECRET-KEY-PQ-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        let result = validate_identity(identity);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code,
            "POST_QUANTUM_NOT_SUPPORTED",
            "PQ identity must be rejected as unsupported, not routed to the x25519 parser"
        );
    }
}
