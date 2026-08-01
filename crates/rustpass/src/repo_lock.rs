// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Cross-process repo lock.
//!
//! `Store::write_mu` only serializes repo mutations within ONE `Store`
//! instance. The foreground app's `Store` and a background Worker's `Store`
//! are separate instances, so `write_mu` can't stop them mutating the same git
//! repo at once. This module adds a `flock`-style advisory lock on a lockfile
//! next to the repo (`<config_dir>/gpm_sync.lock`) so the two instances — and
//! two processes — can't race the git index.
//!
//! `try_lock_exclusive` is non-blocking: on contention the caller receives an
//! [`Error`](crate::Error) with code `REPO_BUSY` and skips (best-effort sync).
//! The lock is held by the OS on the open file description, so it is released
//! on drop AND automatically if the process dies — no stale-lockfile problem
//! (the empty lockfile may persist on disk harmlessly; it carries no data).

use std::fs::{File, OpenOptions};
use std::path::Path;

use fs2::FileExt;

use crate::error::{Error, ErrorCode};

/// The lockfile name, sibling to `repo.json` / `repo/` in the config dir.
pub const REPO_LOCK_FILE: &str = "gpm_sync.lock";
/// Contention retry count. Total wait ≈ retries × sleep ≈ 100 ms.
const REPO_LOCK_RETRIES: u32 = 4;
const REPO_LOCK_RETRY_SLEEP_MS: u64 = 25;

/// A held cross-process repo lock. Releases on drop; also auto-released by the
/// OS if the holding process dies.
#[derive(Debug)]
pub struct RepoLock {
    /// The locked file, kept open for the lock's lifetime. Closed (releasing
    /// the flock) on drop.
    file: Option<File>,
}

impl RepoLock {
    /// Try to acquire an exclusive lock on `<config_dir>/gpm_sync.lock`.
    /// Non-blocking with a brief bounded retry (~`REPO_LOCK_RETRIES` ×
    /// `REPO_LOCK_RETRY_SLEEP_MS` ≈ 100 ms total) so a foreground op that
    /// briefly overlaps a background sync waits instead of erroring — the
    /// background sync is best-effort and should yield to user-initiated work.
    /// Returns `REPO_BUSY` only if the lock is still held after the retries.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::RepoBusy`] if another instance/process holds the
    /// lock past the retry window, or [`ErrorCode::IoError`] on a file-open /
    /// lock failure.
    pub fn try_acquire(config_dir: &Path) -> Result<Self, Error> {
        // The config dir normally exists by the time any Store is constructed;
        // create it best-effort so a fresh worker can open the lockfile.
        let _ = std::fs::create_dir_all(config_dir);
        let path = config_dir.join(REPO_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| Error::new(ErrorCode::IoError, format!("repo lock open: {e}")))?;
        // Bounded retry on contention. Contention is rare
        // (skip-if-foreground prevents most overlap), so a short busy-wait is
        // fine; the total bound (~100 ms) is negligible next to the sync's own
        // 30 s deadline.
        for attempt in 0..=REPO_LOCK_RETRIES {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file: Some(file) }),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if attempt < REPO_LOCK_RETRIES {
                        std::thread::sleep(std::time::Duration::from_millis(
                            REPO_LOCK_RETRY_SLEEP_MS,
                        ));
                    }
                }
                Err(e) => return Err(Error::new(ErrorCode::IoError, format!("repo lock: {e}"))),
            }
        }
        Err(Error::new(
            ErrorCode::RepoBusy,
            "another instance is syncing",
        ))
    }
}

impl Drop for RepoLock {
    fn drop(&mut self) {
        // Explicit unlock, then drop the fd. On Unix closing the fd releases
        // the flock anyway; the explicit unlock is belt-and-suspenders and
        // matches fs2's recommended pattern.
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_while_held_is_busy() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        // First acquires cleanly.
        let first = RepoLock::try_acquire(dir.path()).expect("first acquire");
        // A second acquire while the first is held contends → REPO_BUSY.
        let err = RepoLock::try_acquire(dir.path()).expect_err("second must contend");
        assert_eq!(err.code, "REPO_BUSY");
        // Dropping the first releases the lock; a third acquire succeeds
        // (proves the busy was the held lock, not a stale lockfile).
        drop(first);
        let _third = RepoLock::try_acquire(dir.path()).expect("third after release");
    }
}
