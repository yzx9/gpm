// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Single shared slot for any command that drives the file-save plugin's one
//! pending SAF picker.
//!
//! The Android `FileSavePlugin` tracks a single `pendingTempPath` field, so two
//! overlapping saves — e.g. an attachment export and a diagnostics export running
//! at once — would clobber it: the second `save()` overwrites the path, and the
//! first-launched picker resolves last reading `pendingTempPath == null`. A
//! shared single-flight flag makes the second caller fail fast with
//! [`ErrorCode::RepoBusy`] instead. (Unreachable from the UI in practice — the
//! `WebView` is paused while a SAF picker is on top — but the guard is cheap and
//! makes the defense-in-depth the two commands already claimed actually hold.)

use std::sync::atomic::{AtomicBool, Ordering};

use rustpass::{Error, ErrorCode};

/// The one shared export slot. `false` = free, `true` = a save-driving command
/// is mid-flight.
static FILE_SAVE_BUSY: AtomicBool = AtomicBool::new(false);

/// RAII handle holding [`FILE_SAVE_BUSY`] for the duration of one save-driving
/// command. Acquire at the start; it releases on drop across every return path,
/// `?` short-circuit, and panic.
#[derive(Debug)]
pub(crate) struct FileSaveGuard;

impl FileSaveGuard {
    /// Acquire the single slot, or fail fast with [`ErrorCode::RepoBusy`] (a
    /// benign skip-and-retry signal the UI surfaces as a "busy" toast).
    pub(crate) fn acquire() -> Result<Self, Error> {
        if FILE_SAVE_BUSY
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Ok(FileSaveGuard)
        } else {
            Err(Error::new(
                ErrorCode::RepoBusy,
                "another file-save export is already in progress",
            ))
        }
    }
}

impl Drop for FileSaveGuard {
    fn drop(&mut self) {
        FILE_SAVE_BUSY.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two acquires must not both succeed, and a drop frees the slot again.
    /// Resets the static first because the guard is process-global and tests
    /// run in parallel within the crate — any future guard-touching test must
    /// not run concurrently with this one.
    #[test]
    fn file_save_guard_is_single_flight() {
        FILE_SAVE_BUSY.store(false, Ordering::SeqCst);
        let g1 = FileSaveGuard::acquire();
        assert!(g1.is_ok(), "first acquire succeeds");
        assert_eq!(
            FileSaveGuard::acquire().unwrap_err().code,
            "REPO_BUSY",
            "second acquire is busy"
        );
        drop(g1);
        assert!(
            FileSaveGuard::acquire().is_ok(),
            "re-acquire after drop succeeds"
        );
    }
}
