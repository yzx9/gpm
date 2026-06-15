// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod common;

mod tests {
    use super::common::*;
    use rustpass::crypto;
    use rustpass::store::Store;
    use std::path::Path;

    /// Read a file straight from a bare repo's HEAD tree (the pushed remote).
    fn read_from_bare(bare_path: &Path, rel_path: &str) -> Vec<u8> {
        let repo = git2::Repository::open(bare_path).expect("open bare repo");
        let head = repo.head().expect("get head");
        let commit = repo
            .find_commit(head.target().expect("head oid"))
            .expect("find commit");
        let tree = commit.tree().expect("head tree");
        let entry = tree
            .get_path(Path::new(rel_path))
            .unwrap_or_else(|_| panic!("file {rel_path} should be in the bare repo HEAD"));
        let blob = repo.find_blob(entry.id()).expect("find blob");
        blob.content().to_vec()
    }

    /// Count commits reachable from a repo's HEAD.
    fn head_commit_count(repo_path: &Path) -> usize {
        let repo = git2::Repository::open(repo_path).expect("open repo");
        let head = repo.head().expect("head").target().expect("oid");
        let mut revwalk = repo.revwalk().expect("revwalk");
        revwalk.push(head).expect("push head");
        revwalk.count()
    }

    /// Configure a store backed by a fresh bare repo that ships a recipients
    /// file, so `set` has recipients to encrypt to. Generates its own keypair.
    async fn writable_store() -> (tempfile::TempDir, tempfile::TempDir, Store, Vec<u8>) {
        let (identity, recipient) = generate_test_keypair();

        let (bare_dir, _clone_dir) = create_test_git_repo_with(
            vec![],
            vec![(".gopass-recipients", recipient.as_bytes())],
            &recipient,
        );

        let config_dir = tempfile::tempdir().expect("config dir");
        let store = Store::new(config_dir.path().to_path_buf());
        store
            .configure(
                bare_dir.path().to_str().expect("utf-8"),
                None,
                None,
                None,
                &identity,
                None,
            )
            .await
            .expect("configure should succeed");

        let identity_bytes = identity.into_bytes();
        (bare_dir, config_dir, store, identity_bytes)
    }

    /// Full write flow: set → remote received the encrypted file → get reads it back.
    #[tokio::test]
    async fn set_writes_encrypts_commits_and_pushes() {
        let (identity, recipient) = generate_test_keypair();
        let (bare_dir, _clone_dir) = create_test_git_repo_with(
            vec![],
            vec![(".gopass-recipients", recipient.as_bytes())],
            &recipient,
        );
        let commits_before = head_commit_count(bare_dir.path());

        let config_dir = tempfile::tempdir().expect("config dir");
        let store = Store::new(config_dir.path().to_path_buf());
        store
            .configure(
                bare_dir.path().to_str().expect("utf-8"),
                None,
                None,
                None,
                &identity,
                None,
            )
            .await
            .expect("configure should succeed");

        let result = store
            .set("cloud/aws/root", b"s3kr3t-password\nuser: admin")
            .await
            .expect("set should succeed");
        assert!(!result.commit.is_empty(), "set should return a commit hash");

        // 1. The remote (bare) advanced by exactly one commit.
        assert_eq!(
            head_commit_count(bare_dir.path()),
            commits_before + 1,
            "push should add exactly one commit to the remote"
        );

        // 2. The remote holds the encrypted file, and it decrypts to our plaintext.
        let pushed = read_from_bare(bare_dir.path(), "cloud/aws/root.age");
        let decrypted =
            crypto::decrypt_bytes(&pushed, identity.as_bytes(), None).expect("decrypt pushed file");
        assert_eq!(decrypted, b"s3kr3t-password\nuser: admin");

        // 3. The local store lists the new entry and reads it back.
        let entries = store.list().await.expect("list");
        assert!(entries.iter().any(|e| e.name == "cloud/aws/root"));
        let secret = store.get("cloud/aws/root").await.expect("get");
        assert_eq!(secret.password(), "s3kr3t-password");
        assert!(secret.body().contains("user: admin"));
    }

