// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! End-to-end proof that a store configured for the GPG/OpenPGP crypto backend
//! resolves to `GpgBackend` and decrypts a real secret through the `Store`
//! facade. This exercises the typed `RepoConfig.crypto` selection — `None`/
//! `"age"` → `AgeBackend`, `"gpg"` → `GpgBackend` — routing a full
//! `Store::get` (load config → read `<name>.gpg` → unlock identity → decrypt)
//! through the GPG backend against the committed system-gpg fixtures. The
//! decrypt primitives themselves are covered in-module in `crypto::gpg`; this
//! test wires them through the public `Store` API.

mod common;

use rustpass::crypto::{CryptoBackend, GpgBackend};
use rustpass::{Config, RepoConfig, Store};

/// Committed system-gpg RSA-2048 fixture key (S2K-passphrase-protected secret).
const FIXTURE_SECRET: &[u8] = include_bytes!("fixtures/gpg/secret.asc");
/// A secret encrypted to the fixture key by desktop `gpg` (compress-algo=none).
const FIXTURE_GPG_ENCRYPTED: &[u8] = include_bytes!("fixtures/gpg/gpg-encrypted.gpg");
const FIXTURE_PASSPHRASE: &str = "test-passphrase-fixture-only";
const EXPECTED_PLAINTEXT: &[u8] = b"gpg-to-rpgp interop plaintext";

#[tokio::test]
async fn gpg_store_decrypts_through_store_facade() {
    // The fixture key's gopass recipient id (0x + last 16 hex of fingerprint),
    // derived the same way `Store::save_identity` will derive it.
    let recipient = GpgBackend
        .identity_recipient(std::str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .expect("derive fixture recipient id");

    // A working git repo carrying a GPG-encrypted secret plus a `.gpg-id`
    // listing our recipient (the gopass on-disk layout GpgBackend reads).
    let gpg_id = format!("{recipient}\n");
    let (_bare, repo) = common::create_test_git_repo_with(
        vec![],
        vec![
            ("test.gpg", FIXTURE_GPG_ENCRYPTED),
            (".gpg-id", gpg_id.as_bytes()),
        ],
        // recipient_str is unused — no age entries are committed.
        "age1qcpwGY9xztuw39d8pe8cx3uyhu2v8pz39f6tje0x06d8tnz5eyqqt8z6e2",
    );

    let config_dir = tempfile::tempdir().unwrap();
    let store = Store::new(config_dir.path().to_path_buf(), None);

    // Select the GPG backend + point the store at the repo (sealed repo.json on
    // Android; plaintext here with no master key).
    Config::new(config_dir.path().to_path_buf(), None)
        .save_repo_config_full(&RepoConfig {
            local_path: repo.path().to_string_lossy().to_string(),
            crypto: Some("gpg".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    // Resolve both backends (mirrors the post-unlock one-shot), then store +
    // unlock the GPG identity and decrypt through the facade.
    store.resolve_storage().await.unwrap();
    store.resolve_crypto().await.expect("crypto=gpg resolves");
    store
        .save_identity(std::str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .await
        .expect("save_identity accepts the PGP key matching .gpg-id");
    store
        .unlock(FIXTURE_PASSPHRASE)
        .await
        .expect("unlock strips the S2K layer");

    let secret = store.get("test").await.expect("decrypt through Store::get");
    assert_eq!(
        secret.password().as_bytes(),
        EXPECTED_PLAINTEXT,
        "GpgBackend decrypted the secret end-to-end via Store"
    );
}
