// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! In-app diagnostics viewer commands — read and clear the rotated log file the
//! `tauri-plugin-log` `LogDir` target writes under `app_log_dir()`. The plugin
//! exposes no read/clear command of its own (only the JS→Rust `log` forwarder),
//! so these are thin, best-effort helpers for the Settings → Logs viewer
//! (RFC 0052, phase 2).
//!
//! The heavy lifting lives in [`read_log_from`] / [`clear_log_in`] — pure `&Path`
//! helpers split out so the ordering, truncation, and clear semantics are
//! unit-testable with a tempdir (the Tauri commands just resolve the dir + base
//! name and delegate).
//!
//! Security: the log is plaintext by design (RFC 0052) — only entry names and
//! operation outcomes are ever logged, never secret content. The viewer route is
//! `secure: true` so entry-name metadata is screen-protected.

use std::path::Path;

use rustpass::{Error, ErrorCode};
use tauri::{AppHandle, Manager};

/// Maximum bytes shipped to the webview in one [`read_log`] call. The rotated set
/// (`KeepSome(3)` at ~1 MiB each) can be several MiB; tail-truncate to keep the
/// IPC payload and the `<pre>` render cheap on mobile.
const MAX_LOG_BYTES: usize = 256 * 1024;

/// Resolve the log directory (`app_log_dir()`), mapping a path error to
/// `StoreError` so the command returns a sanitized `rustpass::Error`.
fn log_dir(app: &AppHandle) -> Result<std::path::PathBuf, Error> {
    app.path()
        .app_log_dir()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("log dir unavailable: {e}")))
}

/// The active log file's base name — mirrors the plugin
/// (`app_handle.package_info().name`; tauri-plugin-log lib.rs:719), so the active
/// `{base}.log` we truncate matches the file the plugin is appending to.
fn log_base(app: &AppHandle) -> String {
    app.package_info().name.clone()
}

