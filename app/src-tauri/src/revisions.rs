// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Secret revision-history commands (R027) — list a single secret's past
//! commits and view/copy an old version. Listing is pure metadata (no decrypt);
//! viewing/copying reuses the same identity-unlock + auto-clear + Immediate-wipe
//! contract as [`crate::read`]. A past value the current identity can't decrypt,
//! and a revision that deleted the entry, surface as distinct non-error states
//! so ciphertext never crosses IPC.

use std::fmt;
use std::sync::Arc;

use rustpass::{AttachmentMeta, CommitSigInfo, Error, ErrorCode, RevisionContent};
use serde::Serialize;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_clipboard_notify::NotifyText;
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::{maybe_soft_wipe, reset_gate_idle_timer, reset_lock_timer};
use crate::page::clamp_limit;
use crate::read::{AttributeView, CopyResult, attr_view};
use crate::registry::RepoId;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// One page of revisions for a single secret — the commits (newest first) that
/// touched it, each with verification status, plus the `base_oid` that anchors
/// pagination across a possible background sync. Mirrors the
/// [`crate::authenticity`] page envelope.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RevisionPage {
    commits: Vec<CommitSigInfo>,
    has_more: bool,
    /// The HEAD oid this page walked from — the `WebView` passes it back on the
    /// next page so a background fast-forward can't drift the window.
    base_oid: String,
}

/// The outcome of viewing one revision. Ciphertext never crosses IPC: a past
/// value the current identity can't decrypt, and a revision that deleted the
/// entry, are reported by their `kind`, not surfaced.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RevisionView {
    /// Decrypted past value. Carries the same fields as
    /// [`crate::read::SensitiveContent`] so it feeds the same reveal path.
    Decrypted {
        password: Zeroizing<String>,
        notes: Zeroizing<String>,
        /// The parsed `Key: Value` attribute region (gopass AKV) — same shape and
        /// semantics as [`crate::read::SensitiveContent::attributes`] (empty for
        /// attachments) so this feeds the same reveal path.
        attributes: Vec<AttributeView>,
        has_totp: bool,
        /// `Some` when this revision is a binary attachment — the UI then shows
        /// the attachment notice instead of the (empty) reveal block, since a
        /// past attachment has no copyable password. `notes` is cleared in that
        /// case so the base64 body never reaches the `WebView`. Mirrors
        /// [`crate::read::SensitiveContent::attachment`].
        attachment: Option<AttachmentMeta>,
    },
    /// Encrypted to a recipient set the current identity isn't in.
    Undecryptable,
    /// The commit deleted the entry — no blob at this commit.
    Deleted,
}

/// Redacts the decrypted variant — mirrors [`crate::read::SensitiveContent`] so
/// `Debug` (logs) never leaks a past password. `attachment` is non-secret
/// (filename + size), so it is shown.
impl fmt::Debug for RevisionView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decrypted { attachment, .. } => f
                .debug_struct("RevisionView")
                .field("kind", &"decrypted")
                .field("password", &"[REDACTED]")
                .field("notes", &"[REDACTED]")
                .field("attributes", &"[REDACTED]")
                .field("attachment", attachment)
                .finish(),
            Self::Undecryptable => write!(f, "RevisionView::Undecryptable"),
            Self::Deleted => write!(f, "RevisionView::Deleted"),
        }
    }
}

/// Map a backend [`RevisionContent`] to its IPC view.
fn revision_view(content: RevisionContent) -> RevisionView {
    match content {
        RevisionContent::Decrypted(secret) => {
            let body = secret.body();
            let attachment = rustpass::metadata(&secret);
            RevisionView::Decrypted {
                password: Zeroizing::new(secret.password().to_string()),
                // For an attachment the body is a base64 wall; clear it so the
                // blob never reaches the WebView. `metadata` (not the bare
                // `has_attachment` check) because this path surfaces the
                // attachment metadata to the UI — matching `show_password_core`.
                notes: if attachment.is_some() {
                    Zeroizing::new(String::new())
                } else {
                    Zeroizing::new(body.to_string())
                },
                attributes: attr_view(&secret),
                has_totp: rustpass::totp::has_totp(&secret),
                attachment,
            }
        }
        RevisionContent::Undecryptable => RevisionView::Undecryptable,
        RevisionContent::Deleted => RevisionView::Deleted,
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// One page of revisions for `entry_path` (newest first), each annotated with
/// verification status. Pure metadata — no identity unlock, no decrypt. Page 0
/// passes `base_oid: None`; later pages pass the prior page's `base_oid`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn list_revisions(
    state: State<'_, AppState>,
    repo_id: RepoId,
    entry_path: String,
    offset: usize,
    limit: usize,
    base_oid: Option<String>,
) -> Result<RevisionPage, Error> {
    let store = state.repo(&repo_id)?;
    let name = entry_path.trim_end_matches(".age");
    let page = store
        .list_revisions(name, offset, clamp_limit(limit), base_oid.as_deref())
        .await?;
    Ok(RevisionPage {
        commits: page.commits,
        has_more: page.has_more,
        base_oid: page.base_oid,
    })
}

/// View-revision core, runtime-generic so in-crate tests can drive it against
/// the mock runtime. Mirrors [`crate::read::show_password_core`]: the lock-timer
/// reset + Immediate soft-wipe fire on BOTH paths (a failed read still counts as
/// a secret access).
pub(crate) async fn show_revision_core<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    store: &Arc<rustpass::Store>,
    entry_path: &str,
    commit: &str,
) -> Result<RevisionView, Error> {
    log::info!(
        "show revision: {} @ {commit}",
        entry_path.trim_end_matches(".age")
    );
    let content = store.get_at_revision(entry_path, commit).await;
    reset_lock_timer(state, app, store);
    reset_gate_idle_timer(state, app, store);
    maybe_soft_wipe(state, app, store).await;
    let content = content.inspect_err(|e| {
        log::warn!("show revision failed: {entry_path}@{commit}: {e}");
    })?;
    Ok(revision_view(content))
}

