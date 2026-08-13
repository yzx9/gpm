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

//! End-to-end proof that a store configured for the GPG/OpenPGP crypto backend
//! resolves to `GpgBackend` and decrypts a real secret through the `Store`
//! facade. This exercises the typed `RepoConfig.crypto` selection — `None`/
//! `"age"` → `AgeBackend`, `"gpg"` → `GpgBackend` — routing a full
//! `Store::get` (load config → read `<name>.gpg` → unlock identity → decrypt)
//! through the GPG backend against the committed system-gpg fixtures. The
//! decrypt primitives themselves are covered in-module in `crypto::gpg`; this
//! test wires them through the public `Store` API.

mod common;

use rustpass::crypto::{BackendKind, CryptoBackend, GpgBackend};
use rustpass::{Config, RepoConfig, Store};

/// Committed system-gpg RSA-2048 fixture key (S2K-passphrase-protected secret).
const FIXTURE_SECRET: &[u8] = include_bytes!("fixtures/gpg/secret.asc");
/// The fixture key's armored public half — gopass's `.public-keys/<token>` blob.
const FIXTURE_PUBLIC: &[u8] = include_bytes!("fixtures/gpg/public.asc");
/// A secret encrypted to the fixture key by desktop `gpg` (compress-algo=none).
const FIXTURE_GPG_ENCRYPTED: &[u8] = include_bytes!("fixtures/gpg/gpg-encrypted.gpg");
const FIXTURE_PASSPHRASE: &str = "test-passphrase-fixture-only";
const EXPECTED_PLAINTEXT: &[u8] = b"gpg-to-rpgp interop plaintext";

