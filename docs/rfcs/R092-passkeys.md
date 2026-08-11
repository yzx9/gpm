# Passkey support

**Priority:** P2
**Status:** Blocked
**Phase:** Future

## What

Native passkey support for gpm: store WebAuthn credentials as secrets in the vault, and — in a later phase — serve them to relying parties through Android's Credential Manager so gpm acts as the device's passkey provider. This RFC records the design and the surrounding landscape, and blocks on a single external dependency: gopass has not yet defined how a passkey is stored on disk, and gpm's compatibility constraint means it will not pick that format first.

Serves a future `docs/specs/010-passkeys/` — not written yet, because the design is blocked; this RFC is the record until gopass moves.

## Why

Passkeys are the dominant phishing-resistant replacement for passwords, and a mobile-first password manager is the natural home for them — a gopass maintainer observes that passkeys are "probably best stored on mobile phones," which is exactly the client gpm is. The opportunity is real and the technical path is clear. What is neither clear nor ours to decide is the on-disk representation.

gpm mirrors gopass's secret formats by hard constraint: where gopass defines a concept, gpm reuses it rather than inventing a parallel abstraction. gopass has not yet defined a passkey storage concept. Its only passkey work to date is an in-memory credential type plus the make-credential / get-assertion signing math — groundwork that a gopass reviewer himself flagged as missing the serialization needed to store credentials in secrets, and that still lacks the transport layer to answer browser requests. There is no on-disk format to mirror.

Pioneering a format ahead of gopass would bet gpm's store on a representation gopass may later contradict. This RFC records the analysis so the decision is not re-derived, and so gpm can follow gopass promptly the moment a format exists.

## Context

**The blocker is the storage format, not the crypto or the platform integration.** Everything else is buildable today: signature generation is pure-Rust ES256 (with the WebAuthn-mandated DER signature encoding), and the serving path is Android's Credential Manager provider contract, which deliberately permits software-held, syncable keys and does not require hardware backing. The Credential Manager route is a high-level JSON-in / JSON-out contract; it avoids the CTAP2-virtual-authenticator path that has stalled gopass's own passkey work. What none of this settles is which bytes go in the secret.

**Why gpm waits rather than pioneers.** A passkey's private key is just bytes that could be stored many ways, and gpm is not short of schemas: the FIDO Credential Exchange Format (CXF, ratified 2025) defines a passkey dictionary, and the KeePass `KPEX_PASSKEY_*` attribute convention is a working on-disk schema shared across KeePassXC, Strongbox, and KeePassium. gpm could adopt either today and lead the gopass ecosystem. It deliberately does not. gpm's reason for existing is being a faithful gopass client; a storage format chosen ahead of gopass is a divergence risk — a future migration, and a split in store portability — if gopass later settles on something different. The gopass-compat rule binds gpm to follow gopass's formats, and where gopass has not yet spoken, gpm waits.

**gopass's current state, in one paragraph.** gopass has a single passkey package: an in-memory credential type (P-256, with an incrementing signature counter and WebAuthn level-2 flags — no backup-eligibility or backup-state fields) plus the signing routines for make-credential and get-assertion. The private key cannot be serialized, which is the exact gap that blocks storing credentials in secrets; no CLI, no persistence, no browser or jsonapi wiring, and no CTAP2 transport exist. The maintainer characterizes it as groundwork only. gpm's unblock signal is therefore gopass adding serialization — or documenting a storage convention — to this credential type.

**The honest security posture, for whenever this unblocks.** gpm's git-synced vault is itself a sync fabric: a credential stored in it exists on every synced device by construction, and that fixes much of the eventual design before it is drawn. Every gpm-held passkey must advertise itself as a multi-device (backup-eligible, backup-state) credential, must omit the per-use clone-detection counter — a counter is incoherent across git-pulled copies — and must carry no attestation. This caps the credential at authenticator-assurance-level 2; relying parties that require a hardware-bound credential (some banks, some enterprise) will reject gpm passkeys by policy. This is the same posture as every software passkey provider — the OS keychains, 1Password, Bitwarden — and is not a gpm weakness, but it must be stated rather than hidden when the feature ships. A device-bound, hardware-backed tier would contradict git sync and is out of scope.

**Two phases, both gated on storage.** Phase 1 stores, views, and imports/exports passkeys as secrets, following the same pattern gpm already uses for TOTP: a structured credential detected from the secret's attribute region, derived and used in Rust without the secret crossing to the WebView, with a presence flag surfaced as a byproduct of each decrypt. Phase 2 registers gpm as an Android Credential Manager provider that fulfills registration and assertion requests from the stored credentials, building on the same app-owned Android service foundation as the autofill work. Neither phase can start until the storage format is settled.

## Alternatives considered

- **Pioneer a CXF- or KeePass-style on-disk format now.** Rejected. Both schemas exist and would let gpm lead the ecosystem, but either bets the vault on a representation gopass may not adopt — risking a later migration and split store portability, directly against the gopass-compat constraint that makes gpm worth using.
- **Ship passkeys as gpm-only, ignoring gopass compat.** Rejected. It abandons the shared-store / gopass-interchange model that is gpm's reason for existing.
- **Build only the Credential Manager provider now, deferring the storage format.** Rejected. The provider reads and writes passkey secrets, so it is gated on the same format decision; and it is gated behind the autofill foundation regardless.
- **Defer until gopass defines its passkey storage format.** Chosen. The landscape is recorded here so the eventual work need not re-derive it; gpm follows gopass promptly once a format lands.

## Effort

Not started; blocked on gopass. When unblocked: Phase 1 (storage + CXF interchange + UI, following the TOTP pattern) is Medium; Phase 2 (the Android Credential Manager provider) is Large and additionally gated on the autofill service. (human: ~3–5 days for Phase 1, ~2–3 weeks for Phase 2 / CC: ~2–3 sessions, then several more).

## Depends on / Supersedes

- Blocked on **gopass defining a passkey storage format** — track its passkey package and the reviewer-flagged serialization gap; reassess when gopass adds serialization to its credential type or documents a storage convention. The likely convergence target then is **FIDO CXF** plus the existing KeePass attribute convention.
- Phase 2 depends on the **Android autofill** foundation (**R056** / spec `008-android-autofill`) — passkeys are the "later layer over the same service" that RFC anticipated.
- Builds on the **structured-credential pattern** gpm already uses for TOTP, and on the **AKV secret format**.
- Foundational: **A001** (the Tauri/Rust/age stack) and **A002** (rust-first, but following gopass's formats rather than reimplementing gopass).
