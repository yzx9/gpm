// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

// API-surface lints (missing_docs, pedantic, …) target library code; tests opt out.
#![allow(
    missing_docs,
    unused_qualifications,
    trivial_casts,
    trivial_numeric_casts,
    clippy::pedantic,
    clippy::indexing_slicing
)]

//! Integration tests for the create-a-brand-new-GPG-store flow:
//! `Store::create_gpg_store` — the GPG counterpart to `Store::create_store`. It
//! imports one existing GPG secret key, seeds gopass's `.gpg-id` +
//! `.public-keys/<token>` markers, and mirrors `gopass init` (GPG/gitfs): a
//! `.gitattributes` commit then an "Add current content" commit. Covers the
//! local-only happy path (with a `set`/`get` round-trip through `GpgBackend`),
//! the deferred first push (orphan-recipient invariant), pre-init + persist
//! cleanup, and a cross-binary interop check against the system `gpg` CLI.

mod common;

mod tests {
    use std::path::Path;
    use std::process::Command;

    use rustpass::crypto::{CryptoBackend, GpgBackend};
    use rustpass::{GitAuth, Store};

    /// Committed system-gpg RSA-2048 fixture key (S2K-passphrase-protected secret).
    const FIXTURE_SECRET: &[u8] = include_bytes!("fixtures/gpg/secret.asc");
    /// The fixture key's armored public half — gopass's `.public-keys/<token>` blob.
    const FIXTURE_PUBLIC: &[u8] = include_bytes!("fixtures/gpg/public.asc");
    const FIXTURE_PASSPHRASE: &str = "test-passphrase-fixture-only";

