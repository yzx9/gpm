// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret-read commands — list, decrypt-and-copy, and decrypt-and-show. The
//! read side of the store, mirroring [`crate::write`] on the write side.

use std::fmt;
use std::path::Path;
use std::time::SystemTime;

use rustpass::{AttachmentMeta, Entry, Error, ErrorCode, RankedPage};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_file_save::FileSaveExt;
use tokio::fs;
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::{maybe_soft_wipe, reset_gate_idle_timer, reset_lock_timer};
use crate::page::clamp_limit;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `copy_password` — no secret data, safe for IPC.
#[derive(Debug, Clone, Serialize)]
// Four independent decrypt byproducts cross IPC here (each is a free hint so
// the UI avoids a second read); they aren't a state machine, so bools beat an
// enum that would distort the wire shape.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct CopyResult {
    pub(crate) success: bool,
    pub(crate) entry_name: String,
    pub(crate) cleared_after_secs: u32,
    /// A free byproduct of this decrypt: whether the entry's body carries a
    /// TOTP seed, so the UI can show/hide the 2FA affordance without a second
    /// read. No secret data.
    pub(crate) has_totp: bool,
    /// A free byproduct of this decrypt: whether the entry's body is a binary
    /// attachment, so the UI can switch to the Export affordance without a
    /// second read. No secret data.
    pub(crate) has_attachment: bool,
    /// A free byproduct of this decrypt: whether the password (first line)
    /// isn't valid UTF-8, so the UI can refuse the clipboard write and point
    /// the user at the gopass CLI instead. No secret data.
    pub(crate) password_non_utf8: bool,
}

/// Returned by `copy_totp`. Like [`CopyResult`] but distinguishes "copied a
/// code" from "the entry has no TOTP seed" (`copied == false`, no clipboard
/// write). No secret data — neither the seed nor the code crosses IPC.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TotpCopyResult {
    /// `false` when the entry holds no TOTP seed (no clipboard write happened).
    copied: bool,
    entry_name: String,
    cleared_after_secs: u32,
}

/// Why an entry's Edit affordance is disabled. A non-UTF-8 secret can't be
/// safely round-tripped through a UTF-8 text editor — editing its lossy view
/// and saving would corrupt the original bytes — so the UI edit-blocks it.
/// Not secret.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EditBlockReason {
    NonUtf8,
}

/// Returned by `show_password` — contains secrets, strict Vue lifecycle required.
#[derive(Clone, Serialize)]
pub(crate) struct SensitiveContent {
    pub(crate) password: Zeroizing<String>,
    pub(crate) notes: Zeroizing<String>,
    /// A free byproduct of this decrypt: whether the entry's body carries a
    /// TOTP seed, so the UI can show/hide the 2FA affordance without a second
    /// read. Not itself secret.
    pub(crate) has_totp: bool,
    /// A free byproduct of this decrypt: when `Some`, the entry is a binary
    /// attachment — the UI hides the (empty/base64) password + notes block and
    /// shows Export + this metadata instead. `notes` is cleared in that case so
    /// the base64 body never reaches the `WebView`. Not itself secret.
    pub(crate) attachment: Option<AttachmentMeta>,
    /// When `Some`, the entry cannot be safely text-edited (e.g. non-UTF-8
    /// content) and the UI disables Edit with a reason-specific hint. Not secret.
    pub(crate) edit_blocked: Option<EditBlockReason>,
    /// The blob oid (base version) captured atomically with this decrypt — the
    /// R026 base-version the edit screen sends back as `base_oid` to guard a save
    /// against a stale snapshot. Non-secret; `None` only if a producer doesn't
    /// capture one (`show_password` always does, via `get_with_oid`).
    pub(crate) version: Option<String>,
}

/// Redacts secrets — mirrors `rustpass::Secret` so `Debug` never leaks plaintext.
impl fmt::Debug for SensitiveContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SensitiveContent")
            .field("password", &"[REDACTED]")
            .field("notes", &"[REDACTED]")
            .field("has_totp", &self.has_totp)
            .field("attachment", &self.attachment)
            .field("edit_blocked", &self.edit_blocked)
            .field("version", &self.version)
            .finish()
    }
}

