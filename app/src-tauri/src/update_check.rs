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
//! reads it via [`get_update_status`] to light two red dots (the Settings About
//! entry and the About-page version); tapping the version opens a dialog that
//! owns the download link, the manual check ([`check_update_now`]), and the
//! toggle. Fail-closed applies to the cold-start probe only — any error means
//! no dot and no user-facing error; the manual check fails loud instead,
//! surfacing an error rather than quietly claiming "up to date".
//!
//! The probe result lives in `app.json`, not a separate cache file: the store's
//! `write_mu` already serializes every config write, so the probe and an
//! acknowledgment can't race (a separate cache file would recreate a race the
//! config store already solves, and would need its own mutex).
//!
//! Platform-agnostic: detection + a link out is identical on Android and
//! desktop (the app downloads/verifies nothing, so no signing is involved).

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::AppState;
use crate::app_config::{AppConfigStore, now_unix};

/// GitHub's "latest release" URL. Returns a 3xx redirect to
/// `/yzx9/gpm/releases/tag/vX.Y.Z`; the latest version is the last path segment.
const RELEASES_LATEST_URL: &str = "https://github.com/yzx9/gpm/releases/latest";

/// The only redirect target we accept: GitHub's latest-release redirect always
/// lands on this same-repo tag URL. Validating the full prefix (not just the
/// last segment) means a hostile network path that swaps the `Location` header
/// (e.g. an enterprise TLS-terminating proxy) can't inject an arbitrary
/// semver-looking tag into `app.json`, the log, or the UI — the probe just
/// fails closed instead.
const RELEASES_TAG_PREFIX: &str = "https://github.com/yzx9/gpm/releases/tag/";

/// Re-probe at most this often; the cold-start probe runs only when the stored
/// `release_probe_at` is older than this (or absent). Bounds the default-on
/// phone-home to ≤1/day.
const STALENESS_SECS: u64 = 24 * 60 * 60;

/// Bound a hung connection (the probe is fire-and-forget; this only avoids a
/// lingering spawned task).
const PROBE_TIMEOUT_SECS: u64 = 15;

/// The IPC view of the probe state. `available` lights the About-page dot and
/// drives the version dialog's download view; `unacknowledged` additionally
/// lights the Settings-entry dot.
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

