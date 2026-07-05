# GPG/OpenPGP crypto backend (rpgp)

**Priority:** P1
**Status:** Draft
**Phase:** Next

## What

Add GPG (OpenPGP) as a second crypto backend alongside age, implemented with the
pure-Rust `pgp` crate (rpgp), targeting gopass-GPG repository compatibility:
open, list, decrypt, create, and sync the GPG-encrypted stores that traditional
gopass deployments use. This is the first real second backend behind the
`CryptoBackend` abstraction shipped in 0033, and the concrete implementation
that informs — and is expected to reshape — that trait's shape.

## Why

gopass ships two crypto backends in the real world: age and GPG. gpm today
speaks only age, so any gopass store encrypted the traditional way (the default
for years, and still the norm in team/enterprise setups, CI bots with GPG keys,
and long-established stores) is unreadable. Adding GPG opens that existing
ecosystem rather than asking users to re-encrypt their store to age.

It is also the deliberate purpose of the 0033 abstraction. 0033 extracted a
`CryptoBackend` trait from a single implementation, explicitly budgeting that
the trait shape would be revised when a real second backend arrived — "trait
shapes guessed from a single implementation tend to be reworked." GPG is that
second backend. Its different identity model (a keyring key selected by
fingerprint, not a pasted identity string), its recipient model (fingerprints
resolved through a keyring, not parseable public-key strings), and its need for
carried state (an in-app keyring, where Android has no system `~/.gnupg`) are
exactly the pressure that tells us whether the current trait shape survives. The
work is as much "finish the abstraction" as "add a backend."

## Context

**The reference model — gopass's GPG backend.** gopass's GPG is a thin shell
over a system `gpg` binary; what gpm mirrors is the on-disk format, not the
shell-out architecture. Secrets are binary RFC 4880 OpenPGP — one public-key
session-key packet per recipient plus a single integrity-protected data packet
(SEIPD v1, with MDC), uncompressed because gopass passes `compress-algo=none` —
written as `<name>.gpg`. Recipients are listed in `.gpg-id`, one per line with
`#` comments; modern gopass stores the canonical long key id (`0x` plus the last
16 hex of the fingerprint), and subdirectories may carry their own `.gpg-id` for
team partitioning. Public keys for recipients live as armored blobs under
`.public-keys/<id>`. An "identity" is a secret key in the keyring, addressed by
fingerprint — there is no analog to age's pasted `AGE-SECRET-KEY-...` string;
setup is "generate a keypair into the keyring" or "import an existing secret
key," and unlock is that key's passphrase (gopass delegates to `gpg-agent`;
Android has none, so gpm does it itself).

**Why rpgp, not Sequoia or gpgme.** The OpenPGP implementation is `pgp` (rpgp),
a pure-Rust, MIT/Apache-2.0 crate. The decision is dominated by the Android
constraint. Sequoia (`sequoia-openpgp`) is the more complete library and
cross-compiles cleanly to Android only through its pure-Rust `crypto-rust`
backend, but it is LGPL-2.0+; static linking into a Play-distributed APK
triggers the LGPL's relinking obligations in a way the Play signing /
repackaging model makes materially awkward — a recurring per-release legal
burden the existing MIT/Apache age stack does not carry. gpgme is a dead end on
Android: it is an IPC wrapper that spawns a `gpg` binary, and Android 10+
forbids executing bundled binaries — the same wall that already blocks
age-plugin-yubikey on this device. rpgp avoids both: pure Rust (a trivial NDK
cross-compile through the existing flake toolchain, nothing C to vendor), a
permissive license, and a real mobile
deployment behind it: Delta Chat's Rust core uses rpgp across its Android, iOS,
and desktop builds. There is no Tauri + Android + rpgp app to cite as a
same-stack precedent — the closest cross-platform Rust PGP project instead
chose Sequoia — so the Android cross-compile confidence rests on rpgp being
pure Rust and on Delta Chat's mobile use, not on a same-stack app. That
assumption has since held up empirically: the RFC 0009 verification spike
cross-compiled rpgp to the Android NDK through the existing flake toolchain
with no system C crypto pulled in, moving the crate from "pure-Rust by
inspection" to "pure-Rust by build proof" and retiring the cross-compile risk
this paragraph had to argue around. Its
format coverage — multi-recipient encryption, SEIPD v1 (and v2 / RFC 9580 AEAD),
S2K passphrase-protected secret keys including Argon2, and RSA / Curve25519 /
Ed25519 / NIST curves — is everything gopass produces, and its secret material
is zeroized on drop, matching gpm's existing wipe discipline.

**What rpgp does not do is scoped out the way age-plugin-yubikey is (RFC 0030):
recognized but unsupported, with a clear honest error rather than a silent
failure.** rpgp has no OpenPGP-card / YubiKey hardware-key path, no Brainpool
curves, no LibrePGP AEAD variant, no Elgamal, and no web-of-trust semantics —
its surface is packet formats and crypto primitives (RFC 4880/9580 layers 1–3),
not key policy (layer 4: key flags, expiry, revocation). None of the missing
algorithms arise on a gopass-compatible path; the policy layer is a thin
wrapper gpm writes around rpgp (or borrows from the same team's higher-level
`rpgpie` crate), not a reason to reject it.

