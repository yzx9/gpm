# Security Model

gpm is a local-first, age-encrypted gopass client. It clones a gopass repository to
the device, decrypts entries on demand, copies/shows secrets, creates/edits/deletes
them, and syncs over **git to a repo you control** (self-hosted — no third-party cloud).
Secret encryption is **age-only** (a GPG/OpenPGP backend is in progress; see `specs/004`).

Feature-specific threat models live in `docs/specs/<NNN>/security.md`:
001 entry access, 003 age, 004 GPG, 005 git storage & sync, 006 identities, 007 app lock,
008 android autofill (planned).
This document covers only the cross-cutting model.

## Threat model & non-goals

gpm defends against **local opportunistic access** — someone who briefly has physical
access to an unlocked device, or a malicious app that somehow injects script into the
WebView. That threat is bounded because secrets are kept out of the WebView by design:
`copy_password` never crosses the IPC boundary, and only `show_password` intentionally
exposes a secret to the DOM, on demand (see `specs/001/security.md`). It does **not**
defend against:

- a fully compromised OS,
- a determined attacker with root, or
- a process running as the app (which could read process memory or ask the Keystore to
  unseal keys regardless).

These are explicit non-goals, not gaps to be closed.

## System-wide assumptions

- **No local write attacker.** gpm assumes no local attacker has write access to the
  app's private storage. On Android this rests on the app sandbox; on desktop there is
  no Keystore equivalent, so private files stay plaintext and the assumption rests on
  the user account not being compromised. (At-rest encryption and App Lock harden this —
  feature-specific, see `specs/007/security.md`.)
- **Master key in memory for the session** is consistent with the non-goals above — no
  more sensitive than the git credentials gpm already holds in memory while syncing.

## System-wide measures

| Measure                  | Detail                                                                                                                                        |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Zeroizing memory         | Rust `Secret` wraps `Zeroizing<String>`; content wiped on drop                                                                                |
| Safe Debug output        | Custom `Debug` impl shows `[REDACTED]`, never actual secrets                                                                                  |
| Error sanitization       | Error messages contain only codes and generic descriptions                                                                                    |
| Path traversal guard     | Resolved paths validated to stay within repository; symlink escape detection                                                                  |
| Content Security Policy  | CSP restricts `script-src`, `connect-src` to `self` and IPC only                                                                              |
| WebView script integrity | Secrets render as text (Vue-escaped), never as executable HTML (`v-html`/`innerHTML`); the only script in the WebView is the app's own bundle |

## Cross-cutting runtime limitations

These are fundamental to the WebView/runtime, not bugs, and apply wherever a secret is shown:

- **JavaScript memory persistence.** Setting `password.value = null` clears the Vue ref
  but does not zero the underlying V8 string (strings are immutable). Plaintext may
  persist until garbage collection. There is no reliable way to deterministically zero
  JS string memory.
- **`show_password` plaintext in IPC.** The `SensitiveContent` response crosses the
  Rust → WebView boundary as plaintext JSON — by design, since the password must be
  displayed. Tauri v2 IPC is process-local (the custom `ipc:`/`http://ipc.localhost`
  protocol, or the Android JNI bridge); it does not traverse a network socket.
- **Android accessibility services** can read displayed text. Inherent to showing text
  in a WebView; no reliable way to display text while hiding it from accessibility.

## Diagnostics logging

gpm writes a structured diagnostics log — a rotated file under the app log
directory, mirrored to Android logcat — covering operational outcomes: git
clone/pull/push/sync, decrypt/copy/show, create/edit/delete, setup, identity
and app-lock transitions, biometric, and authenticity verification. The
governing rule is **never log a secret**: only entry names and operation
outcomes (plus the already-sanitized error codes) are ever recorded. Decrypted
content, passphrases, identity material, and the at-rest master key never reach
the logger, and credential-bearing configuration types redact their secret
fields before any debug formatting.

**Logs are unencrypted by construction, not by encryption.** Given that rule,
nothing worth protecting reaches a log line. And an attacker who can read the
on-device log file already has filesystem access to the repository, so the
entry-name metadata a log carries (which entries were copied, and when) gives
them nothing they did not already have. Encrypting logs would add a key
lifecycle — necessarily tied to the same master key that protects the real
secrets — for no meaningful gain, and would couple diagnostics to the unlock
lifecycle, breaking the very use case it serves (reading logs to diagnose an
unlock or setup failure).

**Caveat — the logcat channel.** That argument holds for the log _file_ at
rest. It does _not_ hold for the Android logcat channel the logger mirrors to,
which an attacker with pre-authorized USB debugging can read without repository
filesystem access; entry-name metadata is therefore visible to that narrower
attacker class. The exposure is metadata only (never secret content), requires
prior debugging authorization, and matches how any logging app behaves — so it
is accepted as-is rather than treated as a reason to encrypt. (The diagnostics
export bundle — full log plus a redacted view of the repository config and
device info, written off-device on demand to a location the user picks — is that
separate, more-sensitive artifact: it is gated by an explicit pre-export
confirmation and assembled only from already-redacted sources, so no secret is
read into the bundling path; see `app/src-tauri/src/diagnostics_export.rs`.)

## Approaches not adopted

| Approach                        | Why not                                                                                                                                                                                    |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Tauri Isolation Pattern         | Encrypts the frontend→Rust IPC direction (protects against a malicious frontend calling Rust commands), not the Rust→frontend response. CSP is a more direct defense for our threat model. |
| Custom IPC encryption layer     | Both ends run in the same process — the decryption key would be in the same process. Security theater.                                                                                     |
| Canvas-based password rendering | Would avoid DOM text nodes, but accessibility services can OCR rendered content. Extreme complexity for marginal gain.                                                                     |
| JavaScript memory overwriting   | V8 strings are immutable; overwriting creates a new string and reassigns the reference, leaving the original on the heap until GC. Security theater.                                       |