/// Returned by `entry_probe` — one decrypt gives both the 2FA-presence signal
/// and attachment metadata, halving the decrypts vs two separate probes.
/// `None` when the identity is encrypted + not cached (the probe never prompts).
/// No secret data.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EntryProbe {
    pub(crate) has_totp: bool,
    pub(crate) attachment: Option<AttachmentMeta>,
    /// When `Some`, the entry can't be safely text-edited (e.g. non-UTF-8
    /// content); the detail view greys Edit + shows a reason hint, mirroring the
    /// attachment case. Not secret.
    pub(crate) edit_blocked: Option<EditBlockReason>,
}

/// Returned by `export_attachment` — no secret data. `exported == false` means
/// the entry holds no modern attachment (nothing staged, no dialog shown).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AttachmentExportResult {
    pub(crate) exported: bool,
    pub(crate) entry_name: String,
}

/// One page of entries delivered to the `WebView` — a slice of the ranked set
/// plus the total match count and a `has_more` flag the UI gates "load more"
/// on. Presentation metadata only; like `CopyResult`/`SensitiveContent` it
/// lives here, not in `rustpass`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EntryPage {
    entries: Vec<Entry>,
    /// Total entries matching the query, independent of this page's offset/limit.
    total: usize,
    /// `true` when more pages remain past this slice.
    has_more: bool,
}

/// Build the IPC page envelope from a backend [`RankedPage`], deriving
/// `has_more` from the offset the page was requested at.
fn page_from(r: Result<RankedPage, Error>, offset: usize) -> Result<EntryPage, Error> {
    let p = r?;
    let has_more = offset + p.entries.len() < p.total;
    Ok(EntryPage {
        entries: p.entries,
        total: p.total,
        has_more,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// One page of `.age` entries in the configured repository, starting at
/// `offset` and up to `limit` long. An empty query (browse) path.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn list_entries(
    state: State<'_, AppState>,
    offset: usize,
    limit: usize,
) -> Result<EntryPage, Error> {
    page_from(state.store.list(offset, clamp_limit(limit)).await, offset)
}

/// Fuzzy-search `.age` entries by `query`, ranked by relevance (best score
/// first; ties broken by `path`), and return one page starting at `offset` of
/// up to `limit` entries. An empty query behaves like [`list_entries`].
/// Ranking is computed server-side via [`Store::search`](rustpass::Store::search).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn search_entries(
    state: State<'_, AppState>,
    query: String,
    offset: usize,
    limit: usize,
) -> Result<EntryPage, Error> {
    page_from(
        state.store.search(&query, offset, clamp_limit(limit)).await,
        offset,
    )
}

/// Resolve configured clipboard-clear seconds into (whether to spawn a clear
/// task, the value to report to the UI). `0` (Never) spawns nothing and reports
/// `0`; a nonzero value spawns and reports itself, clamped into `u32`. Pure so
/// the Never/nonzero contract is unit-testable without a clipboard.
#[must_use]
pub(crate) fn clipboard_clear_plan(clear_secs: u64) -> (bool, u32) {
    if clear_secs == 0 {
        (false, 0)
    } else {
        (true, u32::try_from(clear_secs).unwrap_or(u32::MAX))
    }
}

/// Primary operation: decrypt and copy password to clipboard.
/// Password never reaches the `WebView`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn copy_password(
    state: State<'_, AppState>,
    app: AppHandle,
    entry_path: String,
    notify_text: Option<tauri_plugin_clipboard_notify::NotifyText>,
) -> Result<CopyResult, Error> {
    let entry_name = entry_path.trim_end_matches(".age").to_string();
    log::info!("copy: {entry_name}");

    // Decrypt first so a FAILED read still counts as a secret access: under
    // Immediate we reset the timer + wipe on both paths (an errored op must not
    // leave the identity cached with no idle timer to eventually clear it).
    let secret = state.store.get(&entry_path).await;
    reset_lock_timer(&state, &app);
    reset_gate_idle_timer(&state, &app);
    maybe_soft_wipe(&state, &app).await;
    let secret = secret.inspect_err(|e| log::warn!("copy failed: {entry_name}: {e}"))?;
    let has_totp = rustpass::totp::has_totp(&secret);
    let has_attachment = rustpass::has_attachment(&secret);
    if has_attachment {
        // An attachment has no password — don't clobber the clipboard with empty
        // for the auto-clear window. The UI offers Export instead.
        return Ok(CopyResult {
            success: true,
            entry_name,
            cleared_after_secs: 0,
            has_totp,
            has_attachment,
            password_non_utf8: false,
        });
    }

    // A non-UTF-8 password can't be placed on the (UTF-8) clipboard, and the UI
    // can't show it (lossy view) or edit it (edit-blocked) — the gopass CLI is
    // the only path. Skip the clipboard write and tell the UI, rather than
    // crowning an empty copy with a "Copied!" toast.
    if !secret.password_is_utf8() {
        return Ok(CopyResult {
            success: true,
            entry_name,
            cleared_after_secs: 0,
            has_totp,
            has_attachment,
            password_non_utf8: true,
        });
    }

    // Clipboard write + cancellable auto-clear + sticky notification, shared
    // with `copy_totp` via the helper. The password never reaches the WebView —
    // only the resolved auto-clear seconds return here.
    let cleared_after_secs = crate::clipboard::write_and_schedule_clear(
        &state,
        &app,
        secret.password().to_string(),
        notify_text.as_ref(),
    )
    .await
    .inspect_err(|e| log::warn!("copy failed: clipboard stage: {entry_name}: {e}"))?;

    Ok(CopyResult {
        success: true,
        entry_name,
        cleared_after_secs,
        has_totp,
        has_attachment,
        password_non_utf8: false,
    })
}