    /// The fixture key as a `&str` (the identity `create_gpg_store` consumes).
    fn fixture_identity() -> &'static str {
        str::from_utf8(FIXTURE_SECRET).expect("fixture secret is UTF-8 armor")
    }

    /// The fixture key's gopass recipient id (`0x` + last 16 hex), derived the
    /// same way `create_gpg_store` derives it.
    fn fixture_recipient() -> String {
        GpgBackend
            .identity_recipient(fixture_identity(), None)
            .expect("derive fixture recipient id")
    }

    /// Initialize an empty **bare** repository to act as a remote, returning its
    /// temp dir. The create flow pushes its first commit here.
    fn empty_bare_remote() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("failed to create bare dir");
        git2::Repository::init_bare(dir.path()).expect("failed to init bare repo");
        dir
    }

    /// `create_gpg_store` with no remote yields a fully local store that mirrors
    /// `gopass init` (GPG/gitfs): `.gitattributes` + `diff.gpg` config, then
    /// `.gpg-id` + `.public-keys/<token>`, in two commits with gopass's exact
    /// messages. After the identity lands + unlocks, a `set`/`get` round-trip
    /// works through `GpgBackend`.
    #[tokio::test]
    async fn create_gpg_store_local_only_seeds_gopass_layout_and_round_trips() {
        let recipient = fixture_recipient();
        // #41: the token must be UPPERCASE — gpg/gopass write uppercase and match
        // fingerprints case-sensitively, so the `.gpg-id` value AND the
        // `.public-keys/<token>` filename must be uppercase to round-trip through
        // gopass. Pinned authoritatively against the fixture's `gpg --with-colons`
        // fpr (not self-referential against `identity_recipient`'s own output).
        assert_eq!(
            recipient, "0x8C78A415A6EDA09F",
            "recipient token must be uppercase"
        );

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        store
            .create_gpg_store(None, &GitAuth::None, fixture_identity())
            .await
            .expect("local-only create_gpg_store");

        // Repo is initialized; identity not yet saved.
        assert!(store.is_repo_ready(), "repo should be ready after create");
        assert!(
            !store.is_configured(),
            "store is not fully configured until the identity is saved"
        );

        // .gpg-id holds exactly the recipient token (one trimmed line + newline).
        let gpg_id = std::fs::read_to_string(config_dir.path().join("repo").join(".gpg-id"))
            .expect(".gpg-id exists");
        assert_eq!(gpg_id, format!("{recipient}\n"), ".gpg-id holds the token");

        // .public-keys/<token> holds the armored pubkey verbatim.
        let pubkey = std::fs::read(
            config_dir
                .path()
                .join("repo")
                .join(".public-keys")
                .join(&recipient),
        )
        .expect(".public-keys/<token> exists");
        assert_eq!(
            pubkey, FIXTURE_PUBLIC,
            ".public-keys/<token> holds the armored pubkey verbatim"
        );
        // The on-disk filename is the uppercase token, checked independently of
        // the `recipient` variable used to build the read path above (#41).
        let mut pubkeys: Vec<String> =
            std::fs::read_dir(config_dir.path().join("repo").join(".public-keys"))
                .expect(".public-keys dir exists")
                .map(|e| {
                    e.expect("dir entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
        pubkeys.sort();
        assert_eq!(
            pubkeys,
            vec!["0x8C78A415A6EDA09F".to_string()],
            ".public-keys filename is the uppercase recipient token",
        );

        // .gitattributes carries gopass's diff-driver mapping.
        let gitattributes =
            std::fs::read_to_string(config_dir.path().join("repo").join(".gitattributes"))
                .expect(".gitattributes exists");
        assert_eq!(
            gitattributes, "*.gpg diff=gpg\n",
            ".gitattributes matches gopass"
        );

        // Two commits with gopass's exact messages, in order (HEAD = content, parent = config).
        let repo = git2::Repository::open(config_dir.path().join("repo")).unwrap();
        let head = repo.head().unwrap().target().unwrap();
        let head_commit = repo.find_commit(head).unwrap();
        assert_eq!(
            head_commit.message().unwrap(),
            "Add current content of password store",
            "HEAD commit message matches gopass"
        );
        let parent = head_commit.parents().next().expect("has a parent commit");
        assert_eq!(
            parent.message().unwrap(),
            "Configure git repository for gpg file diff.",
            "parent commit message matches gopass"
        );
        // The content commit's tree carries all three files.
        let tree = head_commit.tree().unwrap();
        assert!(
            tree.get_path(Path::new(".gpg-id")).is_ok(),
            "tree has .gpg-id"
        );
        assert!(
            tree.get_path(Path::new(&format!(".public-keys/{recipient}")))
                .is_ok(),
            "tree has .public-keys/<token>"
        );
        assert!(
            tree.get_path(Path::new(".gitattributes")).is_ok(),
            "tree has .gitattributes"
        );

        // diff.gpg config recorded in .git/config (gopass's fixConfig). Read the
        // file directly — git2's `Config::get_str` rejects ondisk configs as
        // "live"; the plain `.git/config` text is the source of truth anyway.
        let git_config =
            std::fs::read_to_string(config_dir.path().join("repo/.git/config")).unwrap();
        assert!(
            git_config.contains("textconv = gpg --no-tty --decrypt"),
            "diff.gpg.textconv set, got:\n{git_config}"
        );
        assert!(
            git_config.contains("binary = true"),
            "diff.gpg.binary set, got:\n{git_config}"
        );

        // crypto kind persisted as Gpg (the load-bearing save_repo_config_with_crypto path).
        assert_eq!(
            store.config().await.unwrap().crypto,
            rustpass::BackendKind::Gpg,
            "repo.json crypto must be gpg"
        );

        // Saving the identity completes configuration; the key matches .gpg-id.
        store
            .save_identity(fixture_identity(), None)
            .await
            .expect("save_identity matches the seeded recipient");
        assert!(store.is_configured());
        store
            .unlock(FIXTURE_PASSPHRASE)
            .await
            .expect("unlock strips the S2K layer");

        // Round-trip a secret through GpgBackend on a local-only store.
        store.set_autosync(false);
        store
            .set("test/entry", b"gpg-create-secret")
            .await
            .expect("set through GpgBackend");
        let secret = store.get("test/entry").await.expect("get");
        assert_eq!(secret.password().as_bytes(), b"gpg-create-secret");
    }

    /// A pre-init error (empty identity) returns `Err` and leaves no repo
    /// directory and no configuration — the next attempt starts clean.
    #[tokio::test]
    async fn create_gpg_store_empty_identity_errors_and_leaves_nothing() {
        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        let err = store
            .create_gpg_store(None, &GitAuth::None, "  ")
            .await
            .expect_err("empty identity must be rejected");
        assert_eq!(err.code, "INVALID_IDENTITY");

        assert!(
            !config_dir.path().join("repo").exists(),
            "no repo dir should be left behind"
        );
        assert!(!store.is_repo_ready(), "no repo config should be persisted");
    }

    /// An unparseable identity (truncated armor) surfaces as `InvalidIdentity`,
    /// never a panic crossing `spawn_blocking` — the parse-isolation contract.
    #[tokio::test]
    async fn create_gpg_store_malformed_identity_is_invalid_identity() {
        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);
        let malformed = "-----BEGIN PGP PRIVATE KEY BLOCK-----\n\ntruncated garbage";
        let err = store
            .create_gpg_store(None, &GitAuth::None, malformed)
            .await
            .expect_err("malformed armor must error, not panic");
        assert_eq!(
            err.code, "INVALID_IDENTITY",
            "malformed armor surfaces as InvalidIdentity, not a JoinError/StoreError"
        );
        assert!(!config_dir.path().join("repo").exists());
    }

    /// Cleanup-on-failure: if the FINAL bootstrap step (`save_repo_config_locked`)
    /// fails — here the config write lock's lockfile path is blocked by a
    /// directory, so the locked persist cannot even open the lock — the
    /// partial repo (with `.gitattributes`, `.gpg-id`, `.public-keys/`, the
    /// two commits, and the `.git/config` writes) is removed so the next
    /// attempt starts clean. Proves the multi-file seed needs no special
    /// cleanup.
    #[tokio::test]
    async fn create_gpg_store_cleans_up_partial_state_when_persist_fails() {
        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        // The persist takes the ConfigLock first; a directory at the lockfile
        // path makes every acquire (hence the persist) fail AFTER init + seed
        // + both commits have already landed.
        std::fs::create_dir(config_dir.path().join("gpm_config.lock")).unwrap();

        let err = store
            .create_gpg_store(None, &GitAuth::None, fixture_identity())
            .await
            .expect_err("create_gpg_store must fail when config persist fails");
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

    /// **Orphan-recipient atomicity:** after `create_gpg_store` against a remote
    /// — but *before* the deferred first push — the remote must still be empty.
    /// The store is only pushed once `Store::push` is called explicitly (after
    /// the identity is durable), so a failure between create and push can never
    /// leave an orphan store whose recipient's identity no longer exists.
    #[tokio::test]
    async fn create_gpg_store_defers_first_push_no_orphan_before_push() {
        let recipient = fixture_recipient();
        let bare_dir = empty_bare_remote();
        let remote_url = bare_dir.path().to_str().expect("valid utf-8").to_string();

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);

        store
            .create_gpg_store(Some(&remote_url), &GitAuth::None, fixture_identity())
            .await
            .expect("create_gpg_store with a remote");

        // Local store is ready; the remote is configured locally (origin added)...
        assert!(store.is_repo_ready());
        let repo = git2::Repository::open(config_dir.path().join("repo")).unwrap();
        assert_eq!(
            repo.find_remote("origin")
                .expect("origin should be configured")
                .url()
                .unwrap(),
            remote_url.as_str()
        );

        // ...but the remote has received NOTHING yet — no orphan recipient/pubkey.
        let bare = git2::Repository::open(bare_dir.path()).unwrap();
        assert!(
            bare.head().is_err(),
            "remote must be empty after create_gpg_store (deferred push)"
        );

        // After the identity is durable, the explicit first push lands the store.
        store
            .save_identity(fixture_identity(), None)
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
            tree.get_path(Path::new(".gpg-id")).is_ok(),
            "remote tree must contain .gpg-id after push"
        );
        assert!(
            tree.get_path(Path::new(&format!(".public-keys/{recipient}")))
                .is_ok(),
            "remote tree must contain .public-keys/<token> after push"
        );
        assert!(
            tree.get_path(Path::new(".gitattributes")).is_ok(),
            "remote tree must contain .gitattributes after push"
        );
    }

    /// **Cross-tool interop:** a secret gpm/rpgp encrypts must decrypt with the
    /// system `gpg` CLI (a separate binary, independent of `rustpass::crypto`).
    /// This is the real gopass-compat proof — not rpgp self-consistency. Skips
    /// gracefully when `gpg` isn't on PATH — CI (nix shell) provides it.
    #[tokio::test]
    async fn created_gpg_store_secret_decrypts_with_system_gpg() {
        if Command::new("gpg").arg("--version").output().is_err() {
            eprintln!("skipping interop test: `gpg` CLI not on PATH");
            return;
        }

        let config_dir = tempfile::tempdir().expect("failed to create config dir");
        let store = Store::new(config_dir.path().to_path_buf(), None);
        store
            .create_gpg_store(None, &GitAuth::None, fixture_identity())
            .await
            .expect("create_gpg_store");
        store
            .save_identity(fixture_identity(), None)
            .await
            .expect("save_identity");
        store.unlock(FIXTURE_PASSPHRASE).await.expect("unlock");
        store.set_autosync(false);
        let plaintext = b"decrypted-by-system-gpg\nuser: bob";
        store.set("interop/entry", plaintext).await.expect("set");

        // Isolated GNUPGHOME so the imported fixture key never touches the user's
        // keyring. gpg requires 0o700 on its home.
        let gpg_home = tempfile::tempdir().expect("failed to create gpg home");
        let home = gpg_home.path();
        std::fs::set_permissions(home, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let secret_file = config_dir.path().join("import-secret.asc");
        std::fs::write(&secret_file, FIXTURE_SECRET).unwrap();

        // Import the fixture secret key (stays S2K-locked; decryption supplies the
        // passphrase via loopback pinentry).
        let import = Command::new("gpg")
            .arg("--homedir")
            .arg(home)
            .args(["--batch", "--yes", "--import"])
            .arg(&secret_file)
            .output()
            .expect("spawn gpg --import");
        assert!(
            import.status.success(),
            "gpg --import failed: {}",
            String::from_utf8_lossy(&import.stderr)
        );

        let entry = config_dir.path().join("repo/interop/entry.gpg");
        let output = Command::new("gpg")
            .arg("--homedir")
            .arg(home)
            .args([
                "--batch",
                "--yes",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                FIXTURE_PASSPHRASE,
                "--decrypt",
            ])
            .arg(&entry)
            .output()
            .expect("spawn gpg --decrypt");

        // Best-effort wipe of the on-disk plaintext key copy.
        let _ = std::fs::remove_file(&secret_file);

        assert!(
            output.status.success(),
            "gpg --decrypt failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, plaintext);
    }
}
