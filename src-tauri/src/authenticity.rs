// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Repository authenticity commands — commit-signature verification, trusted
//! signing keys, and the Off / Audit / Enforce verification modes.

use rustpass::{
    AuthenticityConfig, CommitSigInfo, CommitSigStatus, Error, ErrorCode, TrustedGpgKey, TrustedKey,
    VerifyMode,
};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_file_picker::FilePickerExt;

use crate::setup::map_file_picker_error;
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

/// The trusted entry created by `add_trusted_signing_key` — a tagged union so
/// the unified paste command can report which kind of key was parsed and
/// persisted (the UI refreshes the right list). Internally tagged
/// (`{kind: "ssh"|"gpg", key: …}`) so it crosses the Tauri IPC boundary as a
/// TypeScript discriminated union.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "key", rename_all = "snake_case")]
pub(crate) enum AddedTrustedKey {
    Ssh(TrustedKey),
    Gpg(TrustedGpgKey),
}

/// Armored GPG public keys are small; reject anything larger before UTF-8
/// decode so a mis-picked multi-MB file can't be ingested. Generous enough for
/// a key with many subkeys + signatures, tight enough to exclude a video.
/// The same bound is also enforced at `Store::add_trusted_gpg_key` (the paste
/// path's chokepoint) — defense in depth.
const MAX_GPG_KEY_FILE_BYTES: usize = rustpass::MAX_GPG_KEY_FILE_BYTES;

/// Pure staging of a picked file's bytes into an armored GPG key + label,
/// factored out of [`import_trusted_gpg_key_file`] so the size / UTF-8 /
/// blank-label branches are unit-testable without an `AppHandle` or `Store`.
///
/// # Errors
///
/// Returns [`ErrorCode::SshKeyInvalid`] if the file is too large or not valid
/// UTF-8 — the same "not a usable GPG key" bucket as a parse failure, so every
/// wrong-file outcome surfaces one consistent message.
fn stage_gpg_key_from_bytes(
    bytes: &[u8],
    filename: Option<&str>,
    label: &str,
) -> Result<(String, String), Error> {
    if bytes.len() > MAX_GPG_KEY_FILE_BYTES {
        return Err(Error::new(
            ErrorCode::SshKeyInvalid,
            format!(
                "GPG key file too large ({} bytes; limit {} bytes) — not an armored public key.",
                bytes.len(),
                MAX_GPG_KEY_FILE_BYTES
            ),
        ));
    }
    let armored = std::str::from_utf8(bytes).map_err(|_| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            "GPG key file is not valid UTF-8 text — not an armored public key.",
        )
    })?;
    let label = if label.trim().is_empty() {
        // Pre-fill from the filename stem ("alice.asc" → "alice"); fall back to
        // the raw filename, then a generic default.
        filename
            .and_then(|f| {
                std::path::Path::new(f)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "signer".to_string())
    } else {
        label.trim().to_string()
    };
    Ok((armored.to_string(), label))
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

/// Add a trusted signing key from an armored block of EITHER format — the
/// server detects GPG (`-----BEGIN PGP PUBLIC KEY BLOCK-----`) vs SSH
/// (`ssh-ed25519 …` / `ssh-rsa …` / `ecdsa-…`) and routes to the right trust
/// store. Returns the typed entry so the UI knows which list to refresh. The
/// paste form calls this single command — no client-side format branching.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn add_trusted_signing_key(
    state: State<'_, AppState>,
    armored: String,
    label: String,
) -> Result<AddedTrustedKey, Error> {
    if armored
        .trim()
        .starts_with("-----BEGIN PGP PUBLIC KEY BLOCK")
    {
        let key = state
            .store
            .add_trusted_gpg_key(&armored, &label)
            .await?;
        Ok(AddedTrustedKey::Gpg(key))
    } else {
        let key = state.store.add_trusted_key(&armored, &label).await?;
        Ok(AddedTrustedKey::Ssh(key))
    }
}

