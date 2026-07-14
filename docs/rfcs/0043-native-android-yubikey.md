# Native in-process YubiKey support on Android

**Priority:** P2
**Status:** Draft
**Phase:** Future

## What

Deliver the YubiKey identity that RFC 0030 deferred: decrypt with an
`age-plugin-yubikey` identity **in-process on Android**, without spawning any
binary, by talking to the key directly over the platform USB or NFC transport and
reusing the upstream age-plugin-yubikey stanza format. The desktop story stays as
0030 shipped it (the age library driving the `age-plugin-yubikey` subprocess);
Android replaces the subprocess with a native, in-app PIV implementation behind
the same identity seam, so a YubiKey-owning user gets a working hardware-key
identity on both platforms.

## Why

RFC 0030 deliberately landed only "recognize and refuse" on Android, because the
age plugin model is subprocess-based and cannot run there. That leaves the most
security-conscious Android user — the one who carries a hardware key — unable to
use that key as their gpm identity at all: the identity is recognized but rejected
as unsupported. gpm is Android-first, so this is the platform where the
hardware-key identity matters most, and it is the one place it does not work.
Closing the gap also retires the documented desktop/Android asymmetry around
hardware-backed crypto (the keystore asymmetry is accepted; the YubiKey one was
always meant to be temporary).

## Context

**Why the subprocess path is closed on Android.** 0030 records this in full: the
age plugin protocol fronts hardware keys behind an external binary the age library
must exec, and that binary must in turn reach the key over USB or NFC. Android 10+
forbids executing bundled binaries, there is no such binary on the device, and key
access requires platform USB/NFC APIs only an app context can open. This RFC does
not re-argue that; it picks up the path 0030 explicitly left open — a native,
in-process reimplementation of the YubiKey identity.

**The crypto ground truth (verified against age-plugin-yubikey's source and the
PIV standard).** An age-plugin-yubikey identity is a PIV ECC **P-256** key
generated inside the YubiKey's PIV applet, in one of the "retired"
key-management slots. The private key never leaves the hardware. Decryption is
**PIV key agreement (ECDH)**: the age stanza carries an ephemeral public key, the
YubiKey combines it with its on-card private key to produce a shared secret, and
the plugin's stanza format turns that shared secret into the age file key. Two
facts are decision-relevant:

- It is **P-256, not X25519.** PIV does not support Curve25519, so the plugin
  bridges age's X25519 world to a P-256-on-PIV operation. An in-process impl must
  perform P-256 ECDH and must not assume X25519 primitives.
- There are now **two stanza formats** in the wild — the original HKDF-based one
  and a newer HPKE-based one. Both are valid encodings of a yubikey recipient, so
  a complete impl must handle both to interoperate with keys minted by current and
  future `age-plugin-yubikey` versions.

