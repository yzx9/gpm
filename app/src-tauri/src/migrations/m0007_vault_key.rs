// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Migration `0007_vault_key` (R064).
//!
//! Brings a pre-R064 **App-Lock-ON** upgrader onto the master/vault split. The
//! legacy biometric alias (`gpm_master_key_biometric`) held the master under App
//! Lock; post-R064 the master is permanent auth-free and a **distinct** vault
//! key seals the identity. Concretely this step:
//!
//! 1. Mints the distinct vault key and seals it into `gpm_vault_key` (one
//!    biometric ENCRYPT prompt — the only prompt this migration shows).
//! 2. Re-keys `identity` + `app_id_pass` master→vault (idempotent per file).
//! 3. Deletes the legacy biometric alias. (The master's relocation to the
//!    auth-free alias is done by `app_unlock`'s legacy branch, which has the
//!    master bytes in hand from its DECRYPT — this step has no master bytes.)
//!
//! **No-op cases** (just the schema bump):
//! - Desktop / tests (`app_handle` is `None` — no Keystore, no seals).
//! - App-Lock-OFF upgraders: master already auth-free, identity under master, no
//!   vault — matches the post-split-off shape.
//! - New installs: already at the target schema (the engine peeks `None` ⇒
//!   target ⇒ skips every migration).
//!
//! # App-lock resume + crash-safety
//!
//! The vault mint is gated on [`Store::is_identity_under_master`]: it mints
//! ONLY while the identity is still under the master (not yet re-keyed). A crash
//! after the re-key but before the schema bump leaves the identity under the
//! vault — on resume `is_identity_under_master()` is `false`, so the mint is
//! skipped (the existing vault key in `gpm_vault_key` is preserved, never
//! re-minted) and the idempotent re-key finishes any partial file. The identity
//! file is its own migration marker — no extra bookkeeping. A cancelled ENCRYPT
//! prompt defers (`Pending`): the master is already auth-free and the identity
//! is still under the master (readable via the D6 bridge), so the next
//! `app_unlock` retries cleanly.

use std::sync::atomic::Ordering;

use rustpass::Error;
use tauri_plugin_keystore::KeystoreExt;

use crate::AppState;
use crate::keystore::BiometricSlot;
use crate::migrations::MigrationOutcome;

/// Advance an App-Lock-ON upgrader to the master/vault split, bumping
/// `schema_version` to 7. See the module docs for the no-op cases and the
/// crash-safety resume semantics.
///
/// # Errors
///
/// `Pending` (not `Err`) when blocked: cold start under App Lock (`vault_seal`
/// not yet keyed), or a cancelled ENCRYPT prompt. A genuine failure (vault
/// generation, the re-key, or the schema write) propagates as `Err` so the
/// engine retries on the next run with the schema still below 7.
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // 1. Cold start under App Lock: vault_seal is not keyed yet (the D6 bridge
    //    / vault retrieve happens in app_unlock, AFTER the biometric prompt).
    //    Defer — the next app_unlock retries from the top of the chain.
    if state.app_lock_enabled.load(Ordering::SeqCst) && !state.store.has_vault_key() {
        return Ok(MigrationOutcome::Pending);
    }
    // 2. Desktop / tests: no Keystore handle, no seals, no App Lock. Nothing to
    //    relocate — just advance the schema so the engine converges.
    let Some(app) = state.app_handle.as_ref() else {
        bump_schema_to(state, version).await?;
        return Ok(MigrationOutcome::Done);
    };
    // 3. App-Lock-OFF upgrader: master already auth-free, identity under master,
    //    no vault — already the post-split-off shape. Just the schema bump.
    if !state.app_lock_enabled.load(Ordering::SeqCst) {
        bump_schema_to(state, version).await?;
        return Ok(MigrationOutcome::Done);
    }
    let ks = app.keystore();
    // 4. Mint the vault key ONLY while the identity is still under the master
    //    (not yet re-keyed). Crash-safety: if a prior run already re-keyed the
    //    identity to a vault key but crashed before the schema bump, the mint is
    //    skipped so the existing vault key is preserved — the identity file is
    //    its own marker, and a re-mint would strand it under the old vault key.
    if state.store.is_identity_under_master().await {
        let vault = rustpass::seal::generate_master_key()?;
        // One-time ENCRYPT prompt (prompt text `None` → Kotlin fallback strings;
        // this is a one-shot migration). A cancel/reject ⇒ Pending so the next
        // app_unlock retries; nothing is stranded (master auth-free, identity
        // still under the master and readable via the bridge).
        if let Err(e) = crate::keystore::store_slot(ks, &vault, BiometricSlot::Vault, None).await {
            log::warn!("0007_vault_key: vault ENCRYPT deferred: {e:?}");
            return Ok(MigrationOutcome::Pending);
        }
        state.store.set_vault_key(Some(vault));
    }
    // 5. Re-key identity + app_id_pass master→vault. Idempotent per file
    //    (rekey_seal_pair): a partial prior run self-finishes, a completed one
    //    is a no-op. vault_seal is keyed here (the mint above) or, on a resume,
    //    by app_unlock's retrieve of the existing vault key.
    state.store.rekey_identity_to_vault().await?;
    // 6. Drop the legacy biometric alias — the master now lives auth-free. Gate
    //    on identity existing: a vault key is only minted when identity exists
    //    (step 4), so deleting legacy is safe only then. If identity is absent
    //    (e.g. a pre-upgrade reset_config cleared the file but left the keystore
    //    alias), KEEP the legacy alias as app_unlock's biometric fallback —
    //    deleting it with no vault would lock the user out until a cold restart.
    //    Best-effort: a delete failure leaves a dead alias, not a brick.
    if state.store.has_identity()
        && let Err(e) = crate::keystore::delete_slot(ks, BiometricSlot::Legacy).await
    {
        log::warn!("0007_vault_key: legacy alias delete failed: {e:?}");
    }
    // 7. Advance the schema. Self-contained copy of the bump helper — migrations
    //    do not depend on each other.
    bump_schema_to(state, version).await?;
    Ok(MigrationOutcome::Done)
}

/// Read the pref cache, bump its `schema_version` to `version`, persist
/// atomically. Propagates a save failure so the engine retries — never marks
/// `Done` without persisting (the engine's `debug_assert_eq!` would fire).
///
/// Self-contained copy of `m0005`'s helper: each migration is an independent
/// file and must not reach into another migration's privates.
async fn bump_schema_to(state: &AppState, version: u32) -> Result<(), Error> {
    let mut pref = state.app_config.get_pref();
    pref.schema_version = version;
    state.app_config.save_pref(&pref).await
}
