// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Repository authenticity commands — commit-signature verification, trusted
//! signing keys, and the Off / Audit / Enforce verification modes.

use rustpass::{
    AuthenticityConfig, CommitSigInfo, CommitSigStatus, Error, ErrorCode, TrustedKey, VerifyMode,
};
use serde::Serialize;
use tauri::State;

use crate::AppState;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `get_authenticity_state` — the cached snapshot for the
/// entry-list indicator badge (mode + current HEAD verification status).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AuthenticityState {
    mode: VerifyMode,
    head_status: CommitSigStatus,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Cached authenticity snapshot for the entry-list indicator badge.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_authenticity_state(
    state: State<'_, AppState>,
) -> Result<AuthenticityState, Error> {
    let mode = state
        .store
        .authenticity_config()
        .await
        .map_or(VerifyMode::Off, |c| c.mode);
    // If HEAD status can't be computed (e.g. repo mid-clone), surface Unknown.
    let head_status = state
        .store
        .head_signature_status()
        .await
        .unwrap_or(CommitSigStatus::Unknown);
    Ok(AuthenticityState { mode, head_status })
}

/// Set the verification mode (Off / Audit / Enforce). Enforce is refused
/// until at least one trusted signing key is recorded.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_verification_mode(
    state: State<'_, AppState>,
    mode: VerifyMode,
) -> Result<VerifyMode, Error> {
    state.store.set_verification_mode(mode).await
}

/// Read the persisted authenticity config (no secrets — public trust anchors).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_authenticity_config(
    state: State<'_, AppState>,
) -> Result<AuthenticityConfig, Error> {
    state.store.authenticity_config().await
}

/// Add a trusted signing public key (validated + deduped by fingerprint).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn add_trusted_key(
    state: State<'_, AppState>,
    public_key: String,
    label: String,
) -> Result<TrustedKey, Error> {
    state.store.add_trusted_key(&public_key, &label).await
}

/// Remove a trusted signing key by fingerprint (last-key removal in Enforce
/// auto-downgrades to Audit).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn remove_trusted_key(
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<(), Error> {
    state.store.remove_trusted_key(&fingerprint).await
}

/// Trust HEAD's SSH-signature signer ("trust this signer" TOFU). Errors if HEAD
/// is unsigned or not SSH-signed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn trust_head_signer(
    state: State<'_, AppState>,
    label: String,
) -> Result<TrustedKey, Error> {
    let public_key = state.store.head_signer_public_key().await?.ok_or_else(|| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            "HEAD is not signed by an SSH key — nothing to trust.",
        )
    })?;
    state.store.add_trusted_key(&public_key, &label).await
}

/// Trust the SSH-signature signer of a specific commit ("trust this signer"
/// TOFU from the history detail view).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn trust_commit_signer(
    state: State<'_, AppState>,
    commit: String,
    label: String,
) -> Result<TrustedKey, Error> {
    state.store.trust_commit_signer(&commit, &label).await
}

/// Dismiss a specific commit's issue (per-commit + per-status ignore).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn ignore_commit_issue(
    state: State<'_, AppState>,
    commit: String,
) -> Result<(), Error> {
    state.store.ignore_commit_issue(&commit).await
}

/// List recent commits with per-commit signature status (the `/history` screen).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn list_commit_signatures(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<CommitSigInfo>, Error> {
    state
        .store
        .list_commit_signatures(limit.unwrap_or(50))
        .await
}

/// A single commit's signature detail (the per-commit detail sheet).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_commit_signature(
    state: State<'_, AppState>,
    hash: String,
) -> Result<CommitSigInfo, Error> {
    state.store.commit_signature(&hash).await
}
