// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Setup glue: identity validation and the pending-identity state machine.
//!
//! App-free — [`setup::validate_identity`] is pure and [`setup::verify_picked`]
//! takes `&AppState`. The valuable invariant here is the file-picker flow: a
//! picked identity lives only in memory until verified, and any verify failure
//! (wrong passphrase, not-encrypted, …) **abandons** it.

use rustpass::{IdentityInfo, KeyType};
use zeroize::Zeroizing;

use crate::setup;
use crate::tests::{generate_test_keypair, make_unlocked_state};

/// A valid x25519 identity validates; garbage does not.
#[tokio::test]
async fn validate_identity_accepts_real_rejects_garbage() {
    let (identity, _recipient) = generate_test_keypair();

    setup::validate_identity(identity).expect("a fresh x25519 identity should validate");

    let err = setup::validate_identity("not-a-real-identity".to_string())
        .expect_err("garbage should not validate");
    assert_eq!(err.code, "INVALID_IDENTITY");
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
