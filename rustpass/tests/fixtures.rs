// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

mod tests {
    use super::common::*;
    use rustpass::crypto;
    use rustpass::secret::Secret;
    use rustpass::store;

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

        let entries = store::list_entries(dir.path()).unwrap();
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

        let entries = store::list_entries(dir.path()).unwrap();
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

        let entries = store::list_entries(dir.path()).unwrap();
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

        let entries = store::list_entries(dir.path()).unwrap();
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

        let entries = store::list_entries(dir.path()).unwrap();
        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[1].name, "Beta");
        assert_eq!(entries[2].name, "Zebra");
    }

    // -----------------------------------------------------------------------
    // Content parsing tests (via Secret::parse)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_single_line() {
        let secret = Secret::parse(b"my-password").unwrap();
        assert_eq!(secret.password(), "my-password");
        assert_eq!(secret.body(), "");
    }

    #[test]
    fn test_parse_multi_line() {
        let content = b"my-password\nusername: alice\nurl: https://example.com";
        let secret = Secret::parse(content).unwrap();
        assert_eq!(secret.password(), "my-password");
        assert!(secret.body().contains("username: alice"));
        assert!(secret.body().contains("url: https://example.com"));
    }

    #[test]
    fn test_parse_empty_content_errors() {
        let result = Secret::parse(b"");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_trailing_whitespace_stripped() {
        let secret = Secret::parse(b"pw\nnotes\n").unwrap();
        assert_eq!(secret.password(), "pw");
        assert_eq!(secret.body(), "notes");
    }

    // -----------------------------------------------------------------------
    // Crypto tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decrypt_with_correct_identity() {
        let (identity, recipient) = generate_test_keypair();
        let plaintext = b"super-secret-password\nusername: alice";
        let encrypted = encrypt_to_recipient(plaintext, &recipient);

        let decrypted = crypto::decrypt_bytes(&encrypted, identity.as_bytes()).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_with_wrong_identity_errors() {
        let (_identity, recipient) = generate_test_keypair();
        let (wrong_identity, _wrong_recipient) = generate_test_keypair();
        let encrypted = encrypt_to_recipient(b"secret", &recipient);

        let result = crypto::decrypt_bytes(&encrypted, wrong_identity.as_bytes());
        assert!(result.is_err());
        // Verify error message contains no secret content
        let err_msg = format!("{}", result.unwrap_err());
        assert!(!err_msg.contains("secret"));
    }

    #[test]
    fn test_decrypt_invalid_identity_format() {
        let result = crypto::decrypt_bytes(b"some data", b"not-a-valid-identity");
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_corrupted_data() {
        let (identity, _recipient) = generate_test_keypair();
        let result = crypto::decrypt_bytes(b"not-valid-age-data", identity.as_bytes());
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

        let result = crypto::decrypt_bytes(&encrypted, wrong_identity.as_bytes());
        let err = result.unwrap_err();
        let msg = format!("{}", err);

        assert!(!msg.contains("my-real-password"));
        assert!(!msg.contains(&identity));
    }
}