    /// Writing a nested entry creates intermediate directories.
    #[tokio::test]
    async fn set_creates_nested_directories() {
        let (identity, recipient) = generate_test_keypair();
        let (bare_dir, _clone_dir) = create_test_git_repo_with(
            vec![],
            vec![(".gopass-recipients", recipient.as_bytes())],
            &recipient,
        );

        let config_dir = tempfile::tempdir().expect("config dir");
        let store = Store::new(config_dir.path().to_path_buf());
        store
            .configure(
                bare_dir.path().to_str().expect("utf-8"),
                None,
                None,
                None,
                &identity,
                None,
            )
            .await
            .expect("configure");

        store
            .set("a/b/c/deep", b"deep-secret")
            .await
            .expect("set nested");

        let pushed = read_from_bare(bare_dir.path(), "a/b/c/deep.age");
        assert_eq!(
            crypto::decrypt_bytes(&pushed, identity.as_bytes(), None).unwrap(),
            b"deep-secret"
        );
    }

    /// Our own key is always able to read what we write, even when the
    /// recipients file lists additional recipients we don't hold (ensureOurKeyID).
    #[tokio::test]
    async fn set_encrypts_to_all_recipients_and_stays_readable_by_us() {
        let (identity, recipient) = generate_test_keypair();
        let (_other_identity, other_recipient) = generate_test_keypair();

        // recipients file lists our key AND a second recipient we don't own.
        let recipients = format!("{recipient}\n{other_recipient}\n");
        let (bare_dir, _clone_dir) = create_test_git_repo_with(
            vec![],
            vec![(".gopass-recipients", recipients.as_bytes())],
            &recipient,
        );

        let config_dir = tempfile::tempdir().expect("config dir");
        let store = Store::new(config_dir.path().to_path_buf());
        store
            .configure(
                bare_dir.path().to_str().expect("utf-8"),
                None,
                None,
                None,
                &identity,
                None,
            )
            .await
            .expect("configure");

        store
            .set("shared/entry", b"team-secret")
            .await
            .expect("set");

        // We (first identity) can still decrypt it — our key was an encryption target.
        let secret = store.get("shared/entry").await.expect("get");
        assert_eq!(secret.password(), "team-secret");
    }

    /// Invalid secret names are rejected before any git/crypto work.
    #[tokio::test]
    async fn set_rejects_invalid_names() {
        let (_bare_dir, _config_dir, store, _id) = writable_store().await;

        for bad in [
            "",
            "  ",
            "/leading",
            "trailing/",
            "a//b",
            "..",
            "foo/../bar",
            "a/..",
        ] {
            let err = store.set(bad, b"x").await.unwrap_err();
            assert_eq!(
                err.code, "INVALID_ENTRY_NAME",
                "name {bad:?} should be rejected as INVALID_ENTRY_NAME, got {err}"
            );
        }
    }

    /// Path-traversal names are rejected (defense for the write path).
    #[tokio::test]
    async fn set_rejects_path_traversal() {
        let (_bare_dir, _config_dir, store, _id) = writable_store().await;

        let err = store.set("../escape", b"x").await.unwrap_err();
        assert_eq!(err.code, "INVALID_ENTRY_NAME");
    }

    /// Overwriting an existing local entry re-encrypts and pushes (happy path,
    /// no remote divergence). This is gopass `set` overwriting in place.
    #[tokio::test]
    async fn set_overwrites_existing_entry() {
        let (identity, recipient) = generate_test_keypair();
        let (bare_dir, _clone_dir) = create_test_git_repo_with(
            vec![],
            vec![(".gopass-recipients", recipient.as_bytes())],
            &recipient,
        );

        let config_dir = tempfile::tempdir().expect("config dir");
        let store = Store::new(config_dir.path().to_path_buf());
        store
            .configure(
                bare_dir.path().to_str().expect("utf-8"),
                None,
                None,
                None,
                &identity,
                None,
            )
            .await
            .expect("configure");

        store
            .set("rotate/me", b"old-password")
            .await
            .expect("set 1");
        store
            .set("rotate/me", b"new-password")
            .await
            .expect("set 2");

        let secret = store.get("rotate/me").await.expect("get");
        assert_eq!(secret.password(), "new-password");

        // The remote reflects the latest value.
        let pushed = read_from_bare(bare_dir.path(), "rotate/me.age");
        assert_eq!(
            crypto::decrypt_bytes(&pushed, identity.as_bytes(), None).unwrap(),
            b"new-password"
        );
    }
}