/// Import a trusted GPG public key from a native-picked file — the primary GPG
/// path on Android, where pasting a multi-line armored block is painful. File
/// bytes never reach the `WebView`. See [`stage_gpg_key_from_bytes`] for the
/// size / UTF-8 / label handling; this command owns only the pick + the Store
/// call.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn import_trusted_gpg_key_file(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
) -> Result<TrustedGpgKey, Error> {
    let picked = app
        .file_picker()
        .pick()
        .await
        .map_err(map_file_picker_error)?;
    let (armored, label) =
        stage_gpg_key_from_bytes(&picked.bytes, picked.filename.as_deref(), &label)?;
    state.store.add_trusted_gpg_key(&armored, &label).await
}

/// Remove a trusted GPG key by primary fingerprint (last-key removal in
/// Enforce auto-downgrades to Audit).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn remove_trusted_gpg_key(
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<(), Error> {
    state.store.remove_trusted_gpg_key(&fingerprint).await
}

/// Per-key parse warnings for the persisted trusted GPG keys (Settings-only;
/// NOT part of `get_authenticity_state`, which is the entry-list badge hot
/// path). A trusted key that later fails to re-parse surfaces here instead of
/// silently downgrading its commits to `UnverifiedSignature`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_gpg_key_parse_warnings(
    state: State<'_, AppState>,
) -> Result<Vec<String>, Error> {
    state.store.gpg_key_parse_warnings().await
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

#[cfg(test)]
mod stage_gpg_key_tests {
    //! Unit tests for the pure `stage_gpg_key_from_bytes` helper — the size /
    //! UTF-8 / blank-label branches that `import_trusted_gpg_key_file` depends
    //! on but can't itself be unit-tested for (it takes an `AppHandle`). D4.

    use super::*;

    #[test]
    fn rejects_file_over_the_size_guard() {
        let big = vec![b'A'; MAX_GPG_KEY_FILE_BYTES + 1];
        let err = stage_gpg_key_from_bytes(&big, Some("k.asc"), "lbl").unwrap_err();
        assert_eq!(err.code, "SSH_KEY_INVALID");
        assert!(
            err.message.contains("too large"),
            "got: {err}"
        );
    }

    #[test]
    fn accepts_file_at_the_size_guard_boundary() {
        // The guard is strict (`>`), so exactly MAX bytes pass; the content is
        // valid UTF-8 so decode succeeds. The helper doesn't validate it's a
        // real key — that's the Store's job.
        let at_limit = vec![b'A'; MAX_GPG_KEY_FILE_BYTES];
        let (armor, label) = stage_gpg_key_from_bytes(&at_limit, None, "lbl").expect("boundary ok");
        assert_eq!(armor.len(), MAX_GPG_KEY_FILE_BYTES);
        assert_eq!(label, "lbl");
    }

    #[test]
    fn rejects_non_utf8_file() {
        // 0xFF is invalid as a leading UTF-8 byte.
        let err = stage_gpg_key_from_bytes(&[0xFF, 0xFE, 0xFD], Some("k.asc"), "lbl").unwrap_err();
        assert_eq!(err.code, "SSH_KEY_INVALID");
        assert!(err.message.contains("UTF-8"), "got: {err}");
    }

    #[test]
    fn blank_label_is_prefilled_from_filename_stem() {
        let (_, label) =
            stage_gpg_key_from_bytes(b"armor", Some("alice.asc"), "   ").expect("ok");
        assert_eq!(label, "alice", "blank label should take the filename stem");
    }

    #[test]
    fn blank_label_with_no_filename_falls_back_to_default() {
        let (_, label) = stage_gpg_key_from_bytes(b"armor", None, "   ").expect("ok");
        assert_eq!(label, "signer", "blank label + no filename should default");
    }

    #[test]
    fn provided_label_is_trimmed_and_kept() {
        let (_, label) =
            stage_gpg_key_from_bytes(b"armor", Some("alice.asc"), "  Bob  ").expect("ok");
        assert_eq!(label, "Bob", "non-blank label should be trimmed and kept");
    }
}