#[tokio::test]
async fn gpg_store_decrypts_through_store_facade() {
    // The fixture key's gopass recipient id (0x + last 16 hex of fingerprint),
    // derived the same way `Store::save_identity` will derive it.
    let recipient = GpgBackend
        .identity_recipient(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .expect("derive fixture recipient id");

    // A working git repo carrying a GPG-encrypted secret plus a `.gpg-id`
    // listing our recipient (the gopass on-disk layout GpgBackend reads).
    let gpg_id = format!("{recipient}\n");
    // The gopass on-disk layout: `.gpg-id` lists the recipient token, and
    // `.public-keys/<token>` carries its armored pubkey (required — save_identity
    // now resolves membership by fingerprint through the pubkey pool, not a
    // string compare against `.gpg-id`).
    let pubkey_path = format!(".public-keys/{recipient}");
    let (_bare, repo) = common::create_test_git_repo_with(
        vec![],
        vec![
            ("test.gpg", FIXTURE_GPG_ENCRYPTED),
            (".gpg-id", gpg_id.as_bytes()),
            (pubkey_path.as_str(), FIXTURE_PUBLIC),
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
            crypto: BackendKind::Gpg,
            ..Default::default()
        })
        .await
        .unwrap();

    // Resolve both backends (mirrors the post-unlock one-shot), then store +
    // unlock the GPG identity and decrypt through the facade.
    store.resolve_storage().await.unwrap();
    store.resolve_crypto().await.expect("crypto=gpg resolves");
    store
        .save_identity(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
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

/// `save_identity` is the crypto-persistence authority: starting from
/// `crypto: None` (the age built-in, as a fresh clone leaves it), saving a PGP
/// key flips `repo.json.crypto` to `Some("gpg")`. The decrypt test above
/// pre-sets `crypto: Some("gpg")`, so this is the one path that actually
/// exercises the persist + slot swap.
#[tokio::test]
async fn save_identity_persists_gpg_kind_from_none() {
    let recipient = GpgBackend
        .identity_recipient(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .expect("derive fixture recipient id");
    let gpg_id = format!("{recipient}\n");
    let pubkey_path = format!(".public-keys/{recipient}");
    let (_bare, repo) = common::create_test_git_repo_with(
        vec![],
        vec![
            ("test.gpg", FIXTURE_GPG_ENCRYPTED),
            (".gpg-id", gpg_id.as_bytes()),
            (pubkey_path.as_str(), FIXTURE_PUBLIC),
        ],
        "age1qcpwGY9xztuw39d8pe8cx3uyhu2v8pz39f6tje0x06d8tnz5eyqqt8z6e2",
    );

    let config_dir = tempfile::tempdir().unwrap();
    let store = Store::new(config_dir.path().to_path_buf(), None);
    // `crypto: None` — the state a fresh `clone_only_with` leaves behind.
    Config::new(config_dir.path().to_path_buf(), None)
        .save_repo_config_full(&RepoConfig {
            local_path: repo.path().to_string_lossy().to_string(),
            crypto: BackendKind::Age,
            ..Default::default()
        })
        .await
        .unwrap();
    store.resolve_storage().await.unwrap();

    store
        .save_identity(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .await
        .expect("save_identity accepts the PGP key matching .gpg-id");

    assert_eq!(
        store.config().await.unwrap().crypto,
        BackendKind::Gpg,
        "save_identity persists crypto=gpg (the authority) starting from the age default"
    );
}

/// Flip-guard: saving a GPG identity into a store that holds `.age` secrets
/// is refused — switching to the gpg backend would orphan them (they'd vanish
/// from `list`). This store has no `.gpg-id`, so without the guard the
/// membership gate would be skipped and the flip would silently brick the store.
#[tokio::test]
async fn save_identity_refuses_gpg_flip_into_age_store() {
    // An age store carrying a `.age` secret (placed verbatim — the guard counts
    // extensions, it does not need a valid age ciphertext).
    let (_bare, repo) = common::create_test_git_repo_with(
        vec![],
        vec![("legacy.age", b"an existing age secret")],
        "age1qcpwGY9xztuw39d8pe8cx3uyhu2v8pz39f6tje0x06d8tnz5eyqqt8z6e2",
    );

    let config_dir = tempfile::tempdir().unwrap();
    let store = Store::new(config_dir.path().to_path_buf(), None);
    Config::new(config_dir.path().to_path_buf(), None)
        .save_repo_config_full(&RepoConfig {
            local_path: repo.path().to_string_lossy().to_string(),
            crypto: BackendKind::Age,
            ..Default::default()
        })
        .await
        .unwrap();
    store.resolve_storage().await.unwrap();

    let err = store
        .save_identity(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .await
        .expect_err("a gpg identity must not flip a store with .age secrets");
    assert_eq!(
        err.code, "INVALID_IDENTITY",
        "flip-guard refuses the age→gpg backend switch"
    );
    assert!(
        store.config().await.unwrap().crypto == BackendKind::Age,
        "a refused save must not mutate the persisted crypto kind"
    );
}

/// Marker hole: a store carrying the OTHER backend's root recipients marker
/// but ZERO secrets of that extension must still be refused — otherwise the
/// marker becomes a relic and future secrets of the orphaned extension vanish
/// from `list`. Here an age store (`.age-recipients` present, no `.age` secrets)
/// must reject a GPG identity even though the orphaned secret count is 0. The
/// secret-count guard above (save_identity_refuses_gpg_flip_into_age_store)
/// covers the with-secrets case; this pins the marker-only case.
#[tokio::test]
async fn save_identity_refuses_gpg_into_marker_only_age_store() {
    let age_recipient = "age1qcpwGY9xztuw39d8pe8cx3uyhu2v8pz39f6tje0x06d8tnz5eyqqt8z6e2";
    // `.age-recipients` present, zero `.age` secrets — a freshly-initialized age
    // store. No `.gpg-id`, so without the marker check the membership gate would
    // be skipped (Ok(None)) and the flip would silently proceed.
    let (_bare, repo) = common::create_test_git_repo_with(
        vec![],
        vec![(".age-recipients", age_recipient.as_bytes())],
        age_recipient,
    );

    let config_dir = tempfile::tempdir().unwrap();
    let store = Store::new(config_dir.path().to_path_buf(), None);
    Config::new(config_dir.path().to_path_buf(), None)
        .save_repo_config_full(&RepoConfig {
            local_path: repo.path().to_string_lossy().to_string(),
            crypto: BackendKind::Age,
            ..Default::default()
        })
        .await
        .unwrap();
    store.resolve_storage().await.unwrap();

    let err = store
        .save_identity(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .await
        .expect_err("a gpg identity must not flip an age-marker-only store");
    assert_eq!(
        err.code, "INVALID_IDENTITY",
        "the marker-only flip must be refused"
    );
    assert!(
        store.config().await.unwrap().crypto == BackendKind::Age,
        "a refused save must not mutate the persisted crypto kind"
    );
}

/// Write-side round-trip: `Store::set` writes a `.gpg` secret through GpgBackend
/// (encrypting to the `.gpg-id` recipient via the `.public-keys/` pool) and
/// `Store::get` decrypts it back. Proves gpm can AUTHOR GPG secrets, not just
/// decrypt a fixture — the spec-004 "GPG store is a real store" gap.
#[tokio::test]
async fn gpg_store_round_trips_a_written_secret() {
    let recipient = GpgBackend
        .identity_recipient(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .expect("derive fixture recipient id");
    let gpg_id = format!("{recipient}\n");
    let pubkey_path = format!(".public-keys/{recipient}");
    let (_bare, repo) = common::create_test_git_repo_with(
        vec![],
        vec![
            ("test.gpg", FIXTURE_GPG_ENCRYPTED),
            (".gpg-id", gpg_id.as_bytes()),
            (pubkey_path.as_str(), FIXTURE_PUBLIC),
        ],
        "age1qcpwGY9xztuw39d8pe8cx3uyhu2v8pz39f6tje0x06d8tnz5eyqqt8z6e2",
    );

    let config_dir = tempfile::tempdir().unwrap();
    let store = Store::new(config_dir.path().to_path_buf(), None);
    Config::new(config_dir.path().to_path_buf(), None)
        .save_repo_config_full(&RepoConfig {
            local_path: repo.path().to_string_lossy().to_string(),
            crypto: BackendKind::Gpg,
            ..Default::default()
        })
        .await
        .unwrap();
    store.resolve_storage().await.unwrap();
    store.resolve_crypto().await.expect("crypto=gpg resolves");
    store
        .save_identity(str::from_utf8(FIXTURE_SECRET).unwrap(), None)
        .await
        .expect("save_identity accepts the fixture key");
    store
        .unlock(FIXTURE_PASSPHRASE)
        .await
        .expect("unlock strips the S2K layer");
    // No origin → local-only writes (avoid the push path).
    store.set_autosync(false);

    store
        .set("written", b"freshly-written-gpg-secret")
        .await
        .expect("Store::set encrypts through GpgBackend");

    let secret = store
        .get("written")
        .await
        .expect("decrypt the written secret");
    assert_eq!(
        secret.password().as_bytes(),
        b"freshly-written-gpg-secret",
        "a GPG secret written through Store round-trips"
    );
}