/// Decrypt-and-show core, runtime-generic so the in-crate tests can drive it
/// against the mock runtime. Reads the entry, then — under Immediate — resets
/// the timer and soft-wipes the identity on BOTH paths (a failed read must not
/// leave it cached). The decoded secret lives in the returned `SensitiveContent`
/// independently of the identity cache, so wiping after the read is safe.
pub(crate) async fn show_password_core<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    entry_path: &str,
) -> Result<SensitiveContent, Error> {
    log::info!("show: {}", entry_path.trim_end_matches(".age"));
    let read = state.store.get_with_oid(entry_path).await;
    reset_lock_timer(state, app);
    reset_gate_idle_timer(state, app);
    maybe_soft_wipe(state, app).await;
    let (secret, oid) = read.inspect_err(|e| {
        log::warn!("show failed: {}: {e}", entry_path.trim_end_matches(".age"));
    })?;
    let body = secret.body();
    let attachment = rustpass::metadata(&secret);
    Ok(SensitiveContent {
        password: Zeroizing::new(secret.password().to_string()),
        // For an attachment the body is the attribute lines + a base64 wall;
        // clear it so the blob never reaches the WebView — the metadata +
        // Export carry the entry instead.
        notes: if attachment.is_some() {
            Zeroizing::new(String::new())
        } else {
            Zeroizing::new(body.to_string())
        },
        has_totp: rustpass::totp::has_totp(&secret),
        // A non-UTF-8 secret can't be safely edited as text (the lossy view
        // would be re-encrypted on save, corrupting it) — flag it so the UI
        // edit-blocks. Attachments are base64 (valid UTF-8), so they don't trip
        // this; they stay blocked via `attachment`.
        edit_blocked: if secret.is_utf8() {
            None
        } else {
            Some(EditBlockReason::NonUtf8)
        },
        attachment,
        version: Some(oid),
    })
}

/// Secondary operation: decrypt and return password for display.
/// Password crosses IPC — Vue component must follow strict lifecycle.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn show_password(
    state: State<'_, AppState>,
    app: AppHandle,
    entry_path: String,
) -> Result<SensitiveContent, Error> {
    show_password_core(&state, &app, &entry_path).await
}

