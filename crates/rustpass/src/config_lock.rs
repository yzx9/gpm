// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cross-process write lock for `repo.json` (R097).
//!
//! `repo.json` is written by read-modify-write from many call sites
//! (settings mutations, the identity save's crypto persist, setup full
//! writes, content migrations). An atomic-rename save makes a single write
//! tear-free, but two interleaved RMWs silently drop one side's field
//! change. Every writer therefore funnels through this lock with the WHOLE
//! read-modify-write inside the critical section: acquire → load → apply
//! own-field change → save → release.
//!
//! Sibling to [`crate::repo_lock`] but deliberately different in strategy:
//! the repo lock fails fast (~100 ms, `REPO_BUSY`) because a best-effort
//! background sync should yield to the foreground; config writes are tiny
//! must-succeed user actions, so this lock WAITS up to a deadline and only
//! then reports [`ErrorCode::ConfigBusy`]. That difference is also why
//! acquisition is async — `try_lock_exclusive` is non-blocking, and the
//! retry sleeps on `tokio::time::sleep`, never `thread::sleep`: the
//! headless worker runs a single-threaded runtime where a sync busy-wait
//! would starve its own deadline timers.
//!
//! flock mutual exclusion is per open file description, so the same lock
//! serializes two concurrent handles inside one process (the app's own
//! concurrent commands), the foreground/worker split, and two desktop
//! instances sharing a config directory.
//!
//! **Not reentrant** — acquiring twice on one task deadlocks into
//! `CONFIG_BUSY` (the second acquire times out). Every public operation
//! acquires exactly once, at the lowest sink.
//!
//! **Lock order:** `repo_lock` → `config_lock`, never reversed. The
//! migration paths already take them in this order.

use std::io;
use std::path::Path;
use std::time::Duration;

use fs2::FileExt;

use crate::error::{Error, ErrorCode};

/// The lockfile name, sibling to `repo.json` in the config dir.
pub const CONFIG_LOCK_FILE: &str = "gpm_config.lock";
/// Retry interval while waiting for a contended lock.
const CONFIG_LOCK_RETRY_SLEEP_MS: u64 = 25;
/// Total wait budget before giving up with [`ErrorCode::ConfigBusy`]. The
/// critical section is a file read + AEAD seal + rename (microseconds), so
/// anything near this bound means a stuck holder, not ordinary contention.
const CONFIG_LOCK_DEADLINE: Duration = Duration::from_secs(5);

/// A held cross-process `repo.json` write lock. Releases on drop; also
/// auto-released by the OS if the holding process dies.
#[derive(Debug)]
pub struct ConfigLock {
    /// The locked file, kept open for the lock's lifetime. Closed (releasing
    /// the flock) on drop.
    file: Option<std::fs::File>,
}

impl ConfigLock {
    /// Acquire the exclusive `repo.json` write lock on
    /// `<config_dir>/gpm_config.lock`, waiting up to [`CONFIG_LOCK_DEADLINE`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ConfigBusy`] if the lock is still held at the
    /// deadline, or [`ErrorCode::IoError`] on a file-open / lock failure.
    pub async fn acquire(config_dir: &Path) -> Result<Self, Error> {
        // The config dir normally exists by the time any Store is constructed;
        // create it best-effort so a fresh process can open the lockfile.
        let _ = std::fs::create_dir_all(config_dir);
        let path = config_dir.join(CONFIG_LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| Error::new(ErrorCode::IoError, format!("config lock open: {e}")))?;
        // Non-blocking try + async sleep retry. Each iteration is two fast
        // syscalls; the wait parks the task, not the thread.
        let deadline = tokio::time::Instant::now() + CONFIG_LOCK_DEADLINE;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file: Some(file) }),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::new(
                            ErrorCode::ConfigBusy,
                            "another process is writing the config",
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(CONFIG_LOCK_RETRY_SLEEP_MS)).await;
                }
                Err(e) => return Err(Error::new(ErrorCode::IoError, format!("config lock: {e}"))),
            }
        }
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        // Explicit unlock, then drop the fd. On Unix closing the fd releases
        // the flock anyway; the explicit unlock is belt-and-suspenders and
        // matches fs2's recommended pattern (mirrors RepoLock).
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uncontended acquire succeeds and releases on drop.
    #[tokio::test]
    async fn acquire_uncontended_then_release_on_drop() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        {
            let _lock = ConfigLock::acquire(dir.path()).await.expect("acquire");
            // Held: the lockfile exists (the flock itself is invisible).
            assert!(dir.path().join(CONFIG_LOCK_FILE).exists());
        }
        // Dropped: a second acquire succeeds immediately.
        let _second = ConfigLock::acquire(dir.path())
            .await
            .expect("second acquire");
    }

    /// A second acquire while the first is held contends — here run under
    /// paused time so the deadline lapses instantly and we observe
    /// `CONFIG_BUSY` without sleeping for real.
    #[tokio::test(start_paused = true)]
    async fn second_acquire_while_held_times_out_busy() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let _first = ConfigLock::acquire(dir.path())
            .await
            .expect("first acquire");
        let err = ConfigLock::acquire(dir.path())
            .await
            .expect_err("second must contend");
        assert_eq!(err.code, "CONFIG_BUSY");
        // Same task, sequential awaits: the contention is cross-fd (a second
        // open file description), proving the non-reentrancy contract too.
    }

    /// Two concurrent tasks: the loser waits, then acquires after the winner
    /// drops — the exact serialization the RMW writers rely on.
    #[tokio::test(start_paused = true)]
    async fn waiter_acquires_after_holder_releases() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().to_path_buf();
        let holder = ConfigLock::acquire(&path).await.expect("holder");
        let waiter = tokio::spawn(async move { ConfigLock::acquire(&path).await });
        // Let the waiter park on its retry sleep, then release.
        tokio::time::sleep(Duration::from_millis(60)).await;
        drop(holder);
        waiter
            .await
            .expect("task join")
            .expect("waiter acquires after release");
    }
}
