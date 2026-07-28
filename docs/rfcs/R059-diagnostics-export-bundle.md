# Diagnostics Export Bundle

**Priority:** P2
**Status:** Accepted
**Phase:** Next

## What

Give gpm a one-tap "export diagnostics" action that bundles everything useful for a bug report into a single zip and pushes it off the device — the system share sheet on Android, a native save dialog on desktop. The bundle combines the full rotated log, the non-secret user settings, a redacted view of the repository configuration, and a consolidated set of device/system facts (the standard Android hardware build fields plus the capability probes the app already makes). A pre-export warning is mandatory: the bundle leaves the device, so this is the one place the plaintext-logging threat model requires an explicit user gate. This is the export phase of the in-app diagnostics logging work, broadened from "export the log" to a full diagnostics bundle; it closes that work out.

## Why

Today a user reporting a bug can attach only what they can see — a sanitized error code flashed briefly in the UI. The in-app log viewer added a persisted record, and broad instrumentation made that record meaningful, but there is still no way to get the log, or any context about the device and configuration it was produced on, off the device and into a support channel. Sync failures, auth errors, authenticity mismatches, and startup crashes are the common support cases, and diagnosing them needs more than the log alone: which Android version and device, whether biometrics and the keystore are available, what sync and authenticity settings are in effect, what remote host is configured.

The hard requirement, inherited from that logging work, is that nothing secret may ever leave the device in the bundle. That constraint is load-bearing here because the bundle deliberately widens that shipped work — it adds configuration and device data alongside the log — and ships the result to a third party the user chooses. The redaction design and the mandatory user gate below both follow from that.

## Context

**Bundle contents.** Five entries:

- the **full rotated log set**, untruncated — the viewer tails to a window for cheap rendering; the export carries every rotation segment so history is preserved;
- the **non-secret user settings** — the plaintext app-shell preferences (language, theme, auto-lock and clear timers, autosync, the verbose-logging flag, screen-capture mode), rendered verbatim, which is safe because that file is forced plaintext on disk and holds nothing confidential;
- a **redacted view of the repository configuration** — the remote host (credentials stripped), the commit author identity, the authenticity mode and trusted-key fingerprints, the chosen storage and crypto backends, and the _presence_ (never value) of any credential. This reuses the redacted form that already exists for exactly this reason; the raw serialized config is never used, because raw serialization would emit the personal access token, the SSH key, and the key passphrase in the clear;
- a **system/device info summary** that consolidates the capability probes the app already makes (biometric and keystore availability, notification-permission grant, screen-capture support, safe-area insets, system locale) with the standard Android build fields the app does not yet read (manufacturer, model, brand, SDK level and release, supported ABIs, WebView user-agent, package version, display metrics). The app version is a build fact and lives here. Desktop gets a minimal equivalent (OS, architecture, app version), since desktop is a development surface, not the target;
- a short **manifest** naming the contents, the generation timestamp, and the privacy note.

**Threat model — the bundle is an exported artifact, not an at-rest one.** The on-device log is plaintext by construction (see SECURITY.md § Diagnostics logging): an attacker who can read it already has filesystem access to the repository, so the entry-name metadata it contains gives them nothing new. That argument holds on-device and fails the moment the bundle is shared. The export exists to push data off the device to a recipient the user picks, so the bundle must be treated as a more-sensitive artifact than the log file at rest. It carries: entry names and timestamps (from the log), the repository remote host and commit identity, trusted-key fingerprints, the WebView user-agent (fingerprint-shaped but diagnostic only), and the full non-secret preference set. None of that is secret content, but it is metadata the user may not want to hand to an arbitrary party.

Two consequences. First, a **pre-export confirmation** is mandatory and is the single user gate: it names what leaves the device and warns against public sharing, before any share sheet or save dialog opens. Second, a **redacted-export** option (a second mode that strips entry identifiers from the log) is deferred — the warning is sufficient for a first cut, and redacted export is purely additive later.

