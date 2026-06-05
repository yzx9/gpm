// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::io::Write;
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};

/// Helper: create a test identity and recipient pair.
/// Returns (identity_string, recipient_string).
fn generate_test_keypair() -> (String, String) {
    let sk = Identity::generate();
    let pk = sk.to_public();

    let identity_str = sk.to_string().expose_secret().to_string();
    let recipient_str = pk.to_string();
    (identity_str, recipient_str)
}

/// Helper: encrypt plaintext to a recipient, return the ciphertext bytes.
fn encrypt_to_recipient(plaintext: &[u8], recipient_str: &str) -> Vec<u8> {
    let recipient = Recipient::from_str(recipient_str).unwrap();

    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .unwrap();
    let mut encrypted = Vec::new();
    let mut writer = encryptor.wrap_output(&mut encrypted).unwrap();
    writer.write_all(plaintext).unwrap();
    writer.finish().unwrap();
    encrypted
}

/// Helper: create a temporary directory that acts as a gopass store.
fn create_test_store(entries: Vec<(&str, &[u8])>, recipient_str: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in entries {
        let file_path = dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let encrypted = encrypt_to_recipient(content, recipient_str);
        std::fs::write(file_path, encrypted).unwrap();
    }
    dir
}

mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Store parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_entries_finds_age_files() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(
            vec![
                ("gmail/personal.age", b"s3cret\nuser: alice@gmail.com"),
                ("work/vpn.age", b"vpn-pass\nhost: vpn.example.com"),
                ("bank.age", b"bank-password"),
            ],
            &recipient,
        );

        let entries = gpm_lib::test_support::list_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.name == "gmail/personal"));
        assert!(entries.iter().any(|e| e.name == "work/vpn"));
        assert!(entries.iter().any(|e| e.name == "bank"));
    }

    #[test]
    fn test_list_entries_skips_gpg_files() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(vec![("valid.age", b"password")], &recipient);
        // Add a .gpg file
        std::fs::write(dir.path().join("legacy.gpg"), b"encrypted-gpg-data").unwrap();

        let entries = gpm_lib::test_support::list_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "valid");
    }

    #[test]
    fn test_list_entries_skips_git_dir() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(vec![("real.age", b"password")], &recipient);
        // Create a .git directory with a fake .age file inside
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config.age"), b"should-be-skipped").unwrap();

        let entries = gpm_lib::test_support::list_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "real");
    }

    #[test]
    fn test_list_entries_nested_paths() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(
            vec![
                ("cloud/aws/root.age", b"aws-secret"),
                ("cloud/gcp/admin.age", b"gcp-secret"),
            ],
            &recipient,
        );

        let entries = gpm_lib::test_support::list_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.path == "cloud/aws/root.age"));
        assert!(entries.iter().any(|e| e.path == "cloud/gcp/admin.age"));
    }

    #[test]
    fn test_list_entries_sorted_case_insensitive() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(
            vec![("Zebra.age", b"z"), ("alpha.age", b"a"), ("Beta.age", b"b")],
            &recipient,
        );

        let entries = gpm_lib::test_support::list_entries(dir.path()).unwrap();
        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[1].name, "Beta");
        assert_eq!(entries[2].name, "Zebra");
    }

    // -----------------------------------------------------------------------
    // Content parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_single_line() {
        let entry = gpm_lib::test_support::parse_decrypted_content(b"my-password").unwrap();
        assert_eq!(entry.password.as_str(), "my-password");
        assert_eq!(entry.notes.as_str(), "");
    }

    #[test]
    fn test_parse_multi_line() {
        let content = b"my-password\nusername: alice\nurl: https://example.com";
        let entry = gpm_lib::test_support::parse_decrypted_content(content).unwrap();
        assert_eq!(entry.password.as_str(), "my-password");
        assert!(entry.notes.as_str().contains("username: alice"));
        assert!(entry.notes.as_str().contains("url: https://example.com"));
    }

    #[test]
    fn test_parse_empty_content_errors() {
        let result = gpm_lib::test_support::parse_decrypted_content(b"");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_trailing_whitespace_stripped() {
        let entry = gpm_lib::test_support::parse_decrypted_content(b"pw\nnotes\n").unwrap();
        assert_eq!(entry.password.as_str(), "pw");
        assert_eq!(entry.notes.as_str(), "notes");
    }

    // -----------------------------------------------------------------------
    // Crypto tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decrypt_with_correct_identity() {
        let (identity, recipient) = generate_test_keypair();
        let plaintext = b"super-secret-password\nusername: alice";
        let encrypted = encrypt_to_recipient(plaintext, &recipient);

        let decrypted =
            gpm_lib::test_support::decrypt_bytes(&encrypted, identity.as_bytes()).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_with_wrong_identity_errors() {
        let (_identity, recipient) = generate_test_keypair();
        let (wrong_identity, _wrong_recipient) = generate_test_keypair();
        let encrypted = encrypt_to_recipient(b"secret", &recipient);

        let result = gpm_lib::test_support::decrypt_bytes(&encrypted, wrong_identity.as_bytes());
        assert!(result.is_err());
        // Verify error message contains no secret content
        let err_msg = format!("{}", result.unwrap_err());
        assert!(!err_msg.contains("secret"));
    }

    #[test]
    fn test_decrypt_invalid_identity_format() {
        let result = gpm_lib::test_support::decrypt_bytes(b"some data", b"not-a-valid-identity");
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_corrupted_data() {
        let (identity, _recipient) = generate_test_keypair();
        let result =
            gpm_lib::test_support::decrypt_bytes(b"not-valid-age-data", identity.as_bytes());
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Security tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_messages_contain_no_secrets() {
        let (identity, recipient) = generate_test_keypair();
        let (wrong_identity, _) = generate_test_keypair();
        let encrypted = encrypt_to_recipient(b"my-real-password", &recipient);

        let result = gpm_lib::test_support::decrypt_bytes(&encrypted, wrong_identity.as_bytes());
        let err = result.unwrap_err();
        let msg = format!("{}", err);

        assert!(!msg.contains("my-real-password"));
        assert!(!msg.contains(&identity));
    }
}
