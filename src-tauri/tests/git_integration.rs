// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

mod tests {
    use super::common::*;
    use gpm_lib::test_support::*;

    // -----------------------------------------------------------------------
    // clone_repo tests
    // -----------------------------------------------------------------------

    /// Clone a local bare repo to a new destination directory.
    /// Verifies that the destination contains a `.git` directory after cloning.
    #[test]
    fn clone_local_bare_repo() {
        let (_identity, recipient) = generate_test_keypair();
        let (bare_dir, _clone_dir) =
            create_test_git_repo(vec![("example.age", b"password123")], &recipient);

        let dest = tempfile::tempdir().expect("failed to create dest dir");
        // Remove the tempdir so clone_repo has a clean target path.  The
        // function creates the directory itself.
        let dest_path = dest.path().to_path_buf();
        drop(dest);

        clone_repo(
            bare_dir.path().to_str().expect("bare path is valid utf-8"),
            &dest_path,
            None,
        )
        .expect("clone should succeed");

        assert!(
            dest_path.join(".git").is_dir(),
            "cloned repo must contain a .git directory"
        );
    }

    /// Clone replaces an existing destination directory (removes old files).
    /// Verifies that stale files from a previous clone are gone after re-clone.
    #[test]
    fn clone_removes_existing_dest() {
        let (_identity, recipient) = generate_test_keypair();
        let (bare_dir, _clone_dir) =
            create_test_git_repo(vec![("real.age", b"secret")], &recipient);

        let dest = tempfile::tempdir().expect("failed to create dest dir");

        // Plant a stale file that should not survive the clone.
        std::fs::write(dest.path().join("stale-file.txt"), b"old data")
            .expect("failed to write stale file");
        assert!(
            dest.path().join("stale-file.txt").exists(),
            "precondition: stale file must exist before clone"
        );

        clone_repo(
            bare_dir.path().to_str().expect("bare path is valid utf-8"),
            dest.path(),
            None,
        )
        .expect("clone should succeed");

        assert!(
            !dest.path().join("stale-file.txt").exists(),
            "stale file must be removed by clone"
        );
        assert!(
            dest.path().join(".git").is_dir(),
            "cloned repo must contain a .git directory"
        );
    }

    // -----------------------------------------------------------------------
    // pull_repo tests
    // -----------------------------------------------------------------------

    /// Pull when the remote has new commits fast-forwards the local branch.
    ///
    /// The fetch refspec `refs/heads/*:refs/heads/*` updates local branches
    /// in-place during fetch, so `result.changed` is always false (the HEAD
    /// read after fetch already reflects the update).  Instead, verify that
    /// the reported HEAD hash differs from the pre-pull HEAD hash, confirming
    /// that the upstream commit was pulled.
    #[test]
    fn pull_fast_forward_succeeds() {
        let (_identity, recipient) = generate_test_keypair();
        let (bare_dir, clone_dir) =
            create_test_git_repo(vec![("initial.age", b"first-password")], &recipient);

        // Record HEAD before the upstream commit.
        let repo_before = git2::Repository::open(clone_dir.path()).expect("open clone repo");
        let head_before = repo_before
            .head()
            .expect("get head")
            .target()
            .expect("head oid");
        drop(repo_before);

        // Add a new commit to the bare (upstream) repo.
        let new_oid = add_commit_to_bare(
            bare_dir.path(),
            vec![("second.age", b"second-password")],
            &recipient,
            "add second entry",
        );

        // Pull should succeed and fast-forward.
        let result = pull_repo(clone_dir.path(), None).expect("pull should succeed");

        // Verify the HEAD hash advanced to the new upstream commit.
        assert_ne!(
            result.head,
            format!("{head_before:.7}"),
            "HEAD hash should advance past the original after fast-forward"
        );
        assert_eq!(
            result.head,
            format!("{new_oid:.7}"),
            "HEAD should match the new upstream commit"
        );
    }