/// The status for a known latest tag against the running version. Shared by the
/// cached read ([`get_update_status`]) and the manual check
/// ([`check_update_now`]) so their availability semantics can't drift.
fn status_for(latest: &str, seen: Option<&str>, current: &str) -> UpdateStatus {
    let available = is_newer(Some(latest), current);
    let unacknowledged = available && seen != Some(latest);
    UpdateStatus {
        available,
        unacknowledged,
        latest_version: Some(latest.to_string()),
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

/// Extract and validate the release tag from a redirect `Location` — the pure
/// core of [`probe_latest`], split out so every failure reason is unit-tested.
/// `Err` reasons are static strings: they are logged and can surface in IPC
/// errors, so they never embed the external header content (log/diagnostic
/// injection, and a future toast must not render attacker-influenced text).
fn tag_from_location(loc: &str) -> Result<String, String> {
    let tag = loc
        .strip_prefix(RELEASES_TAG_PREFIX)
        .ok_or("unexpected redirect target")?
        .rsplit('/')
        .next()
        .filter(|t| !t.is_empty())
        .ok_or("empty tag in redirect target")?;
    // Validate it parses before storing, so a malformed tag can't light the dot.
    parse_version(tag)
        .map(|_| tag.to_string())
        .ok_or_else(|| "unparseable tag in redirect target".to_string())
}

/// Probe GitHub's "latest release" redirect and return the version tag in the
/// redirect target (`vX.Y.Z`). `Err` carries a short sanitized reason (static
/// strings, or the public URL/status) for any transport failure, non-redirect,
/// or off-target/unparseable result — one no-follow round-trip, never fetching
/// the body. Callers decide how to fail: the cold-start probe swallows the
/// error (fail-closed), the manual check surfaces it (the user asked, so "up
/// to date" must never be claimed off a probe that didn't complete).
async fn probe_latest(client: &reqwest::Client) -> Result<String, String> {
    let resp = client
        .get(RELEASES_LATEST_URL)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_redirection() {
        return Err(format!("unexpected status {}", resp.status()));
    }
    let loc = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or("missing Location header".to_string())?
        .to_str()
        .map_err(|_| "unparseable Location header".to_string())?;
    tag_from_location(loc)
}

/// The probe client, built once and shared by the cold-start and manual checks:
/// `ClientBuilder::build` synchronously initializes the TLS backend, and a
/// per-call client also discards its connection pool after one request.
/// Redirects are never followed — the tag lives in the redirect itself.
fn probe_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| format!("client build failed: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// One probe attempt under [`PROBE_TIMEOUT_SECS`] using the shared client.
/// `Err` carries the sanitized reason (transport failure, timeout, or a
/// response that wasn't the expected redirect).
async fn probe_once() -> Result<String, String> {
    let client = probe_client()?;
    match tokio::time::timeout(
        Duration::from_secs(PROBE_TIMEOUT_SECS),
        probe_latest(client),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err("timed out".to_string()),
    }
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
    let found = match probe_once().await {
        Ok(tag) => {
            log::info!("update-check: latest release is {tag}");
            Some(tag)
        }
        // Probe error / timeout — fail-closed, no dot.
        Err(reason) => {
            log::warn!("update-check: probe failed: {reason}");
            None
        }
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
    Ok(match cfg.latest_release.as_deref() {
        Some(tag) => status_for(tag, cfg.seen_release.as_deref(), env!("CARGO_PKG_VERSION")),
        None => UpdateStatus::quiet(),
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
pub(crate) async fn acknowledge_update(state: State<'_, AppState>) -> Result<(), rustpass::Error> {
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

/// Manual update check (About-page version dialog). Unlike the passive probe:
/// bypasses the ≤1/day throttle (the user asked), ignores `update_check_enabled`
/// (disabling the automatic check disables the phone-home, not the user's own
/// button), and FAILS LOUD — a transport/parse failure returns `Err` so the
/// dialog shows "check failed" instead of silently claiming "up to date".
/// The result is still recorded via [`AppConfigStore::record_update_probe`]
/// (best-effort) so the cached status and the next cold-start probe agree with
/// what the user just saw; a failure records nothing, keeping the prior cache.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn check_update_now(
    state: State<'_, AppState>,
) -> Result<UpdateStatus, rustpass::Error> {
    let tag = probe_once()
        .await
        .map_err(|reason| manual_check_error(&reason))?;
    log::info!("update-check: manual check found {tag}");
    // Best-effort persistence — the returned status is already fresh, and a
    // write failure just means the next cold-start probe re-fetches.
    if let Err(e) = state
        .app_config
        .record_update_probe(Some(tag.clone()))
        .await
    {
        log::warn!("update-check: could not persist manual probe result: {e}");
    }
    let cfg = state.app_config.get();
    Ok(status_for(
        &tag,
        cfg.seen_release.as_deref(),
        env!("CARGO_PKG_VERSION"),
    ))
}

/// The `NetworkError` wrapper for a failed manual check. The rejection — not
/// the message — is what the frontend keys on (its failed view renders its own
/// localized copy), so the message is diagnostics/logging only; it stays a
/// sanitized static reason that never embeds external header content.
fn manual_check_error(reason: &str) -> rustpass::Error {
    rustpass::Error::new(
        rustpass::ErrorCode::NetworkError,
        format!("Update check failed: {reason}"),
    )
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

    #[test]
    fn status_for_newer_release_is_available() {
        let s = status_for("v0.19.0", None, CURRENT);
        assert!(s.available);
        assert_eq!(s.latest_version.as_deref(), Some("v0.19.0"));
    }

    #[test]
    fn status_for_equal_release_is_not_available() {
        let s = status_for("v0.18.1", Some("v0.18.1"), CURRENT);
        assert!(!s.available);
        assert!(!s.unacknowledged); // unacknowledged only tracks available releases
    }

    #[test]
    fn status_for_unacknowledged_requires_unseen_newer_release() {
        // Newer + never seen ⇒ both dots eligible.
        assert!(status_for("v0.19.0", None, CURRENT).unacknowledged);
        // Newer + already acknowledged ⇒ available, not unacknowledged.
        let seen = status_for("v0.19.0", Some("v0.19.0"), CURRENT);
        assert!(seen.available);
        assert!(!seen.unacknowledged);
    }

    #[test]
    fn status_for_pre_release_is_not_available() {
        let s = status_for("v1.0.0-rc1", None, CURRENT);
        assert!(!s.available);
        assert!(!s.unacknowledged);
    }

    #[test]
    fn tag_from_location_accepts_the_github_tag_redirect() {
        assert_eq!(
            tag_from_location("https://github.com/yzx9/gpm/releases/tag/v0.19.0").unwrap(),
            "v0.19.0"
        );
    }

    #[test]
    fn tag_from_location_rejects_off_target_redirects() {
        // A swapped Location (host or repo path) must not yield a storable tag.
        assert_eq!(
            tag_from_location("https://evil.example.com/releases/tag/v9.9.9").unwrap_err(),
            "unexpected redirect target"
        );
        assert_eq!(
            tag_from_location("https://github.com/other/repo/releases/tag/v9.9.9").unwrap_err(),
            "unexpected redirect target"
        );
        assert_eq!(
            tag_from_location("https://github.com/yzx9/gpm/releases/tag/").unwrap_err(),
            "empty tag in redirect target"
        );
        assert_eq!(
            tag_from_location("https://github.com/yzx9/gpm/releases/tag/not-semver").unwrap_err(),
            "unparseable tag in redirect target"
        );
    }

    #[test]
    fn tag_from_location_never_echoes_the_header_content() {
        // The reasons are static strings — an arbitrary Location payload can't
        // reach the log or an IPC error message.
        let err =
            tag_from_location("https://github.com/yzx9/gpm/releases/tag/<script>alert(1)</script>")
                .unwrap_err();
        assert_eq!(err, "unparseable tag in redirect target");
        assert!(!err.contains("<script>"));
    }

    #[test]
    fn update_status_serializes_to_the_pinned_wire_shape() {
        // The frontend's hand-written `UpdateStatus` interface
        // (app/src/api/system.ts) mirrors these exact field names — the api
        // wrappers are bare passthroughs, so this is the only guard against
        // serde drift silently nulling fields in the WebView (cf. the
        // runtime_platform wire pin in app_config.rs).
        let fresh = serde_json::to_value(status_for("v0.19.0", None, CURRENT)).unwrap();
        assert_eq!(
            fresh,
            serde_json::json!({
                "available": true,
                "unacknowledged": true,
                "latest_version": "v0.19.0",
            })
        );
        let quiet = serde_json::to_value(UpdateStatus::quiet()).unwrap();
        assert_eq!(
            quiet,
            serde_json::json!({
                "available": false,
                "unacknowledged": false,
                "latest_version": null,
            })
        );
    }
}