The PIV **PIN** gates each private-key operation (per the slot's PIN policy); the
**touch policy** gates physical contact; the **management key** is only for key
generation and is never touched at decrypt time. There is no agent: PIN caching is
a property of keeping the PIV session open, which a desktop reader can do but an
NFC tap cannot — over NFC the PIN is effectively re-entered per operation. These
map cleanly onto the passphrase prompt and the physical-presence prompt the app
already has.

**In-process on Android is proven, by Yubico itself.** No separate companion app
is required, and no root. The Android USB and NFC stacks are part of the OS, not
third-party apps, so an app can claim the key's USB CCID interface directly and
speak ISO-7816 to it, or act as the NFC reader against the key emulating an
ISO-7816 card. Two independent precedents confirm this: Yubico's own Android SDK
provides a transport abstraction plus a PIV session that already performs the
key-agreement operation, and the open-source Yubico Authenticator app does all of
its USB-CCID and NFC ISO-Dep I/O in-process. The hard part of the Android path —
PIV key agreement over both transports — already exists in audited,
Yubico-maintained code.

**NFC is the dominant but fragile transport.** Over NFC the phone is the reader
and the key is the card; a single decrypt is several request/response round-trips
(select applet, verify PIN, key agreement) that must all complete within one
physical tap. Removing the key mid-operation loses the connection and restarts the
sequence. NFC is cable-free and ubiquitous, which makes it the realistic default
for phones, but it trades reliability for convenience; USB-OTG is the durable, fast
fallback for the small set of phones and keys that have matching USB-C hardware.
Robust apps surface a "hold the key to the back of the phone" gate and tolerate tag
loss by retrying the whole sequence.

**The Rust host-side stack does not help on Android, but the reusable seams do.**
The upstream Rust YubiKey crate is bound to PC/SC, which Android does not provide,
and no fork swaps the transport; there is also no usable Rust USB/CCID host stack
for Android. So the transport and the PIV framing are reused from Yubico's Kotlin
SDK, bridged to Rust over the same plugin IPC the biometric and secure-keystore
plugins already use — the identical trust boundary, with the key never reachable
from the WebView. Conversely, upstream age-plugin-yubikey's stanza logic is
separable from both its PC/SC transport and its stdio plugin protocol: only a
single key-agreement primitive depends on the hardware, and the rest (stanza parse,
shared-secret-to-file-key derivation, the two format variants) is pure software.
That means the crypto-format logic is **reused, not reimplemented** — the one
hardware touchpoint is what gets injected.

**The identity-model tension this exposes.** Today the engine decrypts an identity
once into bytes and caches them, reusing those bytes to decrypt every entry. A
hardware-key identity is fundamentally different: it is not bytes that can be
cached, it is a capability that must be exercised per operation (PIN, optional
touch, and an ECDH exchange every time, or a PIN cached only for the life of a
transport session). The seam therefore has to move from "a cached bag of identity
bytes" to "an identity handle that produces a per-operation file-key unwrap." This
is the same kind of pressure on the crypto-backend/identity abstraction that the
GPG backend (0036) already records — the abstraction was explicitly budgeted to be
reshaped when a second identity model arrives, and a hardware-key identity is
exactly such a model. Landing this RFC means landing that reshape, not bolting a
branch onto the byte-cache.

## Recommended decision

Treat the YubiKey identity as a **transport-injected hardware identity**, and
specifically _not_ as an extension of the age subprocess plugin protocol (the wrong
layer for Android). The seam is an identity whose per-operation file-key unwrap
dispatches to an injected transport:

- **Desktop** keeps 0030's path: the age library's subprocess plugin,
  transparently, with the PIN flowing through the library's existing callback. No
  new gpm code on this path.
- **Android** replaces the subprocess with an in-process PIV identity whose unwrap
  performs the key agreement over the YubiKit-bridged transport and then runs the
  same reusable stanza derivation.

This mirrors the injection philosophy the at-rest master key already uses — it
crosses a boundary as injected bytes, and the engine never knows whether those bytes
came from a hardware keystore or a desktop passthrough. The hardware identity's
per-op unwrap is the analogous injected capability. It lives in a new native plugin
crate of the same shape as the biometric/secure-keystore plugins: the Kotlin side
owns the USB/NFC transport and the PIV session, the Rust side owns the age-stanza
logic and drives the unwrap, and the derived file key crosses Kotlin→Rust as bytes,
zeroized, never reaching the WebView.

Crucially, the seam is **not YubiKey-specific.** It is "an age plugin identity whose
unwrap is transport-injected," which generalizes to other hardware keys and other
age plugins, rather than scattering special cases through the engine. Desktop's
subprocess path is one implementation of that seam; Android's in-process path is
another.

**Threat-model impact — unchanged secret flow, one new trusted dependency.** The
YubiKey identity string is a public reference (slot plus public key), not secret, so
storing it under the existing at-rest envelope changes nothing about at-rest
confidentiality — the same conclusion 0030 reached. The PIN is supplied like the
passphrase the app already collects and never reaches the WebView; touch is physical
and needs no UI bridge. The new surface is a bundled, Yubico-maintained SDK for the
USB/NFC transport and the associated permission flows. Reusing Yubico's own SDK keeps
the trust boundary identical to what the `age` CLI, gopass, and the Yubico
Authenticator app already assume — the user trusts the hardware-access library they
ship with — and no secret crosses into the WebView; only the age file key crosses the
Kotlin→Rust boundary, exactly as in the existing plugin path. NFC tag loss is a
reliability and UX concern, not a security one: a partial decrypt cannot corrupt the
store (writes are local-then-sync), and a failed operation is simply retried.

## Alternatives considered

- **Bundle or exec `age-plugin-yubikey` on Android.** Rejected (0030 already):
  Android cannot exec bundled binaries, and a bundled binary still could not reach
  the key without the platform USB/NFC APIs.
- **Reuse the upstream Rust YubiKey crate on Android.** Rejected: it is PC/SC-bound
  with no Android transport, no fork swaps the transport, and relying on it would
  mean vendoring PC/SC-lite into an Android app — strictly worse than reusing
  Yubico's Kotlin SDK.
- **Reimplement the PIV operation and stanza format from scratch.** Rejected as the
  default: the stanza derivation is separable and transport-agnostic, so
  reimplementing it only adds format-interoperability risk against the existing
  recipient/identity encodings. Kept as a fallback only if reuse proves impractical
  on closer inspection.
- **Do the entire decrypt in Kotlin, moving the age-stanza logic out of Rust.**
  Rejected: it would split one crypto path across two languages and double the
  divergence risk on the shared store format; the Rust engine must own the crypto.
- **Push a transport hook upstream into the age library's plugin protocol.**
  Rejected: the stdio/subprocess protocol is the wrong layer, it would require
  changes upstream in the age crate, and it still could not represent the Android
  transport. gpm's own injected-transport seam is simpler and fully owned here.

## Effort

Moderate, conditional on reuse — this **revises 0030's "large project" estimate**,
which assumed more from-scratch work. The genuinely hard pieces (PIV key agreement,
USB CCID framing, and the age stanza derivation) already exist, in Yubico's SDK and
in age-plugin-yubikey's pure-software core. What gpm actually writes is: the native
plugin glue (the USB permission flow, the NFC reader-mode lifecycle, and the
Kotlin↔Rust key-exchange bridge), the identity-model reshape (cached bytes → per-op
handle), the PIN/touch UX, and the frontend identity-type changes. Three things
could push it back toward large: supporting **both** stanza formats (the HKDF
original and the newer HPKE variant) rather than just one; **NFC tag-loss UX**
polish (re-tap prompts, per-device timeout tuning — iterative, not blocking); and
the **identity-abstraction reshape**, which collides with 0036's planned reshape and
should be sequenced alongside it rather than done twice.

## Depends on / Supersedes

Depends on `0030-age-plugin-yubikey` — this is the Android follow-on 0030 explicitly
deferred; the recipient recognition and the desktop decrypt path it ships are
prerequisites, and the honest Android "not available" error 0030 added becomes the
real implementation here. Relates to `0036-gpg-crypto-backend` and the crypto-backend
abstraction it exercises: both record that the identity/backend abstraction will be
reshaped when a second identity model arrives, and the hardware-identity seam lands
in that same reshape, not as a one-off. Relates to the biometric/secure-keystore
plugins as the architectural precedent for the in-process native-plugin plus
injected-secret pattern this design reuses.

## Implementation reference

Concrete technical detail the design above depends on, recorded here so it need not
be re-derived when the implementation lands.

**PIV operation and APDU sequence (one decrypt).** Both stanza formats funnel into a
single on-device operation — PIV key agreement (ECDH) on the P-256 key in a retired
slot. The sequence runs over whichever transport is open (USB CCID or NFC IsoDep):

1. **SELECT PIV applet** — AID `A0 00 00 03 08 00 00 10 00 01 00`.
2. **VERIFY PIN** — CLA `00`, INS `20`, P2 `80`, Lc `08`, then the PIN right-padded
   to 8 bytes with `FF`. (PIN reference `80` is the PIV PIN; `81` is the PUK, which
   only unblocks.)
3. **GENERAL AUTHENTICATE (key agreement)** — CLA `00`, INS `87`, P1 = algorithm
   `11` (ECC), P2 = slot (`82`–`95`, the retired slots). The data is a `7C`
   constructed TLV wrapping an empty response placeholder (`82 00`) and tag `85`
   carrying the sender's 65-byte uncompressed ephemeral point (`04 ‖ X ‖ Y`). Tag
   `85` (not `81`) selects ECDH rather than signing. The response
   `7C … 82 … <32-byte shared secret>` is the ECDH X-coordinate, which HKDF/HPKE
   then turns into the file key. Everything fits short APDUs; no extended-length
   support required.

The **management key** (slot `9B`) is only for generating/replacing keys and writing
the slot certificate — it is never touched on the decrypt path. The **PUK** only
unblocks a blocked PIN.

**The reusable seam.** Upstream `age-plugin-yubikey` separates cleanly: its stanza
logic (the HKDF legacy path and the HPKE path) is pure software, and the unwrap
reaches the device through a single call — "given the 65-byte ephemeral point, return
the 32-byte shared secret." The PC/SC transport and the stdio plugin state machine are
independent of that. So the reuse plan is: vendor the pure-software stanza logic, and
replace the single device call with an injected transport. No stdio protocol, no
`age-plugin-*` subprocess, on Android.

**The Android carrier (Yubico's YubiKit).** `yubikit-android` already implements both
the transport and the PIV operation, so gpm reuses rather than rebuilds:

- Transport seam: `SmartCardConnection` — a one-method interface,
  `sendAndReceive(byte[] apdu) → byte[]`. Two implementations:
  `UsbSmartCardConnection` (claims the USB CCID interface and speaks the USB-CCID
  `PC_to_RDR_XfrBlock` bulk protocol itself) and `NfcSmartCardConnection` (literally
  `IsoDep.transceive`). This is the transport trait to mirror on the Rust side.
- PIV: `PivSession` over a `SmartCardConnection` already performs the key agreement —
  `PivSession.calculateSecret(Slot, PublicKeyValues)` returns the ECDH X-coordinate
  (built on INS `87`; handles ECCP256/P384/X25519), and since YubiKit 2.1 a JCA
  provider exposes it as `KeyAgreement.getInstance("ECDH")` with a PIV-backed key. So
  gpm need not assemble PIV APDUs by hand — it calls key agreement and feeds the
  shared secret into the vendored stanza logic.

This rides the same Rust↔Kotlin plugin bridge the biometric/secure-keystore plugins
already use; the shared secret (an age file-key input, like the existing plugin path)
crosses Kotlin→Rust as bytes and is zeroized, never reaching the WebView.

**Two stanza formats (both must be supported).**

- Legacy `piv-p256` — recipient `age1yubikey1…`, HKDF-SHA256 over the shared secret
  (label `piv-p256`) then AEAD.
- Newer default `p256tag` — recipient `age1tag1…` (plugin ≥0.6.0), HPKE per RFC 9180,
  KEM `DhP256HKDFSha256`, salt `age-encryption.org/p256tag`.

The bech32 HRP differs (`age1yubikey` vs `age1tag`), but gpm's existing recipient
classification (any `age1<plugin>1…`) already tags both as plugin, so recognition needs
no change; only the in-process unwrap must dispatch on the format.

**NFC constraints.** The phone is the reader and the key is an ISO-14443-4 card; the
whole SELECT / VERIFY / GENERAL-AUTHENTICATE sequence must finish in one tap.
`IsoDep.getMaxTransceiveLength()` is ~253–261 bytes (short APDU), so the 65-byte point
fits. `IsoDep.setTimeout(5000+)` is required — the default transceive timeout is too
short for PIV crypto. Removing the key throws `TagLostException` and the sequence
restarts. Yubico's own `NfcYubiKeyDevice` polls `IsoDep.isConnected()` roughly every
250 ms as the "held in field / wait for removal" gate, and a single PIV operation can
need up to ~5 s of sustained contact. NFC is the cable-free default; USB-OTG is the
durable, fast fallback.

**Why JNI-to-YubiKit is the only Rust-side path.** The upstream Rust `yubikey` crate
hard-depends on `pcsc` (not optional); Android has no pcscd, and no fork swaps the
transport. The pure-Rust USB crates do not help either: `nusb` targets Linux usbfs
(blocked by the Android app sandbox) and `rusb` would require bundling libusb. There is
no Rust CCID host stack (≈an 80-page spec to write). So the CCID framing stays in
Kotlin (Yubico already wrote it) and raw APDUs cross over JNI — `jni`/jni-rs supports
Android without the `invocation` feature, since ART owns the runtime. Building a Rust
CCID host stack instead is the alternative that would push effort back toward large.

**Policies and supported hardware.** At keygen the plugin defaults to touch policy
`Always` and PIN policy `Once` (touch has a ~15 s firmware cache under `Cached`
policy). PIN caching is PIV-applet session state, preserved by not soft-resetting the
card after a read-only op (`LeaveCard`); the session ends on unplug or on switching
applets (e.g. touching FIDO2). Over NFC a session lasts only one tap, so the PIN is
effectively re-entered per operation. Hardware: only YubiKey 4 and 5 series (they have
the PIV applet); the NEO series and the blue "Security Key by Yubico" lack PIV and
cannot be used.

**Precedents.** Yubico Authenticator for Android (`com.yubico.yubioath`, open source on
F-Droid) does its USB-CCID and NFC IsoDep I/O in-process — the production reference. No
age client decrypts with an in-app YubiKey on Android today: `gopasspw/gopass-android`
does not exist, and `age-plugin-yubikey`'s Android request (open in its issue tracker
since early 2023) is still blocked on PC/SC. gpm would be among the first.