/// Read the diagnostics log for the in-app viewer.
///
/// Reads every `{base}*.log` and `{base}*.log.bak` under `dir`, ordered by
/// modification time **ascending** (oldest first → the active file last, since it
/// is being appended to). Filename ordering is intentionally NOT used: the active
/// `gpm.log` sorts *before* a rotated `gpm_2026-…log` because `.` < `_`, which
/// would show the newest segment first. The concatenated output is tail-truncated
/// to [`MAX_LOG_BYTES`] at a newline boundary so the payload stays small. An
/// empty or missing log directory returns an empty string (not an error) — the
/// viewer shows its "empty" state.
async fn read_log_from(dir: &Path, base: &str) -> String {
    let mut files: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
    // Best-effort: a missing/unreadable dir yields an empty log, not an error.
    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_log =
                name.starts_with(base) && (name.ends_with(".log") || name.ends_with(".log.bak"));
            if !is_log {
                continue;
            }
            if let Ok(meta) = entry.metadata().await
                && let Ok(mtime) = meta.modified()
            {
                files.push((entry.path(), mtime));
            }
        }
    }
    files.sort_by_key(|(_, mtime)| *mtime);

    let mut buf = Vec::new();
    for (path, _) in &files {
        if let Ok(bytes) = tokio::fs::read(path).await {
            buf.extend_from_slice(&bytes);
        }
    }
    if buf.len() > MAX_LOG_BYTES {
        let start = buf.len() - MAX_LOG_BYTES;
        // Snap forward to the next newline so the output doesn't begin mid-line.
        // `.get` (not indexing) keeps clippy::indexing_slicing happy; `start` is
        // always in bounds here since `start = len - MAX < len`.
        let start = buf
            .get(start..)
            .and_then(|slice| slice.iter().position(|&b| b == b'\n'))
            .map_or(start, |p| start + p + 1);
        buf.drain(..start);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Clear the diagnostics log.
///
/// Removes rotated `{base}_*.log` / `{base}_*.log.bak` files and truncates the
/// active `{base}.log` **in place** — NOT deleted, because the `tauri-plugin-log`
/// rotator holds the active file open in append mode; deleting it would leave the
/// logger writing to an unlinked inode and new records would silently vanish.
/// Truncating in place keeps the handle valid. One cosmetic side effect: the
/// rotator's in-memory size counter goes stale, so the next size check may trip
/// one premature rotation — harmless, self-heals after that single rotation.
async fn clear_log_in(dir: &Path, base: &str) -> Result<(), Error> {
    let active = dir.join(format!("{base}.log"));
    let rotated_prefix = format!("{base}_");

    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Rotated files are `{base}_*.log` (and `.bak` collision backups).
            // The active `{base}.log` is truncated below, not removed.
            let is_rotated = name.starts_with(&rotated_prefix)
                && (name.ends_with(".log") || name.ends_with(".log.bak"));
            if is_rotated {
                // Best-effort: a file removed by a concurrent rotation is fine.
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }

    // Truncate the active file in place (the held append handle survives). If it
    // does not yet exist (nothing logged yet), this harmlessly creates an empty
    // one the plugin will append to.
    tokio::fs::write(&active, b"")
        .await
        .map_err(|e| Error::new(ErrorCode::IoError, format!("failed to clear log: {e}")))?;
    Ok(())
}

/// Read the diagnostics log for the in-app viewer (Settings → Logs). See
/// [`read_log_from`] for ordering/truncation semantics.
#[tauri::command]
pub(crate) async fn read_log(app: AppHandle) -> Result<String, Error> {
    Ok(read_log_from(&log_dir(&app)?, &log_base(&app)).await)
}

/// Clear the diagnostics log (rotated removed, active truncated in place). See
/// [`clear_log_in`] for why the active file is truncated, not deleted.
#[tauri::command]
pub(crate) async fn clear_log(app: AppHandle) -> Result<(), Error> {
    clear_log_in(&log_dir(&app)?, &log_base(&app)).await
}

/// Frontend logging bridge (RFC 0052, phase 2): write a frontend-emitted record
/// into the same backend pipeline. The global error handlers in `main.ts` call
/// this so an uncaught frontend error leaves a persisted trace for bug reports.
///
/// This is a custom app command (not `@tauri-apps/plugin-log`) deliberately: it
/// avoids a new JS dependency and a capability entry (app commands aren't
/// ACL-gated here) while reaching the exact same logger. `level` is matched
/// case-insensitively; an unrecognized value degrades to `info`. Records below
/// the current `max_level` are dropped by the `log` macros as usual.
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)] // Tauri IPC needs owned args + the Result shape (mirrors git.rs)
pub(crate) fn write_log(level: String, message: String) -> Result<(), Error> {
    let level = match level.to_ascii_lowercase().as_str() {
        "error" => log::Level::Error,
        "warn" => log::Level::Warn,
        "debug" => log::Level::Debug,
        "trace" => log::Level::Trace,
        _ => log::Level::Info, // "info" and anything unrecognized degrade to Info
    };
    log::log!(level, "frontend: {message}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    /// Write `contents` to `dir/name` and set its mtime to `secs_ago` seconds in
    /// the past, so the ordering test doesn't depend on filesystem time resolution.
    fn write_with_mtime(dir: &Path, name: &str, contents: &[u8], secs_ago: u64) {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let mt = SystemTime::now() - Duration::from_secs(secs_ago);
        f.set_times(std::fs::FileTimes::new().set_modified(mt))
            .unwrap();
    }

    #[tokio::test]
    async fn read_log_orders_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        // Rotated (older), active (newer). `gpm.log` < `gpm_…log` by filename,
        // so a filename sort would wrongly put the active segment first.
        write_with_mtime(
            dir.path(),
            "gpm_2026-01-01_00-00-00.log",
            b"old-line\n",
            120,
        );
        write_with_mtime(dir.path(), "gpm.log", b"new-line\n", 0);

        let out = read_log_from(dir.path(), "gpm").await;
        let old_pos = out.find("old-line").expect("old segment present");
        let new_pos = out.find("new-line").expect("new segment present");
        assert!(
            old_pos < new_pos,
            "oldest segment must come first (mtime sort), got: {out}"
        );
    }

    #[tokio::test]
    async fn read_log_includes_bak_and_ignores_unrelated() {
        let dir = tempfile::tempdir().unwrap();
        write_with_mtime(dir.path(), "gpm.log", b"active\n", 0);
        write_with_mtime(dir.path(), "gpm_2026-01-01_00-00-00.log.bak", b"bak\n", 60);
        // An unrelated file in the dir must be ignored.
        write_with_mtime(dir.path(), "README.txt", b"ignore me\n", 0);

        let out = read_log_from(dir.path(), "gpm").await;
        assert!(out.contains("active"), "active present: {out}");
        assert!(out.contains("bak"), ".bak present: {out}");
        assert!(!out.contains("ignore me"), "unrelated file excluded: {out}");
    }

    #[tokio::test]
    async fn read_log_missing_dir_returns_empty() {
        let out = read_log_from(Path::new("/nonexistent/gpm-log-dir"), "gpm").await;
        assert!(out.is_empty(), "missing dir => empty string, not error");
    }

    #[tokio::test]
    async fn read_log_truncates_large_output_at_newline() {
        let dir = tempfile::tempdir().unwrap();
        // 64-byte lines (uniform, each starting with "L") — well over MAX_LOG_BYTES.
        let line: String = "L".repeat(63) + "\n";
        let big: Vec<u8> = line.repeat(6000).into(); // ~375 KiB > 256 KiB
        write_with_mtime(dir.path(), "gpm.log", &big, 0);

        let out = read_log_from(dir.path(), "gpm").await;
        assert!(
            out.len() <= MAX_LOG_BYTES,
            "output must be capped at MAX_LOG_BYTES, got {}",
            out.len()
        );
        // Tail-truncation snaps to a newline, so we never start mid-line: every
        // line starts with "L".
        for l in out.lines() {
            assert!(l.starts_with('L'), "no partial first line: {l}");
        }
    }

    #[tokio::test]
    async fn clear_log_truncates_active_and_removes_rotated() {
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join("gpm.log");
        let rotated = dir.path().join("gpm_2026-01-01_00-00-00.log");
        let bak = dir.path().join("gpm_2026-01-01_00-00-00.log.bak");
        std::fs::write(&active, b"active-data\n").unwrap();
        std::fs::write(&rotated, b"rotated\n").unwrap();
        std::fs::write(&bak, b"bak\n").unwrap();

        clear_log_in(dir.path(), "gpm").await.unwrap();

        // Active exists, truncated to empty (NOT deleted — the logger's handle
        // stays valid).
        assert!(active.exists(), "active file must remain (handle alive)");
        assert_eq!(std::fs::read_to_string(&active).unwrap(), "");
        // Rotated + bak removed.
        assert!(!rotated.exists(), "rotated file removed");
        assert!(!bak.exists(), ".bak removed");
    }

    #[tokio::test]
    async fn clear_log_tolerates_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        // No active file yet (nothing logged) — clear is a harmless no-op create.
        clear_log_in(dir.path(), "gpm").await.unwrap();
        assert!(dir.path().join("gpm.log").exists());
    }
}
