// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Conflict-stash consume lifecycle — the in-memory `(name, plaintext)` a write
//! collision stashes so a re-resolve doesn't round-trip the secret across IPC
//! again. The security invariant: the stash is consumed on every resolve
//! (success *or* failure) and on lock, so a plaintext never lingers behind a
//! wiped identity cache.
//!
//! The autosync write path never produces a `Conflict`, so nothing populates the
//! stash in production anymore; these tests pin the stash *utility* lifecycle
//! (stash → clear → resolve-consumes) that the (frozen) `resolve_write_conflict`
//! command and the lock handler's defense-in-depth clear still rely on.
//!
//! App-free: the cores ([`write::resolve_pending`], [`write::stash_pending`],
//! [`write::clear_pending`]) take `&AppState` directly.

use rustpass::ConflictChoice;

use crate::tests::make_unlocked_state;
use crate::write;

/// Stashing fills the pending slot; clearing empties it.
#[tokio::test]
async fn stash_then_clear_round_trip() {
    let (state, _guard) = make_unlocked_state(&[]).await;

    write::stash_pending(&state.pending_write, "sites/foo", b"hunter2".to_vec());
    assert!(
        state.pending_write.lock().unwrap().is_some(),
        "stash should fill the pending slot"
    );

    write::clear_pending(&state.pending_write);
    assert!(
        state.pending_write.lock().unwrap().is_none(),
        "clear should empty the pending slot"
    );
}

/// Resolving with nothing stashed returns a store error and consumes nothing.
#[tokio::test]
async fn resolve_with_no_pending_errors() {
    let (state, _guard) = make_unlocked_state(&[]).await;

    let err = write::resolve_pending(&state, ConflictChoice::Cancel)
        .await
        .expect_err("resolving with no pending should error");
    assert_eq!(err.code, "STORE_ERROR");
    assert!(
        state.pending_write.lock().unwrap().is_none(),
        "nothing was stashed, so nothing to consume"
    );
}

/// The stash is consumed even when the underlying resolve errors — the
/// "never linger" invariant. (The store here isn't in a real conflict state, so
/// the resolve errors; what matters is the plaintext is gone either way.)
#[tokio::test]
async fn resolve_consumes_pending_even_on_error() {
    let (state, _guard) = make_unlocked_state(&[]).await;
    write::stash_pending(&state.pending_write, "sites/foo", b"hunter2".to_vec());

    let _ = write::resolve_pending(&state, ConflictChoice::Cancel).await;

    assert!(
        state.pending_write.lock().unwrap().is_none(),
        "the stash must be consumed even when resolve errors"
    );
}
