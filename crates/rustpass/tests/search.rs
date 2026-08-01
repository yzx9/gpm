// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

mod tests {
    use rustpass::store;

    use super::common::*;

    #[test]
    fn search_entries_empty_query_returns_all_alpha_sorted() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(
            vec![
                ("cloud/aws/root.age", b"x"),
                ("email/personal.age", b"x"),
                ("bank.age", b"x"),
            ],
            &recipient,
        );

        // Empty query → every entry, alpha-sorted by name (mirrors list_entries).
        let entries =
            store::search_entries_in(dir.path(), rustpass::crypto::SecretExt::AGE, "").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["bank", "cloud/aws/root", "email/personal"]);
    }

    #[test]
    fn search_entries_subsequence_match_best_first() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(
            vec![
                ("cloud/aws/root.age", b"x"),
                ("email/personal.age", b"x"),
                ("bank.age", b"x"),
            ],
            &recipient,
        );

        // "awsroot" matches only cloud/aws/root, as a non-contiguous subsequence.
        let entries =
            store::search_entries_in(dir.path(), rustpass::crypto::SecretExt::AGE, "awsroot")
                .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.first().unwrap().path, "cloud/aws/root.age");
    }

    #[test]
    fn search_entries_case_insensitive() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(vec![("cloud/aws/root.age", b"x")], &recipient);

        // Uppercase query still matches (search is case-insensitive, not "smart").
        let entries =
            store::search_entries_in(dir.path(), rustpass::crypto::SecretExt::AGE, "AWS").unwrap();
        assert!(entries.iter().any(|e| e.path == "cloud/aws/root.age"));
    }

    #[test]
    fn search_entries_no_match_returns_empty() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(vec![("cloud/aws/root.age", b"x")], &recipient);

        assert!(
            store::search_entries_in(dir.path(), rustpass::crypto::SecretExt::AGE, "zzznomatch")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn search_entries_missing_repo_errors() {
        let missing = std::path::Path::new("/tmp/gpm_no_such_search_dir_12345");
        assert!(!missing.exists());
        // Propagates list_entries' NO_REPO (search_entries_in delegates to it).
        assert!(
            store::search_entries_in(missing, rustpass::crypto::SecretExt::AGE, "anything")
                .is_err()
        );
    }
}