**Android changes the model, not just the library.** With no system `gpg`,
`gpg-agent`, or `~/.gnupg`, the GPG backend must carry its own keyring in
app-private storage: a pool of recipient public keys (read from and written
back to gopass's `.public-keys/` so desktop gopass round-trips), and the user's
own passphrase-protected secret key. Identity setup is in-app keypair generation
(gopass defaults to RSA-2048; gpm may modernize to Curve25519, documenting the
divergence) or secret-key import — fundamentally different from age's "generate
and paste an identity string," so the setup UI gains a crypto-kind selector with
a GPG-specific sub-flow. Unlock is the secret-key passphrase, retrievable
through the existing biometric-keystore plugin the way the age identity
passphrase is today, with the same Immediate / Idle-timeout / Never auto-lock
modes and the same wipe-after-use discipline.

**Threat-model impact — carried state, unchanged secret flow.** The guarantees
that decrypted plaintext never reaches the WebView, and that all decrypted
content is zeroized and wiped after use, are preserved verbatim — rpgp returns
plaintext into Rust and we wipe it the same way age output is wiped. What
changes is the at-rest surface: the in-app keyring (especially the secret key)
is new durable secret-bearing state and is AEAD-encrypted at rest with the
master key the secure-keystore plugin already seals — the same protection the
repo configuration and the age identity get today. The recipient public-key half
of the keyring is not secret. Error-message sanitization carries over unchanged.

**This backend will force trait rework — by design.** The 0033 `CryptoBackend`
shape is age-shaped: an identity is bytes the caller pastes, a recipient is a
parseable public-key string, and the backend is stateless. GPG matches none of
that: an identity is a keyring entry addressed by fingerprint, a recipient is a
fingerprint resolved through the keyring, and the backend must hold keyring
state. The rework 0033's "conscious tradeoff" budgeted lands here. The direction
— make keyring ownership, recipient resolution, and the identity model the
backend's own responsibility rather than the facade's — is clear; the exact
trait shape is deliberately not fixed in this RFC and falls out of the
implementation, not ahead of it. The same pressure revisits the secret file
extension (`.age` vs `.gpg`) and the recipients-file format (`.age-recipients`
vs `.gpg-id`), both currently hardcoded in the storage layer; both become
per-backend properties.

**Shared crate decision unlocks 0009.** Settling on rpgp here settles the
OpenPGP-crate question that RFC 0009 (GPG commit-signature verification) also
depends on. The two features are separate — one encrypts secrets, one verifies
commit signatures — but they should share the one OpenPGP implementation; this
RFC picks it for both.

## Alternatives considered

1. **Sequoia (`sequoia-openpgp`, `crypto-rust` on Android).** More complete,
   more maintained, RFC 9580 first-class, and a higher-level policy API in-box.
   Rejected: the LGPL-2.0+ static-link burden on a Play-distributed APK is the
   dominant cost, not the engineering, and it is a recurring per-release
   obligation the MIT/Apache age stack does not pay. Reconsider only if a
   concrete need forces it (Brainpool curves, an OpenPGP-card flow, or
   Sequoia's policy layer).

2. **gpgme / libgpgme FFI.** Maximally gopass-compatible — it is what gopass
   uses, via the system `gpg`. Rejected: gpgme shells out to a `gpg` binary over
   libassuan, and Android 10+ forbids executing bundled binaries. The full
   GnuPG C stack (libgcrypt, libgpg-error, libassuan, libksba, npth, plus the
   `gpg` binary itself) is also multi-day autotools cross-compile pain for a
   result that cannot run. Desktop-only gpgme would work but splits the
   codebase across two crypto paths — the inverse of what an Android-first app
   wants.

3. **BouncyCastle (or PGPainless) via JNI on the Android Kotlin layer.**
   License-clean (MIT), the most interop-tested OpenPGP implementation, and no
   NDK pain. Rejected as the primary path: desktop Tauri has no JVM, so the
   Rust side would still need its own implementation, splitting one
   gopass-compatibility path across two languages and doubling the divergence
   risk on the same shared store. Kept as a fallback.

4. **Defer the backend until the trait shape is "right."** Rejected: 0033 made
   this argument and rejected it — the trait cannot be shaped correctly from a
   single implementation, and the coupling cleanup (keyring state, per-backend
   file and recipients naming) is valuable regardless of when the second
   backend arrives. The spike writes the backend first and lets it reshape the
   trait, exactly as 0033 budgeted.

## Effort

Large — substantially bigger than 0033 (a pure refactor with no behavior
change): a new crypto implementation, an in-app keyring, a key generation /
import flow, a GPG-specific setup sub-flow, and the trait rework the second
backend triggers. A rpgp spike should land first — encrypt to a fixed recipient
set, decrypt one passphrase key, prove the Android NDK build, and verify a
round-trip against desktop gopass — so the trait reshape is informed by working
code rather than designed ahead.

## Depends on / Supersedes

Depends on `0033-multi-backend-abstraction` (the abstraction this backend
exercises and reshapes; must land first). Relates to `0009-gpg-signature-verification`
(this RFC settles the shared OpenPGP-crate pick that 0009 also needs) and to
`0030-age-plugin-yubikey` (the "recognized but unsupported" pattern this RFC
reuses for OpenPGP-card and the missing algorithms).
