// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: Apache-2.0

use std::str;
use std::sync::atomic::Ordering;

use zeroize::Zeroizing;

use crate::config::{LockMode, RepoConfig};
use crate::error::Error;

// Impl-split submodule: mod.rs is the shared scope for Store's split impl, so a
// super-glob is the idiomatic import (pedantic flags it; scoped allow).
#[allow(clippy::wildcard_imports)]
use super::*;

impl Store {
    /// Push the app-scoped `autosync` flag into the [`Store`]'s cache — the
    /// value [`autosync_write`](Store::autosync_write) reads. The app shell owns
    /// the authoritative copy in `app.json`; this keeps the cached injection in
    /// sync. Call on startup, on the `set_autosync` command, and after the
    /// config-scope migration (the three mutation points).
    pub fn set_autosync(&self, enabled: bool) {
        self.autosync.store(enabled, Ordering::Relaxed);
    }

    /// The cached app-scoped `autosync` flag (the value [`autosync_write`] reads).
    /// Read accessor for tests/diagnostics — production reads it via
    /// [`autosync_write`](Store::autosync_write).
    #[must_use]
    pub fn autosync(&self) -> bool {
        self.autosync.load(Ordering::Relaxed)
    }

    /// Persist the "unlock the identity together with the app" opt-in. A pure
    /// preference (no key migration), read by the app-unlock path right after the
    /// master key is injected.
    ///
    /// # Errors
    ///
    /// Returns an error if `repo.json` cannot be read or written.
    pub async fn set_unlock_identity_with_app(&self, enabled: bool) -> Result<RepoConfig, Error> {
        let mut rc = self.config.load_repo_config().await?;
        rc.unlock_identity_with_app = enabled;
        self.config.save_repo_config_full(&rc).await?;
        Ok(rc)
    }

    /// Seal the identity passphrase under the **vault key**, for the
    /// identity-auto-unlock opt-in. See [`Config::save_app_identity_pass`].
    ///
    /// # Errors
    ///
    /// Returns an error if the AEAD seal or the write fails.
    pub async fn save_app_identity_pass(&self, passphrase: &str) -> Result<(), Error> {
        self.config
            .save_app_identity_pass(passphrase.as_bytes())
            .await
    }

    /// Load the sealed identity passphrase. See
    /// [`Config::load_app_identity_pass`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::NoIdentity`] if the slot is absent, or an error if
    /// the AEAD unseal fails (e.g. the vault key is wiped).
    pub async fn load_app_identity_pass(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        Ok(Zeroizing::new(self.config.load_app_identity_pass().await?))
    }

    /// Clear the sealed identity passphrase slot. See
    /// [`Config::clear_app_identity_pass`].
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be removed.
    pub async fn clear_app_identity_pass(&self) -> Result<(), Error> {
        self.config.clear_app_identity_pass().await
    }

    /// Seal a serialized behavior config into the app-behavior slot. See
    /// [`Config::save_app_behavior`]. The app shell serializes `BehaviorConfig`
    /// to bytes and passes them here.
    ///
    /// # Errors
    ///
    /// Returns an error if the AEAD seal or the write fails.
    pub async fn save_app_behavior(&self, bytes: &[u8]) -> Result<(), Error> {
        self.config.save_app_behavior(bytes).await
    }

    /// Read + unseal the app-behavior slot. See [`Config::load_app_behavior`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::NoIdentity`] if the slot is absent, or an error if
    /// the AEAD unseal fails.
    pub async fn load_app_behavior(&self) -> Result<Vec<u8>, Error> {
        self.config.load_app_behavior().await
    }

    /// Seal a serialized **merged** app config (display + behavior) into the
    /// app-config slot — the R074 post-collapse single sealed home of all app
    /// prefs. See [`Config::save_app_config`].
    ///
    /// # Errors
    ///
    /// Returns an error if the AEAD seal or the write fails.
    pub async fn save_app_config(&self, bytes: &[u8]) -> Result<(), Error> {
        self.config.save_app_config(bytes).await
    }

