# In-app Diagnostics Logging

**Priority:** P2
**Status:** Accepted
**Phase:** 2 implemented (phases 1–2 shipped); 3–4 planned

## What

Give gpm complete, structured logging: every operational failure (git clone/pull/push, decrypt, create/edit, sync, setup, app-lock) leaves a persisted record the user can browse in-app and export or share for bug reports, with a hard guarantee that no secret ever reaches a log line. The rollout is phased across four steps (see Phasing): phase 1 wires the logging pipeline (rotated file + Android logcat) and is shipped; phase 2 adds the in-app viewer, runtime level control, and the redacting-`Debug` safety hardening (implemented); phase 3 is broad instrumentation; phase 4 is export/share. This RFC records the design for the complete feature and where each phase stands.

## Why

Today gpm has no observability. When a push is rejected, a decrypt fails, or a setup step errors, there is no record the user can attach to a bug report; the only signal is a sanitized error code flashed briefly in the UI. The pre-existing debug prints go to stderr, are unstructured, are not file-persisted, and are invisible on Android. Logging unlocks user-driven diagnostics (a shareable log for support) and makes future instrumentation and incident postmortems possible.

The hard requirement, which shapes the whole design, is that a password manager's logs must never contain a secret. That constraint interacts with the export feature in a non-obvious way (see Context), and the decision to keep logs unencrypted is a deliberate threat-model call worth recording.

## Context

**Stack.** Use the official Tauri logging plugin (built on the `log` facade). It provides, off the shelf, size-based file rotation, automatic routing of stdout output to Android logcat (there is no separate logcat target in the plugin), and a frontend logging bridge so Vue code can share the same backend. The `log` facade is runtime-agnostic, so log calls are safe inside the async runtime, inside work offloaded to blocking threads, and inside native-library callbacks — all patterns gpm uses heavily. `tracing` was considered and rejected (see Alternatives).

**Threat model — plaintext logs, "never log a secret."** Logs are stored unencrypted. This is deliberate. Secrets are excluded by construction, not by encryption: only entry names and operation outcomes (plus already-sanitized error codes) are logged; credential-bearing configuration types must redact their secret fields before any debug formatting; and decrypted content, passphrases, identity material, and the at-rest master key are never passed to the logger. Given that, there is nothing in a log worth protecting with encryption. Furthermore, an attacker who can read the on-device log file already has filesystem access to the repository directory, so entry paths — the only "sensitive-ish" data that remains — give them nothing they did not already have. One caveat: that premise holds for the log _file_ but not for the _logcat_ channel, which an attacker with pre-authorized USB debugging can read without repository filesystem access; entry-name metadata (which entries are copied, and when) is therefore visible to that narrower attacker class. The exposure is metadata only (never secret content), requires prior debugging authorization, and matches how any logging app behaves, so it is folded into the phase-2 redaction work rather than treated as a reason to encrypt. Encrypting logs would add a key lifecycle (necessarily tied to the same master key that protects the real secrets) for no meaningful gain, and would couple logging to the unlock lifecycle, breaking the very use case it serves (reading logs to diagnose an unlock or setup failure). The governing rule is therefore "never log a secret," which is stricter than "log it encrypted."

**Export changes the threat surface — the load-bearing phase 2 decision.** The reasoning above holds for the log file at rest on the device. It does _not_ hold for an exported log. The export-and-share flow exists precisely to push a log off the device (to email, a messenger, a support channel), so when the user taps export, entry names leave with it to a third party the user chooses. At that point the "attacker already has filesystem access" premise is gone. Phase 4 must therefore treat the exported log as a distinct, more-sensitive artifact: surface a clear pre-export warning that entry names leave the device, and/or offer a redacted export that strips entry identifiers. This is the one place where plaintext logging needs an explicit user gate.

**Phasing — four phases, isolating risk.** The rollout splits the foundation (pipeline), the usable viewer, the instrumentation, and the cross-device export so each can be verified before the next depends on it, and so the highest-risk piece (Android share) lands last. The original design bundled viewer + export + instrumentation into one "phase 2"; it is split here into phases 2–4 for that isolation.