/// Blob oid (base version) of `entry` at HEAD, or `null` if absent — the R026
/// base-version capture for a base-version-aware delete, fetched on the detail
/// page mount so a delete-without-reveal is still protected. Non-secret: no
/// identity, no decrypt.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn entry_oid(
    state: State<'_, AppState>,
    entry_path: String,
) -> Result<Option<String>, Error> {
    state.store.entry_oid(&entry_path).await
}

/// Decrypt the entry, compute its TOTP code in Rust, and copy it to the
/// clipboard. Neither the seed nor the code reaches the `WebView` — only this
/// result. `copied == false` means the entry has no TOTP seed (no clipboard
/// write). Mirrors [`copy_password`]'s lock-timer reset + Immediate wipe on
/// both paths, so a failed read still counts as a secret access.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn copy_totp(
    state: State<'_, AppState>,
    app: AppHandle,
    entry_path: String,
    notify_text: Option<tauri_plugin_clipboard_notify::NotifyText>,
) -> Result<TotpCopyResult, Error> {
    let entry_name = entry_path.trim_end_matches(".age").to_string();
    log::info!("copy-totp: {entry_name}");

    // Decrypt first so a FAILED read still counts as a secret access (Immediate).
    let secret = state.store.get(&entry_path).await;
    reset_lock_timer(&state, &app);
    reset_gate_idle_timer(&state, &app);
    maybe_soft_wipe(&state, &app).await;
    let secret = secret.inspect_err(|e| log::warn!("copy failed: {entry_name}: {e}"))?;

    let Some(otp) = rustpass::totp::extract(&secret)
        .inspect_err(|e| log::warn!("copy-totp failed: extract: {entry_name}: {e}"))?
    else {
        // No TOTP seed: don't touch the clipboard. A prior copy's auto-clear
        // timer is left intact; `cleared_after_secs` is unused on this branch.
        return Ok(TotpCopyResult {
            copied: false,
            entry_name,
            cleared_after_secs: 0,
        });
    };
    let code = rustpass::totp::generate_at(&otp, SystemTime::now())
        .inspect_err(|e| log::warn!("copy-totp failed: generate: {entry_name}: {e}"))?;
    let cleared_after_secs = crate::clipboard::write_and_schedule_clear(
        &state,
        &app,
        (*code).clone(),
        notify_text.as_ref(),
    )
    .await
    .inspect_err(|e| log::warn!("copy-totp failed: clipboard stage: {entry_name}: {e}"))?;
    Ok(TotpCopyResult {
        copied: true,
        entry_name,
        cleared_after_secs,
    })
}

/// One-shot entry probe — **never triggers an unlock**. A single decrypt
/// returns both whether the body carries a TOTP seed and whether it is a binary
/// attachment (with metadata), so the detail view settles both affordances from
/// one read instead of two. The only "would need a prompt" outcome is an
/// encrypted identity that is not cached: `Store::get` then fails with
/// `IDENTITY_ENCRYPTED`, and we return `Ok(None)` ("unknown") instead of
/// prompting. Mirrors the read commands' lock-timer reset + Immediate wipe on
/// the decrypt path; the not-cached branch touches no timers (no access).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn entry_probe(
    state: State<'_, AppState>,
    app: AppHandle,
    entry_path: String,
) -> Result<Option<EntryProbe>, Error> {
    let secret = state.store.get(&entry_path).await;
    // Encrypted + not cached ⇒ would need an unlock prompt. Signal "unknown"
    // and never prompt.
    let secret = match secret {
        Err(e) if e.code == "IDENTITY_ENCRYPTED" => return Ok(None),
        s => s,
    };
    reset_lock_timer(&state, &app);
    reset_gate_idle_timer(&state, &app);
    maybe_soft_wipe(&state, &app).await;
    let secret = secret.inspect_err(|e| log::warn!("entry-probe failed: {entry_path}: {e}"))?;
    Ok(Some(EntryProbe {
        has_totp: rustpass::totp::has_totp(&secret),
        attachment: rustpass::metadata(&secret),
        edit_blocked: if secret.is_utf8() {
            None
        } else {
            Some(EditBlockReason::NonUtf8)
        },
    }))
}

