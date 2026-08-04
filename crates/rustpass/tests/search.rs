// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

// API-surface lints (missing_docs, pedantic, …) target library code; tests opt out.
#![allow(
    missing_docs,
    unused_qualifications,
    trivial_casts,
    trivial_numeric_casts,
    clippy::pedantic,
    clippy::indexing_slicing
)]

mod common;

mod tests {
    use std::path::Path;

    use rustpass::crypto::SecretExt;
    use rustpass::entry::Entry;
    use rustpass::error::Error;
    use rustpass::storage::git::list_entries;
    use rustpass::store::rank_entries;

    use super::common::*;

    /// Walk the store at `repo_path` and return its entries fuzzy-ranked by
    /// `query` (empty query → all entries, alpha-sorted). A thin wrapper over
    /// `list_entries` + `rank_entries` — the path-based search semantics these
    /// tests exercise. (Was a `rustpass::store` free fn; moved here once it had
    /// no non-test callers.)
    fn search_entries_in(
        repo_path: &Path,
        ext: SecretExt,
        query: &str,
    ) -> Result<Vec<Entry>, Error> {
        Ok(rank_entries(list_entries(repo_path, ext)?, query))
    }

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
        let entries = search_entries_in(dir.path(), SecretExt::AGE, "").unwrap();
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
        let entries = search_entries_in(dir.path(), SecretExt::AGE, "awsroot").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.first().unwrap().path, "cloud/aws/root.age");
    }

    #[test]
    fn search_entries_case_insensitive() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(vec![("cloud/aws/root.age", b"x")], &recipient);

        // Uppercase query still matches (search is case-insensitive, not "smart").
        let entries = search_entries_in(dir.path(), SecretExt::AGE, "AWS").unwrap();
        assert!(entries.iter().any(|e| e.path == "cloud/aws/root.age"));
    }

    #[test]
    fn search_entries_no_match_returns_empty() {
        let (_identity, recipient) = generate_test_keypair();
        let dir = create_test_store(vec![("cloud/aws/root.age", b"x")], &recipient);

        assert!(
            search_entries_in(dir.path(), SecretExt::AGE, "zzznomatch")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn search_entries_missing_repo_errors() {
        let missing = Path::new("/tmp/gpm_no_such_search_dir_12345");
        assert!(!missing.exists());
        // Propagates list_entries' NO_REPO (search_entries_in delegates to it).
        assert!(search_entries_in(missing, SecretExt::AGE, "anything").is_err());
    }
}
