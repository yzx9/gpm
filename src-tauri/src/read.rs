// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret-read commands — list, decrypt-and-copy, and decrypt-and-show. The
//! read side of the store, mirroring [`crate::write`] on the write side.

use std::fmt;

use rustpass::{Entry, Error, RankedPage};
use serde::Serialize;
use tauri::{AppHandle, Runtime, State};
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::{maybe_soft_wipe, reset_lock_timer};
use crate::page::clamp_limit;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `copy_password` — no secret data, safe for IPC.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CopyResult {
    success: bool,
    entry_name: String,
    cleared_after_secs: u32,
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

/// Returned by `show_password` — contains secrets, strict Vue lifecycle required.
#[derive(Clone, Serialize)]
pub(crate) struct SensitiveContent {
    pub(crate) password: Zeroizing<String>,
    pub(crate) notes: Zeroizing<String>,
}

/// Redacts secrets — mirrors `rustpass::Secret` so `Debug` never leaks plaintext.
impl fmt::Debug for SensitiveContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SensitiveContent")
            .field("password", &"[REDACTED]")
            .field("notes", &"[REDACTED]")
            .finish()
    }
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
    page_from(
        state.store.list_page(offset, clamp_limit(limit)).await,
        offset,
    )
}

/// Fuzzy-search `.age` entries by `query`, ranked by relevance (best score
/// first; ties broken by `path`), and return one page starting at `offset` of
/// up to `limit` entries. An empty query behaves like [`list_entries`].
/// Ranking is computed server-side via [`Store::search_page`](rustpass::Store::search_page).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn search_entries(
    state: State<'_, AppState>,
    query: String,
    offset: usize,
    limit: usize,
) -> Result<EntryPage, Error> {
    page_from(
        state
            .store
            .search_page(&query, offset, clamp_limit(limit))
            .await,
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
    maybe_soft_wipe(&state, &app).await;
    let secret = secret.inspect_err(|e| log::warn!("copy failed: {entry_name}: {e}"))?;

    // Clipboard write + cancellable auto-clear + sticky notification, shared
    // with `copy_totp` via the helper. The password never reaches the WebView —
    // only the resolved auto-clear seconds return here.
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
    let secret = state.store.get(entry_path).await;
    reset_lock_timer(state, app);
    maybe_soft_wipe(state, app).await;
    let secret = secret.inspect_err(|e| {
        log::warn!("show failed: {}: {e}", entry_path.trim_end_matches(".age"));
    })?;
    Ok(SensitiveContent {
        password: Zeroizing::new(secret.password().to_string()),
        notes: Zeroizing::new(secret.body().to_string()),
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
    maybe_soft_wipe(&state, &app).await;
    let secret = secret.inspect_err(|e| log::warn!("copy failed: {entry_name}: {e}"))?;

    let Some(otp) = rustpass::totp::extract(secret.body())? else {
        // No TOTP seed: don't touch the clipboard. A prior copy's auto-clear
        // timer is left intact; `cleared_after_secs` is unused on this branch.
        return Ok(TotpCopyResult {
            copied: false,
            entry_name,
            cleared_after_secs: 0,
        });
    };
    let code = rustpass::totp::generate_at(&otp, std::time::SystemTime::now())?;
    let cleared_after_secs = crate::clipboard::write_and_schedule_clear(
        &state,
        &app,
        (*code).clone(),
        notify_text.as_ref(),
    )
    .await?;
    Ok(TotpCopyResult {
        copied: true,
        entry_name,
        cleared_after_secs,
    })
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
        };
        assert_eq!(
            serde_json::to_string(&content).expect("serialize"),
            r#"{"password":"hunter2","notes":"username: alice"}"#
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
}