    /// Read + unseal the merged app-config slot (dual-AAD: falls back to the
    /// legacy `"app_behavior"` tag during the R074 transition). See
    /// [`Config::load_app_config`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::NoIdentity`] if the slot is absent, or an error if
    /// the AEAD unseal fails.
    pub async fn load_app_config(&self) -> Result<Vec<u8>, Error> {
        self.config.load_app_config().await
    }
}

/// Normalize a view/clipboard auto-clear override: `None` stays (default),
/// `Some(0)` stays (Never), any other `Some(n)` is clamped to
/// [`CLEAR_SECS_MIN`]..[`CLEAR_SECS_MAX`]. Infallible — out-of-range clamps
/// rather than erroring, since the UI sends only preset values. `pub` so the
/// app shell (which owns the app-scoped clear-timer setters post-scope-split)
/// applies the same rule.
#[must_use]
pub fn normalize_clear_secs(secs: Option<u64>) -> Option<u64> {
    match secs {
        None => None,
        Some(0) => Some(0),
        Some(n) => Some(n.clamp(CLEAR_SECS_MIN, CLEAR_SECS_MAX)),
    }
}

/// Clamp a [`LockMode::Idle`] timeout into
/// [`LOCK_IDLE_SECS_MIN`]..[`LOCK_IDLE_SECS_MAX`]; `Immediate` and `Never` pass
/// through. `pub` so the app shell's `lock_mode` setter applies the same rule
/// the old in-`Store` setter did.
#[must_use]
pub fn clamp_lock_mode(mode: LockMode) -> LockMode {
    match mode {
        LockMode::Idle(secs) => LockMode::Idle(secs.clamp(LOCK_IDLE_SECS_MIN, LOCK_IDLE_SECS_MAX)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_lock_mode_clamps_idle_and_passes_others() {
        // Idle secs below the minimum clamp up.
        assert_eq!(
            clamp_lock_mode(LockMode::Idle(1)),
            LockMode::Idle(LOCK_IDLE_SECS_MIN)
        );
        // Idle secs above the maximum clamp down.
        assert_eq!(
            clamp_lock_mode(LockMode::Idle(99_999)),
            LockMode::Idle(LOCK_IDLE_SECS_MAX)
        );
        // Never + Immediate pass through unchanged.
        assert_eq!(clamp_lock_mode(LockMode::Never), LockMode::Never);
        assert_eq!(clamp_lock_mode(LockMode::Immediate), LockMode::Immediate);
    }

    #[test]
    fn normalize_clear_secs_clamps_keeps_never_and_none() {
        // A nonzero value below the minimum clamps up; Never (0) is preserved.
        assert_eq!(normalize_clear_secs(Some(1)), Some(CLEAR_SECS_MIN));
        assert_eq!(
            normalize_clear_secs(Some(0)),
            Some(0),
            "Some(0) (Never) must be kept"
        );
        // None stays None (resolves to the default at read time).
        assert_eq!(normalize_clear_secs(None), None);
        // Values above the maximum clamp down.
        assert_eq!(normalize_clear_secs(Some(999_999)), Some(CLEAR_SECS_MAX));
    }

    #[test]
    fn autosync_cache_default_true_and_set_round_trips() {
        // Proves the injected `autosync` cache is what set_autosync writes and
        // autosync_write reads — the plumbing the app shell pushes into. Default
        // is true (a caller that never seeds gets today's fresh-repo behavior).
        let store = Store::new(std::env::temp_dir(), None);
        assert!(store.autosync(), "default is true");
        store.set_autosync(false);
        assert!(
            !store.autosync(),
            "set_autosync(false) must reach the cache autosync_write reads"
        );
        store.set_autosync(true);
        assert!(store.autosync());
    }
}
