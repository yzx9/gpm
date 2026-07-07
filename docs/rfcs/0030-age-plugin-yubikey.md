# age plugin support (age-plugin-yubikey and the generic plugin protocol)

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Recognize and operate on age _plugin_ recipients and identities — the `age1<plugin>1...`
recipient encoding and the `AGE-PLUGIN-<NAME>-1...` identity encoding defined by the age
plugin protocol — with `age-plugin-yubikey` as the primary target. On platforms where the
matching `age-plugin-<name>` binary is reachable, encrypt secrets to plugin recipients and
decrypt them with plugin identities, transparently, exactly as the `age` CLI and gopass do.
On Android (where no such binary can run), recognize the encodings and surface a clear,
honest "not available here" error instead of a cryptic parse failure.

## Why

Two things break today, and one thing is impossible today:

1. **A store that shares a yubikey recipient is silently broken for writes.** A teammate's
   `age1yubikey1...` line in the shared recipients file is currently misclassified as a
   native x25519 recipient (it shares the `age1` prefix), so the encryption step fails to
   parse it and the whole write aborts. The most common real-world yubikey scenario —
   _someone else_ on the shared store uses a hardware key — is enough to make gpm unable to
   add or edit any secret in that store.

2. **Plugin identities are invisible.** An `AGE-PLUGIN-YUBIKEY-1...` identity is today
   "unknown," so it cannot be imported or used at all, even on a desktop that has the
   plugin installed and a key plugged in.

3. **The dominant shared-store need is the recipient (encryption) side; the identity
   (decryption) side is desktop-only.** A yubikey-owning desktop user should be able to use
   their key as their gpm identity end-to-end. On Android that is infeasible by the plugin
   model itself (see Context), and should fail with a real message, not a stack trace.

## Context

**How age plugins work.** The age plugin protocol fronts hardware-backed or exotic key
types behind an external `age-plugin-<name>` binary that the age library spawns as a
subprocess, speaking a line-based protocol over stdio. Encryption _to_ a plugin recipient
and decryption _with_ a plugin identity both require that subprocess to run; the recipient
and identity strings are just bech32-encoded references into it. The Rust age crate exposes
this through its `plugin` feature: a plugin recipient/identity parses from the bech32
string, but the actual `Recipient`/`Identity` trait implementation is a wrapper that locates
the binary on `PATH` (returning a "missing plugin" error if absent) and drives it at
wrap/unwrap time.

**How gopass does it (verified against source).** gopass consumes the Go age library
in-process — it does not shell out to the `age` CLI — and gets plugin support "for free"
from the library, exactly as this crate would. Three details from its implementation are
load-bearing for us:

- gopass dispatches by prefix before parsing: `AGE-PLUGIN-` identities and `age1...`
  recipients that fail the native x25519 parser are routed to the plugin constructor. The
  generic identity parser rejects plugin identities outright, so prefix dispatch is
  mandatory.
- gopass stores a plugin identity together with its recipient as `IDENTITY|RECIPIENT`. The
  recipient encoding **cannot** be recovered from the identity encoding (the two bech32
  strings are unrelated), and gopass needs the recipient to (a) list the identity's public
  key without touching the hardware and (b) encrypt-to-self. This is the canonical fix for
  "what recipient does my yubikey identity correspond to."
- gopass routes plugin UI (touch prompt, PIN request) through a small callback shim. Our
  equivalent is the age crate's `Callbacks` trait; for a yubikey, the meaningful callback is
  the PIN request, which maps cleanly onto the passphrase a user already supplies.

**Platform reality (Android).** The plugin model is fundamentally subprocess-based: the
library must exec `age-plugin-<name>`, and `age-plugin-yubikey` in turn must talk to the
YubiKey over USB or NFC. Neither is possible inside an Android app: there is no such binary
on the device, apps cannot freely exec arbitrary binaries, and YubiKey access on Android
requires the platform USB-Host / NFC ISO-7816 APIs that only an Android app context can
open. So plugin encrypt/decrypt cannot work on Android through the upstream library. The
only path to yubikey _on Android_ is a native, in-process reimplementation (talk to the key
directly via a PIV-capable crate over Android USB/NFC, reusing the age stanza format) — a
large, separate project. This RFC therefore delivers the desktop story in full and treats
Android as "recognize and refuse, with a clear reason," mirroring the existing documented
desktop/Android asymmetry around hardware-backed crypto.

