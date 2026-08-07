// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Setup glue: identity validation and the pending-identity state machine.
//!
//! App-free — [`setup::validate_identity`] is pure and [`setup::verify_picked`]
//! takes `&AppState`. The valuable invariant here is the file-picker flow: a
//! picked identity lives only in memory until verified, and any verify failure
//! (wrong passphrase, not-encrypted, …) **abandons** it.

use rustpass::crypto::{CryptoBackend, GpgBackend};
use rustpass::{IdentityInfo, KeyType};
use zeroize::Zeroizing;

use crate::setup;
use crate::tests::{generate_test_keypair, make_unlocked_state};

/// Committed system-gpg fixture key for the GPG `verify_picked` tests. The
/// keygen primitive is `pub(crate)` in rustpass (unreachable from the app), so
/// the test reuses the integration fixture. S2K-passphrase-protected.
const GPG_FIXTURE_SECRET: &[u8] =
    include_bytes!("../../../../crates/rustpass/tests/fixtures/gpg/secret.asc");
const GPG_FIXTURE_PASSPHRASE: &str = "test-passphrase-fixture-only";

/// A valid x25519 identity validates; garbage does not.
#[tokio::test]
async fn validate_identity_accepts_real_rejects_garbage() {
    let (identity, _recipient) = generate_test_keypair();

    let info = setup::validate_identity(identity).expect("a fresh x25519 identity should validate");
    assert!(
        info.recipient.is_some(),
        "x25519 validate_identity must return a derived recipient"
    );

    let err = setup::validate_identity("not-a-real-identity".to_string())
        .expect_err("garbage should not validate");
    assert_eq!(err.code, "INVALID_IDENTITY");
}

/// Pasted encrypted SSH: correct passphrase derives the matching recipient.
#[tokio::test]
async fn verify_pasted_correct_passphrase_derives_recipient() {
    let pair = rustpass::ssh::generate_keypair(Some("test-passphrase"))
        .expect("generate encrypted SSH keypair");
    let res = setup::verify_pasted(pair.private_key.to_string(), "test-passphrase".to_string())
        .await
        .expect("correct passphrase derives recipient");
    assert_eq!(res.recipient, pair.public_key);
}

/// Pasted encrypted SSH: wrong passphrase surfaces as `WRONG_PASSPHRASE`.
#[tokio::test]
async fn verify_pasted_wrong_passphrase() {
    let pair = rustpass::ssh::generate_keypair(Some("test-passphrase"))
        .expect("generate encrypted SSH keypair");
    let err = setup::verify_pasted(
        pair.private_key.to_string(),
        "not-the-passphrase".to_string(),
    )
    .await
    .expect_err("wrong passphrase should error");
    assert_eq!(err.code, "WRONG_PASSPHRASE");
}

/// Pasted non-encrypted identity: nothing to verify.
#[tokio::test]
async fn verify_pasted_non_encrypted_identity_errors() {
    let (identity, _recipient) = generate_test_keypair();
    let err = setup::verify_pasted(identity, "any".to_string())
        .await
        .expect_err("non-encrypted identity has nothing to verify");
    assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
}

/// Verifying with no identity picked returns `NO_IDENTITY` (the router gate).
#[tokio::test]
async fn verify_with_no_pending_identity_errors() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    assert!(
        state.pending_identity.lock().unwrap().is_none(),
        "precondition: nothing picked"
    );

    let err = setup::verify_picked(&state, "any-passphrase".to_string())
        .await
        .expect_err("verifying with nothing picked should error");
    assert_eq!(err.code, "NO_IDENTITY");
}

/// A verify failure abandons the picked file — the pending identity is dropped,
/// not left behind for a later accidental save. Uses the not-encrypted branch,
/// which errors without touching crypto.
#[tokio::test]
async fn verify_failure_abandons_picked_identity() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let (identity, _recipient) = generate_test_keypair();

    // Pick an unencrypted identity (so verify hits the not-encrypted error path).
    *state.pending_identity.lock().unwrap() = Some(setup::PendingIdentity {
        identity: Zeroizing::new(identity),
        info: IdentityInfo {
            key_type: KeyType::X25519,
            encrypted: false,
            recipient: None,
        },
    });

    let err = setup::verify_picked(&state, "irrelevant".to_string())
        .await
        .expect_err("a not-encrypted identity has nothing to verify");
    assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
    assert!(
        state.pending_identity.lock().unwrap().is_none(),
        "a verify failure must abandon the picked file"
    );
}

