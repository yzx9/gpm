// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret-read commands — list, decrypt-and-copy, and decrypt-and-show. The
//! read side of the store, mirroring [`crate::write`] on the write side.

use std::fmt;
use std::time::Duration;

use rustpass::error::ErrorCode;
use rustpass::{Entry, Error, RankedPage};
use serde::Serialize;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::{maybe_soft_wipe, reset_lock_timer};

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

/// Upper bound on a client-requested page size, so a buggy/malicious caller
/// can't ask for `usize::MAX`. The frontend requests 50 by default.
const MAX_PAGE_SIZE: usize = 200;

/// Clamp a client-requested page size to a sane, non-zero bound.
fn clamp_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_PAGE_SIZE)
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
fn clipboard_clear_plan(clear_secs: u64) -> (bool, u32) {
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
) -> Result<CopyResult, Error> {
    let entry_name = entry_path.trim_end_matches(".age").to_string();

    // Decrypt first so a FAILED read still counts as a secret access: under
    // Immediate we reset the timer + wipe on both paths (an errored op must not
    // leave the identity cached with no idle timer to eventually clear it).
    let secret = state.store.get(&entry_path).await;
    reset_lock_timer(&state, &app);
    maybe_soft_wipe(&state, &app).await;
    let secret = secret?;

    app.clipboard()
        .write_text(secret.password().to_string())
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("Clipboard error: {e}")))?;

    // Clipboard auto-clear: configured seconds, or never when the user set it to
    // 0. Read from the AppState cache (not repo.json) — copy is the hot path.
    let clear_secs = state
        .clipboard_clear_secs
        .lock()
        .map_or_else(|_| rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS, |s| *s);
    let (spawn_clear, cleared_after_secs) = clipboard_clear_plan(clear_secs);
    if spawn_clear {
        // The OS clipboard already holds the password; the spawned task just
        // blanks it after the configured window. No need to keep a heap copy of
        // the plaintext alive for the sleep (the decoded `secret` is zeroized on
        // drop at end of scope).
        let clear_handle = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(clear_secs)).await;
            let _ = clear_handle.clipboard().write_text(String::new());
        });
    }

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
    let secret = state.store.get(entry_path).await;
    reset_lock_timer(state, app);
    maybe_soft_wipe(state, app).await;
    let secret = secret?;
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

#[cfg(test)]
mod tests {
    //! Pagination envelope logic — the Tauri-layer bits `rustpass` can't test:
    //! [`clamp_limit`] bounds a client-requested page size, and [`page_from`]
    //! derives `has_more` from the offset/total (the classic off-by-one). Pure
    //! fns, no Store needed.

    use super::*;

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
    fn clamp_limit_bounds_request_size() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(1), 1);
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(MAX_PAGE_SIZE), MAX_PAGE_SIZE);
        assert_eq!(clamp_limit(MAX_PAGE_SIZE + 1), MAX_PAGE_SIZE);
        assert_eq!(clamp_limit(usize::MAX), MAX_PAGE_SIZE);
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