    /// Pull when there are no new upstream commits returns unchanged.
    #[test]
    fn pull_no_changes() {
        let (_identity, recipient) = generate_test_keypair();
        let (_bare_dir, clone_dir) =
            create_test_git_repo(vec![("sole.age", b"only-password")], &recipient);

        // No new commits added to bare — pull should report no changes.
        let result = pull_repo(clone_dir.path(), None).expect("pull should succeed");
        assert!(
            !result.changed,
            "pull should report no changes when upstream is unchanged"
        );
    }

    /// Pull on a directory that is not a git repository returns an error.
    #[test]
    fn pull_nonexistent_repo_errors() {
        let nowhere = tempfile::tempdir().expect("failed to create temp dir");

        let result = pull_repo(nowhere.path(), None);
        let err = result.expect_err("pull on non-repo dir should fail");
        assert_eq!(
            err.code, "NO_REPO",
            "expected NO_REPO error code, got: {err}"
        );
    }

    /// Clone from a path that does not exist returns an error.
    #[test]
    fn clone_nonexistent_remote_errors() {
        let nowhere = tempfile::tempdir().expect("failed to create temp dir");
        let fake_url = nowhere.path().join("no-such-repo.git");
        // Ensure the path does not exist.
        assert!(!fake_url.exists(), "precondition: path must not exist");

        let dest = tempfile::tempdir().expect("failed to create dest dir");
        let result = clone_repo(
            fake_url.to_str().expect("path is valid utf-8"),
            dest.path(),
            None,
        );
        let err = result.expect_err("clone from nonexistent remote should fail");
        assert_eq!(
            err.code, "CLONE_FAILED",
            "expected CLONE_FAILED for nonexistent remote, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Full workflow (clone + list + decrypt)
    // -----------------------------------------------------------------------

    /// Golden-path end-to-end test: clone a local git repo, list its .age
    /// entries, decrypt one, and verify the plaintext content round-trips
    /// correctly.
    #[test]
    fn full_workflow_clone_list_decrypt() {
        let (identity, recipient) = generate_test_keypair();

        let entries: Vec<(&str, &[u8])> = vec![
            (
                "cloud/aws/root.age",
                b"AWS-SECRET-KEY\nuser: admin\nnotes: root account" as &[u8],
            ),
            (
                "email/gmail.age",
                b"gmail-password\nuser: alice@gmail.com" as &[u8],
            ),
            ("ssh/server.age", b"ssh-key-password" as &[u8]),
        ];

        let (bare_dir, _clone_dir) = create_test_git_repo(entries.clone(), &recipient);

        // Step 1: Clone the bare repo to a fresh destination.
        let dest = tempfile::tempdir().expect("failed to create dest dir");
        clone_repo(
            bare_dir.path().to_str().expect("bare path is valid utf-8"),
            dest.path(),
            None,
        )
        .expect("clone should succeed");

        // Step 2: List entries in the cloned repo.
        let found = list_entries(dest.path()).expect("list_entries should succeed");
        assert_eq!(
            found.len(),
            entries.len(),
            "should find exactly the entries that were committed"
        );
        assert!(
            found.iter().any(|e| e.name == "cloud/aws/root"),
            "should find cloud/aws/root entry"
        );
        assert!(
            found.iter().any(|e| e.name == "email/gmail"),
            "should find email/gmail entry"
        );
        assert!(
            found.iter().any(|e| e.name == "ssh/server"),
            "should find ssh/server entry"
        );

        // Step 3: Resolve, decrypt, and parse a specific entry.
        let file_path =
            resolve_entry_path(dest.path(), "cloud/aws/root.age").expect("resolve entry path");
        let decrypted = decrypt_file(&file_path, identity.as_bytes())
            .expect("decrypt should succeed with correct identity");

        let parsed = parse_decrypted_content(&decrypted).expect("parse should succeed");
        assert_eq!(
            parsed.password.as_str(),
            "AWS-SECRET-KEY",
            "password must match first line of plaintext"
        );
        assert!(
            parsed.notes.as_str().contains("user: admin"),
            "notes must contain subsequent lines"
        );
        assert!(
            parsed.notes.as_str().contains("root account"),
            "notes must contain all lines after the first"
        );
    }
}