/// Detect a binary attachment and export its decoded bytes to a user-chosen
/// file — decrypt → base64-decode → stage → save. The decoded bytes never reach
/// the `WebView`; only this non-secret result crosses IPC. `exported == false`
/// means the entry holds no modern attachment. Mirrors `copy_password`'s
/// decrypt-first + Immediate-wipe-on-both-paths lifecycle.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn export_attachment(
    state: State<'_, AppState>,
    app: AppHandle,
    entry_path: String,
) -> Result<AttachmentExportResult, Error> {
    export_attachment_core(&state, &app, &entry_path).await
}

/// Runtime-generic core of [`export_attachment`], so in-crate tests can drive it
/// against the mock runtime. See [`export_attachment`] for the contract.
pub(crate) async fn export_attachment_core<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    entry_path: &str,
) -> Result<AttachmentExportResult, Error> {
    let entry_name = entry_path.trim_end_matches(".age").to_string();
    log::info!("export-attachment: {entry_name}");

    // Single-flight first: the Android save plugin tracks one pending picker,
    // so acquire the slot before paying for decrypt + decode — a losing
    // concurrent export fails fast with REPO_BUSY instead of churning the
    // identity cache for nothing.
    let _guard = crate::export_guard::FileSaveGuard::acquire()?;

    // Decrypt first so a FAILED read still counts as a secret access: under
    // Immediate we reset the timer + wipe on both paths (an errored op must not
    // leave the identity cached with no idle timer to eventually clear it).
    let secret = state.store.get(entry_path).await;
    reset_lock_timer(state, app);
    reset_gate_idle_timer(state, app);
    maybe_soft_wipe(state, app).await;
    let secret =
        secret.inspect_err(|e| log::warn!("export-attachment failed: {entry_name}: {e}"))?;

    // Detect + decode. Bytes never reach the WebView.
    let Some(attachment) = rustpass::attachment::extract(&secret)? else {
        return Ok(AttachmentExportResult {
            exported: false,
            entry_name,
        });
    };

    // Stage the decoded bytes, then hand the path to the save plugin; the
    // StageGuard wipes the stage on drop regardless of outcome (incl. panic).
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("cache dir unavailable: {e}")))?;
    let temp_path = cache_dir.join(STAGE_FILENAME);
    let _stage = StageGuard::new(&temp_path);
    fs::write(&temp_path, attachment.bytes())
        .await
        .map_err(|e| {
            Error::new(
                ErrorCode::IoError,
                format!("failed to stage attachment: {e}"),
            )
        })?;
    // The stage holds decrypted bytes; 0600 keeps a stage stranded by a hard
    // kill readable only by the app/user during the window before the
    // next-launch `sweep_attachment_stage` wipes it. Best-effort: a perms
    // failure is unusual and not worth aborting the export over.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600)).await
        {
            log::warn!("export-attachment: stage perms 0600 failed: {e}");
        }
    }

    let suggested = sanitize_save_name(attachment.filename(), &entry_name);
    let save_result = app
        .file_save()
        .save(
            suggested,
            temp_path.clone(),
            "application/octet-stream".to_string(),
        )
        .await;
    map_save_result(save_result)?;

    Ok(AttachmentExportResult {
        exported: true,
        entry_name,
    })
}

// ---------------------------------------------------------------------------
// Attachment-export helpers (pure / unit-testable without the save dialog)
// ---------------------------------------------------------------------------

/// Filename used for the staged decoded attachment in the app cache dir.
const STAGE_FILENAME: &str = "gpm-attachment.bin";

/// Strip path separators from `filename` (the `Content-Disposition` value) or
/// fall back to `entry_name`'s basename, mirroring gopass's `filepath.Base`. The
/// save dialog must never receive a name carrying a path separator — an entry
/// path like `servers/prod` would otherwise break the suggested name.
#[must_use]
pub(crate) fn sanitize_save_name(filename: Option<&str>, entry_name: &str) -> String {
    let raw = filename.unwrap_or(entry_name);
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let clean: String = base
        .chars()
        .filter(|c| !is_path_sep(*c) && *c != '\0')
        .collect();
    if clean.is_empty() {
        "attachment.bin".to_string()
    } else {
        clean
    }
}