- **Phase 1 — pipeline (DONE, shipped `dff9890`).** Wire `tauri-plugin-log` to a rotated file under `app_log_dir()` and to Android logcat via the `Stdout` target, at a fixed Info level, and migrate the legacy `eprintln!` onto the real logger. Ships no UI and instruments almost nothing — it only proves the pipeline.
- **Phase 2 — viewer, runtime level, safety hardening (IMPLEMENTED).** The redacting-`Debug` hardening lands first (closes a latent leak and gates instrumentation), then a Settings → Logs viewer that reads and clears the rotated log (mtime-ordered, tail-truncated), runtime level control persisted in `app.json` and applied immediately via `log::set_max_level` (the plugin is configured at Trace and gated at runtime), and a light frontend logging bridge that routes uncaught frontend errors into the backend log via a custom `write_log` command. Export/share and broad instrumentation are deliberately deferred to phases 4 and 3, so phase 2 is "get logging running end-to-end" without them.
- **Phase 3 — instrumentation (PLANNED).** Broad but disciplined logging across the key operations — git clone/pull/push/sync, decrypt/copy/show, create/edit/delete, setup, identity and app-lock transitions, biometric, authenticity — recording entry name + outcome (never content), plus logs on the error paths currently swallowed silently. Gated behind phase 2's redacting-`Debug` work.
- **Phase 4 — export/share (PLANNED).** Get the log off the device for support: native save dialog on desktop, system share sheet on Android via a small plugin that reuses the app's existing `FileProvider` (the export snapshot is written to app cache, so no new file-provider paths are needed), with the pre-export warning that entry names leave the device and/or an optional redacted export. Highest-risk piece (Android share) and the one place plaintext logging needs an explicit user gate — hence last.

**Safety ordering.** Broad instrumentation must not land before the credential-bearing types that currently leak secrets through their debug representation are made to redact. That hardening is therefore the first step of phase 2, ahead of the viewer, and is valuable on its own: it closes a latent leak that exists today, independent of logging.

## Alternatives considered

- **`tracing` instead of the Tauri logging plugin.** Rejected. `tracing` would require hand-building size-based file rotation (its rolling appender rotates by time only), the Android logcat bridge, and the frontend bridge the viewer depends on. Its contextual-span power is low-value for a mostly-serial mobile password manager and is eroded where it would matter most (work offloaded to blocking threads does not carry spans without manual propagation). The plugin gives the needed backends for free.

- **Encrypting the log file at rest.** Rejected, as above: nothing worth protecting reaches the log by construction, and encryption tied to the master key would couple diagnostics to unlock. "Never log a secret" is the rule.

- **Encrypting only the exported log.** Considered and rejected as the primary defense; the export problem is better solved by a user warning plus an optional redacted export, because the recipient (support) needs to read the log, and a shareable key defeats the purpose.

- **Live log tail (event stream into the viewer).** Deferred beyond phase 2. Refresh-on-open is sufficient for a diagnostics viewer and avoids a persistent event channel; the plugin's webview target makes tail additive later.

- **A single large change.** Rejected in favor of phasing. Splitting the pipeline (phase 1) from the UI (phase 2), instrumentation (phase 3), and export (phase 4) lets the foundation be verified before anything depends on it, and isolates the higher-risk Android share work.

## Effort

Phase 1: small, shipped (~1 human-day / ~15 min CC).
Phase 2: medium (~2 human-days / ~30 min CC) — redacting-`Debug` hardening, viewer, runtime level control, frontend bridge.
Phase 3: medium (~1–2 human-days / ~30 min CC) — disciplined instrumentation across the key operations + the swallowed-error paths.
Phase 4: medium (~2 human-days / ~45 min CC) — export/share plugin (desktop save + Android share via the existing `FileProvider`) + pre-export warning / redacted export.

## Depends on / Supersedes

None. Aligns with the existing at-rest encryption and sanitized-error threat model documented in the security model, and extends it with the "exported log leaves the device" consideration.
