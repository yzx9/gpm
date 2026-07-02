// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the create-store-from-scratch flow:
//! `Store::create_store` + the deferred first push. Covers the local-only happy
//! path, the bare-remote first push, the orphan-recipient atomicity guarantee,
//! the SSH recipient path, cleanup on a pre-init error, and a cross-binary
//! interop check against the bare `age` CLI.

mod common;

mod tests {
    use std::path::Path;

    use rustpass::recipient;
    use rustpass::ssh;
    use rustpass::store::Store;

    use super::common::*;

    /// Initialize an empty **bare** repository to act as a remote, returning its
    /// temp dir. The create flow pushes its first commit here.
    fn empty_bare_remote() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("failed to create bare dir");
        git2::Repository::init_bare(dir.path()).expect("failed to init bare repo");
        dir
    }

    /// `create_store` with no remote yields a fully local store: `is_repo_ready`
    /// is true, but `is_configured` stays false until the identity is saved
    /// (the caller runs `complete_setup` separately). After the identity lands,
    /// a `set`/`get` round-trip works — proving the push/pull no-ops hold
    /// end-to-end for a local-only store.
    #[tokio::test]
    async fn create_store_local_only_then_set_get_round_trips() {
        let (identity, recipient) = generate_test_keypair();
        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        store
            .create_store(None, None, None, None, &recipient)
            .await
            .expect("local-only create_store");

        // Repo is initialized (recipients file = init marker); identity not yet saved.
        assert!(store.is_repo_ready(), "repo should be ready after create");
        assert!(
            !store.is_configured(),
            "store is not fully configured until the identity is saved"
        );

        let recipients_path = config_dir.path().join("repo/.age-recipients");
        let recipients_content =
            std::fs::read_to_string(&recipients_path).expect("recipients file exists");
        assert!(
            recipients_content.trim() == recipient,
            ".age-recipients should hold exactly the seeded recipient"
        );

        // The initial commit exists with the gopass "Initialized Store" message.
        let repo = git2::Repository::open(config_dir.path().join("repo")).unwrap();
        let head = repo.head().unwrap().target().unwrap();
        let message = repo
            .find_commit(head)
            .unwrap()
            .message()
            .unwrap()
            .to_string();
        assert!(
            message.starts_with("Initialized Store for "),
            "initial commit message should match gopass, got: {message}"
        );

        // Saving the identity completes configuration; the recipient matches (it's ours).
        store
            .save_identity(&identity, None)
            .await
            .expect("save_identity matches the seeded recipient");
        assert!(store.is_configured());

        // Round-trip a secret. set() pre-syncs (no-op pull), writes, commits, and
        // pushes (no-op push) — all must succeed without an `origin`.
        let result = store
            .set("test/entry", b"super-secret\nuser: alice")
            .await
            .expect("set on a local-only store");
        assert!(
            !result.commit.is_empty(),
            "local-only write should succeed"
        );

        let secret = store.get("test/entry").await.expect("get");
        assert_eq!(secret.password(), "super-secret");
    }

    /// **Orphan-recipient atomicity (P1):** after `create_store` against a real
    /// remote — but *before* the deferred first push — the remote must still be
    /// empty. The store is only pushed once `Store::push` is called explicitly
    /// (after the identity is durable), so a failure between create and push can
    /// never leave an orphan store whose recipient's identity no longer exists.
    #[tokio::test]
    async fn create_store_defers_first_push_no_orphan_before_push() {
        let (identity, recipient) = generate_test_keypair();
        let bare_dir = empty_bare_remote();
        let remote_url = bare_dir.path().to_str().expect("valid utf-8").to_string();

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        store
            .create_store(Some(&remote_url), None, None, None, &recipient)
            .await
            .expect("create_store with a remote");

        // Local store is ready; the remote is configured locally (origin added)...
        assert!(store.is_repo_ready());
        let repo = git2::Repository::open(config_dir.path().join("repo")).unwrap();
        assert_eq!(
            repo.find_remote("origin")
                .expect("origin should be configured")
                .url(),
            Some(remote_url.as_str())
        );

        // ...but the remote has received NOTHING yet — no orphan recipient.
        let bare = git2::Repository::open(bare_dir.path()).unwrap();
        assert!(
            bare.head().is_err(),
            "remote must be empty after create_store (deferred push)"
        );

        // After the identity is durable, the explicit first push lands the store.
        store
            .save_identity(&identity, None)
            .await
            .expect("save_identity");
        store.push().await.expect("first push lands");

        let bare = git2::Repository::open(bare_dir.path()).unwrap();
        let head = bare
            .head()
            .expect("remote HEAD exists after push")
            .target()
            .unwrap();
        let tree = bare.find_commit(head).unwrap().tree().unwrap();
        assert!(
            tree.get_path(Path::new(".age-recipients")).is_ok(),
            "remote tree must contain .age-recipients after push"
        );
    }

    /// The SSH path: an SSH-ed25519 public key can be the seeded recipient, and
    /// the resulting store round-trips a secret encrypted to that recipient.
    #[tokio::test]
    async fn create_store_with_ssh_recipient_round_trips() {
        let ssh_private = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQAAAJCfEwtqnxML\nagAAAAtzc2gtZWQyNTUxOQAAACB7Ci6nqZYaVvrjm8+XbzII89TsXzP111AflR7WeorBjQ\nAAAEADBJvjZT8X6JRJI8xVq/1aU8nMVgOtVnmdwqWwrSlXG3sKLqeplhpW+uObz5dvMgjz\n1OxfM/XXUB+VHtZ6isGNAAAADHN0cjRkQGNhcmJvbgE=\n-----END OPENSSH PRIVATE KEY-----";
        let recipient =
            rustpass::recipient::identity_to_recipient(ssh_private, None).expect("ssh recipient");

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        store
            .create_store(None, None, None, None, &recipient)
            .await
            .expect("create_store with SSH recipient");

        let recipients_content =
            std::fs::read_to_string(config_dir.path().join("repo/.age-recipients")).unwrap();
        assert!(
            recipients_content.starts_with("ssh-ed25519 "),
            "recipients file should hold the SSH recipient"
        );

        store
            .save_identity(ssh_private, None)
            .await
            .expect("save_identity for SSH key");

        let result = store
            .set("ssh/entry", b"ssh-secret")
            .await
            .expect("set with SSH identity");
        assert!(!result.commit.is_empty());
        let secret = store.get("ssh/entry").await.expect("get");
        assert_eq!(secret.password(), "ssh-secret");
    }

    /// A pre-init error (empty recipient) returns `Err` and leaves no repo
    /// directory and no configuration — the next attempt starts clean.
    #[tokio::test]
    async fn create_store_empty_recipient_errors_and_leaves_nothing() {
        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        let err = store
            .create_store(None, None, None, None, "  ")
            .await
            .expect_err("empty recipient must be rejected");
        assert_eq!(err.code, "INVALID_IDENTITY");

        assert!(
            !config_dir.path().join("repo").exists(),
            "no repo dir should be left behind"
        );
        assert!(!store.is_repo_ready(), "no repo config should be persisted");
    }

    /// **Cross-tool interop:** a secret gpm encrypts must decrypt with the bare
    /// `age` CLI (a separate binary, independent of `rustpass::crypto`). Skips
    /// gracefully when `age` isn't on PATH — CI (nix shell) provides it.
    #[tokio::test]
    async fn created_store_secret_decrypts_with_bare_age_cli() {
        if std::process::Command::new("age")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping interop test: `age` CLI not on PATH");
            return;
        }

        let (identity, recipient) = generate_test_keypair();
        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        store
            .create_store(None, None, None, None, &recipient)
            .await
            .unwrap();
        store.save_identity(&identity, None).await.unwrap();
        store
            .set("interop/entry", b"decrypted-by-age-cli\nuser: bob")
            .await
            .unwrap();

        // Write the private identity to a temp file `age -d -i` can consume.
        let id_file = config_dir.path().join("interop-identity");
        std::fs::write(&id_file, identity.as_bytes()).unwrap();
        let entry = config_dir.path().join("repo/interop/entry.age");

        let output = std::process::Command::new("age")
            .arg("-d")
            .arg("-i")
            .arg(&id_file)
            .arg(&entry)
            .output()
            .expect("spawn age");

        // Best-effort wipe of the on-disk plaintext identity.
        let _ = std::fs::remove_file(&id_file);

        assert!(
            output.status.success(),
            "age -d failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"decrypted-by-age-cli\nuser: bob");
    }

    /// **SSH-create recipient guard:** the public key `ssh::generate_keypair`
    /// emits and the recipient `recipient::identity_to_recipient` re-derives from
    /// the private key must be byte-identical. `create_store` seeds
    /// `.age-recipients` from the former and `save_identity` checks the latter —
    /// if these two serializations ever diverge, every SSH-create strands the
    /// store (seeded recipient ≠ re-derived recipient → "does not match" on
    /// save). The `create_store_with_ssh_recipient_round_trips` test sidesteps
    /// this by deriving the recipient itself; this one exercises the real
    /// generate-then-seed path.
    #[test]
    fn ssh_generated_public_key_matches_age_derived_recipient() {
        for passphrase in [None, Some("create-passphrase")] {
            let pair = ssh::generate_keypair(passphrase).expect("ssh keygen");
            let derived = recipient::identity_to_recipient(pair.private_key.as_str(), passphrase)
                .expect("derive recipient from generated key");
            assert_eq!(
                derived, pair.public_key,
                "generate_ssh_key().public_key must equal the age-derived recipient \
                 (passphrase = {passphrase:?}); otherwise SSH-create strands the store"
            );
        }
    }

    /// Cleanup-on-failure (local-side atomicity): if a bootstrap step fails
    /// AFTER `git init` — here `save_repo_config` can't write its atomic temp —
    /// the partial repo dir + config must be removed so the next attempt starts
    /// clean and the store never looks half-initialized.
    #[tokio::test]
    async fn create_store_cleans_up_partial_state_when_persist_fails() {
        let (_identity, recipient) = generate_test_keypair();
        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        // Sabotage the FINAL bootstrap step: `save_repo_config` writes its atomic
        // temp to `repo.tmp`, so a directory there makes the persist fail AFTER
        // git init + recipients write + the initial commit have already landed.
        std::fs::create_dir(config_dir.path().join("repo.tmp")).unwrap();

        let err = store
            .create_store(None, None, None, None, &recipient)
            .await
            .expect_err("create_store must fail when config persist fails");
        let _ = err;

        assert!(
            !config_dir.path().join("repo").exists(),
            "partial repo dir must be removed on failure"
        );
        assert!(
            !store.is_repo_ready(),
            "store must not be marked ready after a failed create"
        );
    }
}