/// A path separator on either platform (`/` Unix/Android, `\` Windows).
fn is_path_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Translate a file-save plugin result into the app error model: `CANCELLED` is
/// a soft cancel (the user dismissed the picker), everything else is a real
/// failure.
fn map_save_result(result: Result<(), tauri_plugin_file_save::FileSaveError>) -> Result<(), Error> {
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.code == "CANCELLED" => Err(Error::new(ErrorCode::Cancelled, "Save cancelled")),
        Err(e) => Err(Error::new(
            ErrorCode::StoreError,
            format!("attachment export failed: {}", e.message),
        )),
    }
}

/// Wipes the staged decoded attachment on drop (best-effort, sync so it works
/// under panic). Constructed before the stage write so a failure between write
/// and save still cleans up.
struct StageGuard<'a> {
    path: &'a Path,
}
impl<'a> StageGuard<'a> {
    fn new(path: &'a Path) -> Self {
        let _ = std::fs::remove_file(path); // best-effort wipe of a stranded prior stage
        Self { path }
    }
}
impl Drop for StageGuard<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path);
    }
}

/// Best-effort removal of a stranded attachment stage from a prior run killed
/// mid-export (`StageGuard`'s Drop runs on panic/cancel but not on SIGKILL).
/// Called once at app startup so a hard-killed export doesn't leave decrypted
/// bytes sitting in `app_cache_dir` until the next export overwrites them.
pub(crate) async fn sweep_attachment_stage<R: Runtime>(app: &AppHandle<R>) {
    let Ok(cache_dir) = app.path().app_cache_dir() else {
        return;
    };
    let _ = fs::remove_file(cache_dir.join(STAGE_FILENAME)).await;
}

#[cfg(test)]
mod tests {
    //! Pagination envelope logic — the Tauri-layer bits `rustpass` can't test:
    //! [`clamp_limit`] bounds a client-requested page size, and [`page_from`]
    //! derives `has_more` from the offset/total (the classic off-by-one). Pure
    //! fns, no Store needed.

    use super::*;
    use rustpass::error::ErrorCode;