/// `verify_picked` SSH branch: correct passphrase derives the matching recipient.
/// Guards the refactor that routed encrypted-SSH verify through
/// `derive_ssh_recipient` (combined validate + derive in one `spawn_blocking`).
#[tokio::test]
async fn verify_picked_correct_ssh_passphrase_derives_recipient() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let pair = rustpass::ssh::generate_keypair(Some("test-passphrase"))
        .expect("generate encrypted SSH keypair");
    *state.pending_identity.lock().unwrap() = Some(setup::PendingIdentity {
        identity: Zeroizing::new(pair.private_key.to_string()),
        info: IdentityInfo {
            key_type: KeyType::SshEd25519,
            encrypted: true,
            recipient: None,
        },
    });

    let res = setup::verify_picked(&state, "test-passphrase".to_string())
        .await
        .expect("correct passphrase derives recipient");
    assert_eq!(res.recipient, pair.public_key);
}

/// `verify_picked` SSH branch: a wrong passphrase abandons the picked file (the
/// central regression risk of the `derive_ssh_recipient` refactor).
#[tokio::test]
async fn verify_picked_wrong_ssh_passphrase_abandons() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let pair = rustpass::ssh::generate_keypair(Some("test-passphrase"))
        .expect("generate encrypted SSH keypair");
    *state.pending_identity.lock().unwrap() = Some(setup::PendingIdentity {
        identity: Zeroizing::new(pair.private_key.to_string()),
        info: IdentityInfo {
            key_type: KeyType::SshEd25519,
            encrypted: true,
            recipient: None,
        },
    });

    let err = setup::verify_picked(&state, "not-the-passphrase".to_string())
        .await
        .expect_err("wrong passphrase should error");
    assert_eq!(err.code, "WRONG_PASSPHRASE");
    assert!(
        state.pending_identity.lock().unwrap().is_none(),
        "an SSH verify failure must abandon the picked file"
    );
}

/// `verify_pasted` rejects an unencrypted SSH identity (the `_ =>` arm's whole
/// purpose). Mirrors the x25519 rejection test so the arm stays honest.
#[tokio::test]
async fn verify_pasted_unencrypted_ssh_errors() {
    let pair = rustpass::ssh::generate_keypair(None).expect("generate unencrypted SSH keypair");
    let err = setup::verify_pasted(pair.private_key.to_string(), "any".to_string())
        .await
        .expect_err("unencrypted SSH has nothing to verify");
    assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
}

/// `verify_picked` GPG branch: a correct S2K passphrase is validated (throwaway)
/// and the S2K armor is re-staged BYTE-UNCHANGED — GPG stores the S2K-locked form
/// at rest, unlike the age arm which decrypts to a bare key. A future refactor
/// that accidentally mirrors the age arm (transforming `pending.identity`) would
/// brick the identity at rest on the next save; the byte-equal assertion pins it.
#[tokio::test]
async fn verify_picked_correct_gpg_passphrase_re_stages_armor() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let armor = std::str::from_utf8(GPG_FIXTURE_SECRET).expect("fixture armor utf-8");
    *state.pending_identity.lock().unwrap() = Some(setup::PendingIdentity {
        identity: Zeroizing::new(armor.to_string()),
        info: IdentityInfo {
            key_type: KeyType::Gpg,
            encrypted: true,
            recipient: None,
        },
    });

    let res = setup::verify_picked(&state, GPG_FIXTURE_PASSPHRASE.to_string())
        .await
        .expect("correct passphrase verifies");
    // The recipient is public-packet data (gopass Key.ID.: 0x + last 16 hex).
    let expected = GpgBackend
        .identity_recipient(armor, None)
        .expect("derive fixture recipient");
    assert_eq!(res.recipient, expected);

    // Load-bearing: the re-staged identity is the SAME S2K armor, byte-for-byte
    // (not an unlocked/re-armored form, which the at-rest store can't use).
    let guard = state.pending_identity.lock().unwrap();
    let re_staged = guard
        .as_ref()
        .expect("a successful verify re-stores the identity")
        .identity
        .as_str();
    assert_eq!(
        re_staged, armor,
        "the GPG S2K armor must be re-staged byte-unchanged"
    );
}

/// `verify_picked` GPG branch: a wrong S2K passphrase surfaces
/// `WRONG_PASSPHRASE` AND abandons the picked file (mirrors the SSH arm's
/// take()/re-store contract — any error drops `pending_identity`).
#[tokio::test]
async fn verify_picked_wrong_gpg_passphrase_abandons() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    let armor = std::str::from_utf8(GPG_FIXTURE_SECRET).expect("fixture armor utf-8");
    *state.pending_identity.lock().unwrap() = Some(setup::PendingIdentity {
        identity: Zeroizing::new(armor.to_string()),
        info: IdentityInfo {
            key_type: KeyType::Gpg,
            encrypted: true,
            recipient: None,
        },
    });

    let err = setup::verify_picked(&state, "not-the-passphrase".to_string())
        .await
        .expect_err("wrong passphrase should error");
    assert_eq!(err.code, "WRONG_PASSPHRASE");
    assert!(
        state.pending_identity.lock().unwrap().is_none(),
        "a GPG verify failure must abandon the picked file"
    );
}
