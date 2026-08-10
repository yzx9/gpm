// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Passive update-availability check (RFC R090).
//!
//! Detects — does not install — when a newer stable release exists. On cold
//! start [`run_once`] probes GitHub's "latest release" redirect (the version
//! comes from the redirect `Location`, so no API token, no 60/hr rate limit, no
//! JSON), compares it against the build-baked version, and stores the result in
//! the sealed `app.json` next to the `update_check_enabled` toggle. The frontend
//! reads it via [`get_update_status`] and lights two red dots (Settings About
//! entry + About page) plus an "Update" link that opens the release page.
//! Fail-closed: any error ⇒ no dot, no user-facing error.
//!
//! The probe result lives in `app.json`, not a separate cache file: the store's
//! `write_mu` already serializes every config write, so the probe and an
//! acknowledgment can't race (a separate cache file would recreate a race the
//! config store already solves, and would need its own mutex). See
//! `docs/rfcs/R090-update-check.md`.
//!
//! Platform-agnostic: detection + a link out is identical on Android and
//! desktop (the app downloads/verifies nothing, so no signing is involved).

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::AppState;
use crate::app_config::{AppConfigStore, now_unix};

/// GitHub's "latest release" URL. Returns a 3xx redirect to
/// `/yzx9/gpm/releases/tag/vX.Y.Z`; the latest version is the last path segment.
const RELEASES_LATEST_URL: &str = "https://github.com/yzx9/gpm/releases/latest";

/// Re-probe at most this often; the cold-start probe runs only when the stored
/// `release_probe_at` is older than this (or absent). Bounds the default-on
/// phone-home to ≤1/day.
const STALENESS_SECS: u64 = 24 * 60 * 60;

/// Bound a hung connection (the probe is fire-and-forget; this only avoids a
/// lingering spawned task).
const PROBE_TIMEOUT_SECS: u64 = 15;

/// The IPC view of the probe state. `available` lights the About-page dot + the
/// Update link; `unacknowledged` additionally lights the Settings-entry dot.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct UpdateStatus {
    /// A newer stable release exists than the built-in version.
    available: bool,
    /// A newer release exists that the user has not yet acknowledged (lights the
    /// Settings-entry dot; the About-page dot ignores the ack).
    unacknowledged: bool,
    /// The latest release tag seen (e.g. `v0.19.0`), or `None` if never probed.
    latest_version: Option<String>,
}

impl UpdateStatus {
    /// All-false / no-dot view — the fail-closed result when the check is off or
    /// no probe has succeeded yet.
    const fn quiet() -> Self {
        Self {
            available: false,
            unacknowledged: false,
            latest_version: None,
        }
    }
}

/// Strip a leading `v`/`V` and parse a semantic version. `None` on a non-semver
/// string; the caller fail-closes (no dot).
fn parse_version(s: &str) -> Option<semver::Version> {
    semver::Version::parse(s.trim_start_matches(['v', 'V'])).ok()
}

/// Whether `latest_known` is a stable release newer than `current`. Pre-releases
/// (`-rc`/`-beta`) do NOT count (only stable releases light the dot, RFC R090).
/// Any parse failure fail-closes to `false`.
fn is_newer(latest_known: Option<&str>, current: &str) -> bool {
    let Some(latest) = latest_known.and_then(parse_version) else {
        return false;
    };
    if !latest.pre.is_empty() {
        return false;
    }
    parse_version(current).is_some_and(|c| latest > c)
}

/// Probe GitHub's "latest release" redirect and return the version tag in the
/// redirect target (`vX.Y.Z`), or `None` on any non-redirect / missing /
/// unparseable result. One no-follow round-trip; never fetches the body.
async fn probe_latest(client: &reqwest::Client) -> Option<String> {
    let resp = client.get(RELEASES_LATEST_URL).send().await.ok()?;
    if !resp.status().is_redirection() {
        log::warn!("update-check: unexpected status {}", resp.status());
        return None;
    }
    let loc = resp
        .headers()
        .get(reqwest::header::LOCATION)?
        .to_str()
        .ok()?;
    // The redirect target ends in `/releases/tag/vX.Y.Z`; take the last segment.
    let tag = loc.rsplit('/').next().filter(|t| !t.is_empty())?;
    // Validate it parses before storing, so a malformed tag can't light the dot.
    parse_version(tag).map(|_| tag.to_string())
}