    fn entry(name: &str) -> Entry {
        Entry {
            path: format!("{name}.age"),
            name: name.to_string(),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn ok_page(entries: Vec<Entry>, total: usize) -> Result<RankedPage, Error> {
        Ok(RankedPage { entries, total })
    }

    #[test]
    fn page_from_empty_has_no_more() {
        let p = page_from(ok_page(vec![], 0), 0).unwrap();
        assert_eq!(p.total, 0);
        assert!(!p.has_more);
    }

    #[test]
    fn page_from_full_page_with_remaining_has_more() {
        // 5 of 12 at offset 0 → 0 + 5 < 12.
        let p = page_from(ok_page(vec![entry("a"); 5], 12), 0).unwrap();
        assert_eq!(p.entries.len(), 5);
        assert_eq!(p.total, 12);
        assert!(p.has_more);
    }

    #[test]
    fn page_from_exact_fill_has_no_more() {
        // Page fills exactly to total → no more (the off-by-one: `<`, not `<=`).
        let p = page_from(ok_page(vec![entry("a"); 5], 5), 0).unwrap();
        assert!(!p.has_more);
    }

    #[test]
    fn page_from_partial_last_page_has_no_more() {
        // Offset 5, 3 returned, total 8 → 5 + 3 == 8 → last page.
        let p = page_from(ok_page(vec![entry("a"); 3], 8), 5).unwrap();
        assert!(!p.has_more);
    }

    #[test]
    fn page_from_mid_offset_with_remaining_has_more() {
        // Offset 5, 3 returned, total 12 → 5 + 3 < 12 → more remain.
        let p = page_from(ok_page(vec![entry("a"); 3], 12), 5).unwrap();
        assert!(p.has_more);
    }

    #[test]
    fn page_from_propagates_store_error() {
        let err = Error::new(ErrorCode::StoreError, "boom");
        assert!(page_from(Err(err), 0).is_err());
    }

    #[test]
    fn sensitive_content_serializes_transparently() {
        // `Zeroizing<String>` must serialize as a plain JSON string so the
        // Vue frontend's `SensitiveContent` shape stays unchanged, and `Debug`
        // must never leak the plaintext.
        let content = SensitiveContent {
            password: Zeroizing::new("hunter2".to_string()),
            notes: Zeroizing::new("username: alice".to_string()),
            has_totp: true,
            attachment: None,
            edit_blocked: None,
            version: None,
        };
        assert_eq!(
            serde_json::to_string(&content).expect("serialize"),
            r#"{"password":"hunter2","notes":"username: alice","has_totp":true,"attachment":null,"edit_blocked":null,"version":null}"#
        );
        assert!(!format!("{content:?}").contains("hunter2"));
    }

    #[test]
    fn clipboard_clear_plan_never_skips_spawn_and_reports_zero() {
        // 0 (Never): no clear task, UI shows 0.
        assert_eq!(clipboard_clear_plan(0), (false, 0));
    }

    #[test]
    fn clipboard_clear_plan_nonzero_spawns_and_reports_itself() {
        assert_eq!(clipboard_clear_plan(45), (true, 45));
        assert_eq!(clipboard_clear_plan(180), (true, 180));
    }

    #[test]
    fn clipboard_clear_plan_clamps_huge_values_into_u32() {
        // A hand-edited config could carry a value beyond u32; the UI must not
        // panic on the cast.
        assert_eq!(
            clipboard_clear_plan(u64::from(u32::MAX) + 1),
            (true, u32::MAX)
        );
    }

    // ---- attachment-export pure helpers ----

    #[test]
    fn sanitize_save_name_uses_header_filename() {
        assert_eq!(
            sanitize_save_name(Some("photo.png"), "ignored"),
            "photo.png"
        );
    }

    #[test]
    fn sanitize_save_name_strips_path_separators() {
        // gopass writes filepath.Base (no separators), but a hand-crafted store
        // could carry them — strip so the save dialog never gets a path.
        assert_eq!(sanitize_save_name(Some("../evil.png"), "x"), "evil.png");
        assert_eq!(sanitize_save_name(Some("a/b/c.bin"), "x"), "c.bin");
    }

    #[test]
    fn sanitize_save_name_falls_back_to_entry_basename() {
        // No Content-Disposition filename → entry name's basename.
        assert_eq!(sanitize_save_name(None, "servers/prod"), "prod");
        assert_eq!(sanitize_save_name(None, "top"), "top");
    }

    #[test]
    fn sanitize_save_name_final_fallback_when_empty() {
        assert_eq!(sanitize_save_name(Some(""), ""), "attachment.bin");
        assert_eq!(sanitize_save_name(None, ""), "attachment.bin");
    }

    #[test]
    fn map_save_result_categorizes_outcomes() {
        use tauri_plugin_file_save::FileSaveError;
        assert!(map_save_result(Ok(())).is_ok());
        let cancelled = map_save_result(Err(FileSaveError {
            code: "CANCELLED".into(),
            message: "dismissed".into(),
        }));
        assert_eq!(cancelled.unwrap_err().code, "CANCELLED");
        let failed = map_save_result(Err(FileSaveError {
            code: "IO_ERROR".into(),
            message: "disk full".into(),
        }));
        assert_eq!(failed.unwrap_err().code, "STORE_ERROR");
    }

    // ---- StageGuard wipes the staged decoded bytes (security-load-bearing) ----

    #[test]
    fn stage_guard_wipes_staged_file_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gpm-attachment.bin");
        {
            let _guard = StageGuard::new(&path);
            // Simulate the stage write that happens after construction.
            std::fs::write(&path, b"decoded bytes").unwrap();
            assert!(path.exists(), "stage exists while guard is held");
        }
        assert!(!path.exists(), "StageGuard::drop must wipe the staged file");
    }

    #[test]
    fn stage_guard_wipes_on_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gpm-attachment.bin");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = StageGuard::new(&path);
            std::fs::write(&path, b"decoded bytes").unwrap();
            panic!("simulated mid-export panic");
        }));
        assert!(result.is_err(), "the panic should propagate");
        assert!(
            !path.exists(),
            "StageGuard::drop must wipe the staged file even on panic"
        );
    }
}