**Threat-model impact.** Spawning a user-installed `age-plugin-<name>` is the same trust
boundary the `age` CLI and gopass already assume: the user trusts the binary they installed.
No secret crosses into the WebView; the plugin binary receives only age file keys / stanzas
over its stdio protocol, as designed. The identity string for a hardware key is not itself
secret (it is a reference to a key locked in hardware), so storing it under the existing
at-rest envelope changes nothing about at-rest confidentiality. The `|RECIPIENT` suffix is a
public key. Net new surface is the subprocess spawn on desktop and a new, clearly-labeled
error code for the missing-binary case; both are lower-risk than the existing SSH-key path.

## Recommended decision

Ship full plugin support in the library layer, desktop-operative, with honest Android
handling.

**Staging.** This design lands in two coherent halves because the two directions are not
symmetric in cost or testability:

- **Now (this change): recognition everywhere + encrypt-to plugin recipients.** Pure
  parsing plus the encryption direction. Fixes the broken-write bug for any store that
  shares a yubikey recipient, works wherever the plugin binary exists (desktop), and
  refuses cleanly where it cannot (Android, missing binary). Fully deterministic-testable.
- **Next (follow-on): decrypt-with-a-plugin-identity.** A plugin identity is a hardware
  reference, not a decryptable blob, so "unlock" means physical presence plus a YubiKey PIN
  at decrypt time. That needs per-operation PIN plumbing the app does not have today, plus a
  frontend prompt, and it can never run on Android. Until that lands, plugin _identities_
  are recognized but rejected as not-yet-supported (the same pattern used for post-quantum
  keys), while plugin _recipients_ are fully supported.

1. **Recognize everywhere.** A new "plugin" key/identity type, detected by prefix, for both
   recipients (`age1<plugin>1...`, excluding the already-special-cased post-quantum
   encoding) and identities (`AGE-PLUGIN-<NAME>-1...`). This alone fixes the broken-write
   bug's classification half and lets plugin identities be imported. This part is pure
   parsing and works on every platform.

2. **Encrypt to plugin recipients** by grouping recipients per plugin, constructing the
   library's plugin-recipient wrapper, and feeding it to the existing encrypt step
   alongside native and SSH recipients. Requires the binary; on its absence, map to a clear
   error.

3. **Decrypt with plugin identities** by routing `AGE-PLUGIN-<NAME>-1...` identities to the
   library's plugin-identity wrapper, with a callback shim that supplies a YubiKey PIN from
   the passphrase the user already provides. Touch/insert prompts are inherently physical
   and need no UI bridge.

4. **Adopt the gopass `IDENTITY|RECIPIENT` convention** so the encrypt-to-self ("ensure our
   own key is a recipient") and "show my recipient" paths work for a plugin identity: the
   recipient is carried as a suffix, stripped before parsing, and used directly for
   derivation. Native and SSH identities are unaffected.

5. **Android refusal.** Recognize plugin encodings, but since the binary cannot exist
   there, operations fail with the same missing-plugin error the desktop would give — no
   silent corruption, no mystery parse failure.

6. **Mirror the type** through the IPC layer and the frontend identity classifier so the UI
   labels plugin keys correctly and the import flow accepts them.

## Alternatives considered

- **Bundle or exec `age-plugin-yubikey` on Android.** Rejected: Android apps cannot run
  such a binary, and even a bundled one could not reach the YubiKey without the platform
  USB/NFC APIs. This is the gap that motivates the separate native-Android RFC below.

- **Native in-process yubikey on Android now.** Rejected for this RFC: it is a large,
  hardware-and-platform-specific effort (PIV over Android USB/NFC, reusing the age stanza
  format) that does not belong in the desktop-focused plugin landing. Recorded as the
  follow-on.

- **Shell out to the `age` CLI instead of using the library plugin module.** Rejected: the
  library already drives plugins in-process with no shell-out, matching gopass; a CLI
  dependency would add a runtime binary requirement on desktop and still be impossible on
  Android, strictly worse.

- **Defer the identity (decrypt) side and ship only encrypt-to.** Rejected: half a feature
  — a yubikey-owning desktop user could encrypt but never decrypt. The decrypt path is
  mechanically symmetric (route to the plugin wrapper) and ships with the rest.

## Effort

~1–1.5 days (human) / ~30–45 min (CC) — library-layer recognition + both plugin directions

- the `|RECIPIENT` convention + IPC/frontend type mirror + deterministic tests. Hardware
  round-trip is validated manually / via an opt-in ignored test; it cannot be a CI gate.

## Depends on / Supersedes

Builds on the existing recipient/identity classification and the post-quantum special-case
precedent (0008). The follow-on — native in-process YubiKey support on Android (USB/NFC PIV
without the subprocess) — is left as a separate future RFC; this one records why the desktop
plugin path is the right first step and why Android must wait for it.