/// The cold-start probe (RFC R090). Throttled to ≤1/day via the stored
/// `release_probe_at`; on any outcome (success / no-release / failure / timeout)
/// the probe time is refreshed so the throttle holds. The result is written
/// back through [`AppConfigStore::record_update_probe`] — a `write_mu`-guarded
/// RMW — so it can't race a concurrent ack or toggle. Fail-closed: errors only
/// warn. Spawned fire-and-forget from `.setup()` (gated on the user's
/// `update_check_enabled` pref), so it never blocks launch.
pub(crate) async fn run_once(app_config: Arc<AppConfigStore>) {
    // Throttle: skip if probed recently. A snapshot read is enough — it only
    // gates whether to do the network round-trip.
    if let Some(last) = app_config.get().release_probe_at
        && last.saturating_add(STALENESS_SECS) > now_unix()
    {
        return; // probed recently — ≤1/day.
    }
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("update-check: client build failed: {e}");
            return;
        }
    };
    let probed = tokio::time::timeout(
        Duration::from_secs(PROBE_TIMEOUT_SECS),
        probe_latest(&client),
    )
    .await;
    let found = match probed {
        Ok(Some(tag)) => {
            log::info!("update-check: latest release is {tag}");
            Some(tag)
        }
        // Non-redirect / unparseable / timeout — fail-closed, no dot.
        Ok(None) | Err(_) => None,
    };
    // Record the outcome (bumps `release_probe_at` either way; stores the tag
    // only on success). Best-effort: a write failure means we retry next launch
    // — fail-closed, no user-facing error.
    if let Err(e) = app_config.record_update_probe(found).await {
        log::warn!("update-check: could not persist probe result: {e}");
    }
}

/// Read the stored probe state for the frontend. No network — the cold-start
/// probe writes `app.json` (≤1/day); this reads the in-memory cache. Fail-closed
/// (returns a quiet status when the check is off or no probe has run yet).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_update_status(
    state: State<'_, AppState>,
) -> Result<UpdateStatus, rustpass::Error> {
    let cfg = state.app_config.get();
    if !cfg.update_check_enabled {
        return Ok(UpdateStatus::quiet());
    }
    let available = is_newer(cfg.latest_release.as_deref(), env!("CARGO_PKG_VERSION"));
    let unacknowledged =
        available && cfg.seen_release.as_deref() != cfg.latest_release.as_deref();
    Ok(UpdateStatus {
        available,
        unacknowledged,
        latest_version: cfg.latest_release,
    })
}

/// Acknowledge the current latest release — records that the user has opened
/// About for this version, so the Settings-entry dot falls quiet. The About-page
/// dot ignores the ack and stays lit until the user actually updates (RFC R090).
/// Best-effort (fire-and-forget from the About page): a failure is logged and
/// swallowed so the frontend promise never rejects — a missed ack just
/// re-lights the dot next launch.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn acknowledge_update(
    state: State<'_, AppState>,
) -> Result<(), rustpass::Error> {
    if let Err(e) = state.app_config.acknowledge_update().await {
        log::warn!("update-check: acknowledge failed: {e}");
    }
    Ok(())
}

/// Toggle the passive update check on/off (sealed in `app.json` like `autosync`).
/// When off, the cold-start probe is skipped and [`get_update_status`] returns a
/// quiet status. Returns the updated app config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_update_check(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<crate::app_config::AppConfig, rustpass::Error> {
    log::info!("update-check: set-update-check: {enabled}");
    state.app_config.set_update_check(enabled).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `is_newer` against a fixed "current" so the tests survive a version bump.
    const CURRENT: &str = "0.18.1";

    #[test]
    fn newer_stable_lights_the_dot() {
        assert!(is_newer(Some("v0.19.0"), CURRENT));
        assert!(is_newer(Some("0.19.0"), CURRENT)); // leading-v is optional
        assert!(is_newer(Some("v1.0.0"), CURRENT));
    }

    #[test]
    fn equal_or_older_does_not_light() {
        assert!(!is_newer(Some("v0.18.1"), CURRENT)); // equal
        assert!(!is_newer(Some("v0.18.0"), CURRENT)); // older
        assert!(!is_newer(Some("v0.17.99"), CURRENT));
    }

    #[test]
    fn pre_release_does_not_light() {
        // Only stable releases light the dot (RFC R090).
        assert!(!is_newer(Some("v0.19.0-rc1"), CURRENT));
        assert!(!is_newer(Some("v1.0.0-beta.2"), CURRENT));
    }

    #[test]
    fn missing_or_unparseable_fails_closed() {
        assert!(!is_newer(None, CURRENT));
        assert!(!is_newer(Some("not-a-version"), CURRENT));
        assert!(!is_newer(Some("v"), CURRENT));
        assert!(!is_newer(Some(""), CURRENT));
    }

    #[test]
    fn parse_version_strips_leading_v() {
        assert_eq!(
            parse_version("v0.18.1").unwrap(),
            semver::Version::new(0, 18, 1)
        );
        assert_eq!(
            parse_version("0.18.1").unwrap(),
            semver::Version::new(0, 18, 1)
        );
        assert!(parse_version("garbage").is_none());
    }
}
