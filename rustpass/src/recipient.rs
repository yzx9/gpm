// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Recipient discovery and identity validation for age-encrypted stores.
//!
//! Reads `.gopass-recipients` or `.age-recipients` files from a cloned
//! repository and provides utilities to validate that an age identity
//! matches a known recipient.

use std::path::Path;
use std::str::FromStr;

use serde::Serialize;

use crate::error::{Error, ErrorCode};

/// A recipient (age public key) discovered in the store.
#[derive(Debug, Clone, Serialize)]
pub struct Recipient {
    /// Age public key string (starts with `age1...`).
    pub public_key: String,
    /// Optional comment from the recipients file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub comment: Option<String>,
}

/// Read recipients from a cloned gopass repository.
///
/// Looks for `.gopass-recipients` first, then falls back to `.age-recipients`.
/// Returns an empty list if neither file exists (the user can still proceed).
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read.
pub fn list_recipients(repo_path: &Path) -> Result<Vec<Recipient>, Error> {
    let gopass_path = repo_path.join(".gopass-recipients");
    let age_path = repo_path.join(".age-recipients");

    let file_path = if gopass_path.exists() {
        &gopass_path
    } else if age_path.exists() {
        &age_path
    } else {
        return Ok(Vec::new());
    };

    let content = std::fs::read_to_string(file_path)?;
    Ok(parse_recipients(&content))
}

/// Parse recipients from file content.
///
/// Each line can be:
/// - An age public key (`age1...`), optionally followed by `# comment`
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
            let (key_part, comment) = if let Some(idx) = trimmed.find('#') {
                let (k, c) = trimmed.split_at(idx);
                (k.trim(), Some(c[1..].trim().to_string()))
            } else {
                (trimmed, None)
            };

            // Only accept lines that look like age public keys
            if key_part.starts_with("age1") {
                Some(Recipient {
                    public_key: key_part.to_string(),
                    comment,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Derive the recipient (public key) from an age identity (private key).
///
/// Takes a string starting with `AGE-SECRET-KEY-...` and returns the
/// corresponding `age1...` public key.
///
/// # Errors
///
/// Returns an error if the identity format is invalid or cannot be parsed.
pub fn identity_to_recipient(identity: &str) -> Result<String, Error> {
    let trimmed = identity.trim();
    if !trimmed.starts_with("AGE-SECRET-KEY-") {
        return Err(Error::new(
            ErrorCode::InvalidIdentity,
            "Identity must start with AGE-SECRET-KEY-...",
        ));
    }

    // Parse the x25519 identity directly and derive the public key
    let sk = age::x25519::Identity::from_str(trimmed)
        .map_err(|_| Error::new(ErrorCode::InvalidIdentity, "Cannot parse age identity key"))?;

    Ok(sk.to_public().to_string())
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
    fn parse_recipients_skip_non_age_lines() {
        let content = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...\nage1validkey\nsome-random-text\n";
        let recipients = parse_recipients(content);
        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients.first().unwrap().public_key, "age1validkey");
    }

    #[test]
    fn parse_recipients_empty_content() {
        let recipients = parse_recipients("");
        assert!(recipients.is_empty());
    }

    #[test]
    fn list_recipients_from_gopass_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".gopass-recipients"),
            "age1key1 # Alice\nage1key2\n",
        )
        .unwrap();

        let recipients = list_recipients(dir.path()).unwrap();
        assert_eq!(recipients.len(), 2);
    }

    #[test]
    fn list_recipients_fallback_to_age_file() {
        let dir = tempfile::tempdir().unwrap();
        // Only .age-recipients exists
        std::fs::write(dir.path().join(".age-recipients"), "age1key1\n").unwrap();

        let recipients = list_recipients(dir.path()).unwrap();
        assert_eq!(recipients.len(), 1);
    }

    #[test]
    fn list_recipients_prefers_gopass_over_age() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gopass-recipients"), "age1gopass\n").unwrap();
        std::fs::write(dir.path().join(".age-recipients"), "age1age\n").unwrap();

        let recipients = list_recipients(dir.path()).unwrap();
        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients.first().unwrap().public_key, "age1gopass");
    }

    #[test]
    fn list_recipients_no_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let recipients = list_recipients(dir.path()).unwrap();
        assert!(recipients.is_empty());
    }

    #[test]
    fn identity_to_recipient_derives_correct_key() {
        use age::secrecy::ExposeSecret;
        use age::x25519::Identity;
        let sk = Identity::generate();
        let pk = sk.to_public();
        let identity_str = sk.to_string().expose_secret().to_string();
        let expected_recipient = pk.to_string();

        let derived = identity_to_recipient(&identity_str).unwrap();
        assert_eq!(derived, expected_recipient);
    }

    #[test]
    fn identity_to_recipient_invalid_format() {
        let result = identity_to_recipient("not-a-valid-key");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "INVALID_IDENTITY");
    }

    #[test]
    fn recipient_struct_debug_format() {
        let r = Recipient {
            public_key: "age1test".to_string(),
            comment: Some("Alice".to_string()),
        };
        let debug = format!("{r:?}");
        assert!(debug.contains("age1test"));
    }
}
