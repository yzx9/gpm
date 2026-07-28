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

## Approaches not adopted

| Approach                        | Why not                                                                                                                                                                                    |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Tauri Isolation Pattern         | Encrypts the frontend→Rust IPC direction (protects against a malicious frontend calling Rust commands), not the Rust→frontend response. CSP is a more direct defense for our threat model. |
| Custom IPC encryption layer     | Both ends run in the same process — the decryption key would be in the same process. Security theater.                                                                                     |
| Canvas-based password rendering | Would avoid DOM text nodes, but accessibility services can OCR rendered content. Extreme complexity for marginal gain.                                                                     |
| JavaScript memory overwriting   | V8 strings are immutable; overwriting creates a new string and reassigns the reference, leaving the original on the heap until GC. Security theater.                                       |