**Redaction is by construction, not by filtering.** The bundle is assembled in the backend from sources that are already safe at their origin: the log (never contains secrets, by construction), the plaintext preference file (non-secret by design), and the repository config rendered through its existing redacted form (credentials and URL userinfo stripped before the text exists). No secret is ever read into the bundling path, so there is no filtering step that could miss one. Only the finished zip bytes leave the backend; the webview never sees the bundle's contents, and the native share path receives only the staged file.

**Mechanism.** The backend assembles and zips the bundle in the app cache directory, which the existing file provider already exposes for sharing — so no manifest or provider-path changes are needed. Sharing itself goes through a new backend-only local plugin that follows the same shape as the existing native plugins: on Android it hands the staged file to the system share sheet through the existing provider and a content URI; on desktop it opens a native save dialog and copies the staged file to the chosen path. A new plugin (rather than extending an existing one) matches the one-concern-per-plugin pattern the other native plugins follow. The zip format comes from the standard Rust zip library; the bundle is small (a few megabytes of logs at most), so there is no need for streaming or special memory discipline beyond ordinary bounds.

**Where it lives.** The export action joins the existing Settings → Logs screen, beside the refresh and clear actions. The logs screen is the natural home: it is where a user already goes to look at diagnostics, and the export is "get these diagnostics off the device."

**Phasing.** One RFC, two ordered steps so the pure-backend parts are unit-testable before the native integration lands: (1) the bundle assembler — log collection, system-info gathering, redacted settings, and the zip builder, all pure and testable with a temporary directory; (2) the share plugin, the UI action, and the pre-export warning. The whole thing closes out the export phase.

## Alternatives considered

- **Export the log only (the original export-phase scope).** Rejected: a log without device or configuration context is much less useful for the common support cases (sync, auth, authenticity, startup). The marginal redaction cost of adding the redacted config and the system-info probes is low, and the user gate already covers the wider payload.

- **Encrypt the bundle.** Rejected, for the same reason the on-device log is not encrypted (see SECURITY.md): the recipient (support) must read it, a shareable key defeats the purpose, and nothing in the bundle is secret by construction. The defense is the user gate plus redaction, not encryption.

- **Omit the repository configuration from the bundle.** Considered; rejected. The remote host, commit identity, authenticity mode, and backend choices are exactly what sync/auth/authenticity diagnoses need, and the redacted form is already safety-reviewed. Omitting them would cripple the most common support case to avoid disclosing data (a host name, a commit email) the user is already willing to attach to a bug report.

- **Filter secrets out of a raw config dump.** Rejected in favor of redaction-by-construction. A filter is a denial surface: one missed field leaks a credential. Rendering the config through its existing redacted form means the plaintext credential text is never produced, so there is nothing to miss.

- **Stream the bundle bytes to the share sheet through the IPC bridge.** Considered (mirrors how the file _picker_ round-trips bytes). Rejected: the backend already stages the zip in the cache directory the file provider exposes, so the share plugin needs only the file name, not megabytes of file bytes crossing the native boundary.

- **Extend an existing native plugin instead of adding a new one.** Rejected: the existing plugins each own one distinct concern (safe-area, biometric, secure keystore, file picking, screen capture, clipboard notification). A dedicated share plugin keeps that boundary clean and is trivial to add following the established pattern.

## Effort

Medium (~2 human-days / ~45 min CC) — bundle assembler + zip, the new backend-only share plugin (desktop save + Android share via the existing file provider), the system-info probe, and the UI action with its pre-export warning.

## Depends on / Supersedes

Closes out the in-app diagnostics logging work: its shipped phases (pipeline, viewer, instrumentation) live in code, and the plaintext-logs threat model in SECURITY.md (§ Diagnostics logging). Builds on **R055** (the verbose toggle), whose persisted-deadline and notification work the export's "capture one repro" flow assumes. Aligns with the at-rest encryption and sanitized-error threat model, extending the "exported log leaves the device" consideration to the wider bundle.
