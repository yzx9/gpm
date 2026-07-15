# In-app Diagnostics Logging

**Priority:** P2
**Status:** Accepted
**Phase:** Next

## What

Give gpm complete, structured logging: every operational failure (git clone/pull/push, decrypt, create/edit, sync, setup, app-lock) leaves a persisted record the user can browse in-app and export or share for bug reports, with a hard guarantee that no secret ever reaches a log line. The rollout is phased — phase 1 wires the logging pipeline (record to a rotated file and Android logcat) and is complete; phase 2 adds the in-app viewer, export/share, runtime level control, and the safety hardening that lets broad instrumentation land. This RFC records the design for the complete feature and where it stands today.

## Why

Today gpm has no observability. When a push is rejected, a decrypt fails, or a setup step errors, there is no record the user can attach to a bug report; the only signal is a sanitized error code flashed briefly in the UI. The pre-existing debug prints go to stderr, are unstructured, are not file-persisted, and are invisible on Android. Logging unlocks user-driven diagnostics (a shareable log for support) and makes future instrumentation and incident postmortems possible.

The hard requirement, which shapes the whole design, is that a password manager's logs must never contain a secret. That constraint interacts with the export feature in a non-obvious way (see Context), and the decision to keep logs unencrypted is a deliberate threat-model call worth recording.

## Context

**Stack.** Use the official Tauri logging plugin (built on the `log` facade). It provides, off the shelf, size-based file rotation, automatic routing of stdout output to Android logcat (there is no separate logcat target in the plugin), and a frontend logging bridge so Vue code can share the same backend. The `log` facade is runtime-agnostic, so log calls are safe inside the async runtime, inside work offloaded to blocking threads, and inside native-library callbacks — all patterns gpm uses heavily. `tracing` was considered and rejected (see Alternatives).

**Threat model — plaintext logs, "never log a secret."** Logs are stored unencrypted. This is deliberate. Secrets are excluded by construction, not by encryption: only entry names and operation outcomes (plus already-sanitized error codes) are logged; credential-bearing configuration types must redact their secret fields before any debug formatting; and decrypted content, passphrases, identity material, and the at-rest master key are never passed to the logger. Given that, there is nothing in a log worth protecting with encryption. Furthermore, an attacker who can read the on-device log file already has filesystem access to the repository directory, so entry paths — the only "sensitive-ish" data that remains — give them nothing they did not already have. One caveat: that premise holds for the log _file_ but not for the _logcat_ channel, which an attacker with pre-authorized USB debugging can read without repository filesystem access; entry-name metadata (which entries are copied, and when) is therefore visible to that narrower attacker class. The exposure is metadata only (never secret content), requires prior debugging authorization, and matches how any logging app behaves, so it is folded into the phase-2 redaction work rather than treated as a reason to encrypt. Encrypting logs would add a key lifecycle (necessarily tied to the same master key that protects the real secrets) for no meaningful gain, and would couple logging to the unlock lifecycle, breaking the very use case it serves (reading logs to diagnose an unlock or setup failure). The governing rule is therefore "never log a secret," which is stricter than "log it encrypted."

**Export changes the threat surface — the load-bearing phase 2 decision.** The reasoning above holds for the log file at rest on the device. It does _not_ hold for an exported log. The export-and-share flow exists precisely to push a log off the device (to email, a messenger, a support channel), so when the user taps export, entry names leave with it to a third party the user chooses. At that point the "attacker already has filesystem access" premise is gone. Phase 2 must therefore treat the exported log as a distinct, more-sensitive artifact: surface a clear pre-export warning that entry names leave the device, and/or offer a redacted export that strips entry identifiers. This is the one place where plaintext logging needs an explicit user gate.

**Phasing.** Phase 1 proves the pipeline (file + logcat, fixed level, a couple of example logs) and migrates the legacy debug prints onto the real logger; it ships no UI and instruments almost nothing, and is complete. Phase 2 builds on the proven pipeline: a Settings sub-page that reads and displays the current log with a level selector and a clear control; export/share (native save dialog on desktop, system share sheet on Android via a small Android plugin that reuses the app's existing file-provider, with the export snapshot written to the app cache so no new file-provider paths are needed); broad but disciplined instrumentation of the key operations (entry name + outcome, never content); runtime level control persisted in the app-shell preferences so it survives restart and is readable before unlock; and light frontend logging through the plugin's bridge plus a global error handler.

**Safety ordering.** Broad instrumentation must not land before the credential-bearing types that currently leak secrets through their debug representation are made to redact. That hardening is therefore the first step of phase 2, ahead of the viewer, and is valuable on its own: it closes a latent leak that exists today, independent of logging.

## Alternatives considered

- **`tracing` instead of the Tauri logging plugin.** Rejected. `tracing` would require hand-building size-based file rotation (its rolling appender rotates by time only), the Android logcat bridge, and the frontend bridge the viewer depends on. Its contextual-span power is low-value for a mostly-serial mobile password manager and is eroded where it would matter most (work offloaded to blocking threads does not carry spans without manual propagation). The plugin gives the needed backends for free.

- **Encrypting the log file at rest.** Rejected, as above: nothing worth protecting reaches the log by construction, and encryption tied to the master key would couple diagnostics to unlock. "Never log a secret" is the rule.

- **Encrypting only the exported log.** Considered and rejected as the primary defense; the export problem is better solved by a user warning plus an optional redacted export, because the recipient (support) needs to read the log, and a shareable key defeats the purpose.

- **Live log tail (event stream into the viewer).** Deferred beyond phase 2. Refresh-on-open is sufficient for a diagnostics viewer and avoids a persistent event channel; the plugin's webview target makes tail additive later.

- **A single large change.** Rejected in favor of phasing. Splitting the pipeline (phase 1) from the UI and instrumentation (phase 2) lets the foundation be verified before anything depends on it, and isolates the higher-risk Android share work.

## Effort

Phase 1: small, shipped (~1 human-day / ~15 min CC).
Phase 2: medium (~2–3 human-days / ~45 min CC) — viewer, export/share plugin, instrumentation, and the redacting safety hardening.

## Depends on / Supersedes

None. Aligns with the existing at-rest encryption and sanitized-error threat model documented in the security model, and extends it with the "exported log leaves the device" consideration.