/// View one revision: decrypt and return it for display. The past value crosses
/// IPC only on the `Decrypted` arm — `Undecryptable`/`Deleted` carry no content.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn show_revision(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: RepoId,
    entry_path: String,
    commit: String,
) -> Result<RevisionView, Error> {
    let store = state.repo(&repo_id)?;
    show_revision_core(&state, &app, &store, &entry_path, &commit).await
}

/// Copy a past revision's password straight to the clipboard — the past value
/// never reaches the `WebView`. Mirrors [`crate::read::copy_password`]'s
/// lock-timer reset + Immediate wipe on both paths. Only a `Decrypted`
/// non-attachment revision copies; `Undecryptable`/`Deleted` error, and an
/// attachment revision short-circuits with no clipboard write (the `WebView`
/// disables copy for those states).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn copy_revision(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: RepoId,
    entry_path: String,
    commit: String,
    notify_text: Option<NotifyText>,
) -> Result<CopyResult, Error> {
    let store = state.repo(&repo_id)?;
    let entry_name = entry_path.trim_end_matches(".age").to_string();
    log::info!("copy revision: {entry_name}@{commit}");

    let content = store.get_at_revision(&entry_path, &commit).await;
    reset_lock_timer(&state, &app, &store);
    reset_gate_idle_timer(&state, &app, &store);
    maybe_soft_wipe(&state, &app, &store).await;
    let content = content.inspect_err(|e| {
        log::warn!("copy revision failed: {entry_name}@{commit}: {e}");
    })?;
    let secret = match content {
        RevisionContent::Decrypted(secret) => secret,
        RevisionContent::Undecryptable => {
            return Err(Error::new(
                ErrorCode::DecryptFailed,
                "This revision can't be decrypted with the current identity.",
            ));
        }
        RevisionContent::Deleted => {
            return Err(Error::new(
                ErrorCode::EntryNotFound,
                "This revision deleted the entry — nothing to copy.",
            ));
        }
    };

    let has_totp = rustpass::totp::has_totp(&secret);
    let has_attachment = rustpass::has_attachment(&secret);
    if has_attachment {
        // An attachment has no password — don't clobber the clipboard with
        // empty for the auto-clear window. The UI hides copy for attachment
        // entries; this guards a revision that is an attachment anyway (an
        // entry whose type changed over history). Mirrors copy_password.
        return Ok(CopyResult {
            success: true,
            entry_name,
            cleared_after_secs: 0,
            has_totp,
            has_attachment,
            password_non_utf8: false,
            password_empty: false,
        });
    }

    // A non-UTF-8 password can't be placed on the (UTF-8) clipboard and can't
    // be shown or edited — the gopass CLI is the only path. Skip the clipboard
    // write and tell the UI, mirroring copy_password.
    if !secret.password_is_utf8() {
        return Ok(CopyResult {
            success: true,
            entry_name,
            cleared_after_secs: 0,
            has_totp,
            has_attachment,
            password_non_utf8: true,
            password_empty: false,
        });
    }

    // An empty password (a bare legacy-YAML document, A004) is a fake success
    // over an empty clipboard — skip the write and tell the UI, mirroring
    // copy_password.
    if secret.password_bytes().is_empty() {
        return Ok(CopyResult {
            success: true,
            entry_name,
            cleared_after_secs: 0,
            has_totp,
            has_attachment,
            password_non_utf8: false,
            password_empty: true,
        });
    }

    let cleared_after_secs = crate::clipboard::write_and_schedule_clear(
        &state,
        &app,
        secret.password().to_string(),
        notify_text.as_ref(),
    )
    .await?;
    Ok(CopyResult {
        success: true,
        entry_name,
        cleared_after_secs,
        has_totp,
        has_attachment,
        password_non_utf8: false,
        password_empty: false,
    })
}
