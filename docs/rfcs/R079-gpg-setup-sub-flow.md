# GPG setup sub-flow (open an existing gopass-GPG store)

**Priority:** P1
**Status:** Accepted
**Phase:** Now

## What

The setup sub-flow that lets a user open an existing gopass-GPG store on a
device: clone the repo, import an existing GPG secret key, and complete setup so
the store is usable for read and write. This is the v1 cut of GPG (spec 004 scope
"A"): open-and-use an existing store — no in-app key generation, no
recipient-keyring management. Serves `docs/specs/004-encryption-gpg`; builds on
the GPG backend (R036), which is implemented and wired through the store for read
and write but currently unreachable from setup.

## Why

The GPG backend works — read and write route through it once the store config
selects GPG, and it round-trips against system gopass — but setup never selects
it: today setup always configures the age backend and explicitly rejects a picked
OpenPGP secret key. So no user can reach a working GPG store. Jordan (the
gopass-GPG persona) wants to open an existing store on the phone, and this
sub-flow is the missing door. It deliberately narrows v1 to "open and use an
existing store," deferring key generation and recipient-keyring management to
later phases.

## Context

The sub-flow mirrors the existing age setup shape (connect repo → provide
identity → verify → complete), branching to GPG where the identity model
differs. The agreed design:

- **Backend selection by auto-detection, not a selector.** After the repo is
  cloned, the store kind is read from the authoritative gopass on-disk marker —
  a `.gpg-id` index means GPG, an `.age-recipients` index means age — and
  persisted as the store's crypto backend. The detected kind is shown to the user
  as a confirmable fact, not a question. An explicit age/GPG selector is deferred
  until a create-from-scratch flow needs it; every v1 entry is an existing repo,
  so the marker is always present.

- **Identity by file import, not paste.** The user supplies an existing armored
  OpenPGP secret key through the file picker, which reads the bytes directly into
  the backend without routing them through the WebView. Paste is deferred: an
  armored block is unwieldy to paste on a phone and would route a full secret key
  through the UI text field for no strong benefit.

- **Membership check before passphrase.** A key's fingerprint and user id are
  public-packet data, so the flow checks whether the imported key is a recipient
  of this store (resolving each `.gpg-id` token through the matching
  `.public-keys` entry by fingerprint) _before_ asking for the key's passphrase.
  A non-member key is hard-blocked with an actionable message — importing one has
  no legitimate meaning in the v1 use case — and if the check itself cannot run
  (the recipient public-key pool is incomplete), that surfaces as a distinct
  "repo looks incomplete" error rather than a silent allow. Only after membership
  passes is the passphrase requested, with the key's user id and fingerprint
  shown first so the user confirms which key they are unlocking. A hardware-token
  (OpenPGP-card / YubiKey) key, exported as a stub with no usable secret
  material, is rejected here where the stub is detectable, falling back to the
  same clear error on the decrypt path when it is not (R036).

- **Sealing and re-lock reuse the existing machinery.** The S2K passphrase is
  sealed through the biometric-keystore plugin and re-fetched on re-unlock under
  biometrics; the unlocked key is AEAD-sealed at rest with the existing master
  key, the same protection the age identity gets; and the same Immediate / Idle /
  Never auto-lock lifecycle that governs the age identity governs the GPG
  identity. No new durable-secret handling.

- **Transparent main flow; one quiet setting.** Once unlocked, the entry list,
  secret view, copy, create, and sync are identical to age — the crypto backend
  sits behind the store facade — so v1 needs no main-UI changes. The only
  persistent surface is a single "backend: GPG" line on the repository settings
  page, the durable landing point for the detected-kind fact shown during setup.

The threat model is unchanged from R036 / spec 004's security model: plaintext
never reaches the WebView, decrypted content is zeroized and wiped, and rpgp
stays panic-isolated on attacker-controlled input.

## Alternatives considered

1. **Explicit age/GPG selector at setup.** Rejected for v1: every entry is an
   existing repo carrying its authoritative marker, so the kind is inferable with
   no extra user decision; a selector only becomes forced by a create-from-
   scratch flow (future). Surfacing it now would add a step even for age users.

2. **Paste as well as file-pick for the key.** Rejected for v1: a multi-line
   armored block is error-prone to paste on a phone and routes a full secret key
   through the UI; file-pick covers the realistic import path. Paste can be added
   later for parity with age.

3. **Warn-but-allow on a non-member key.** Rejected: in the v1 scope (open and
   use an existing store) importing a non-member key has no legitimate meaning —
   read fails, and write only half-works. A hard block with an actionable message
   is the honest, mobile-friendly behavior; the only reason to allow would be a
   check false-negative, better fixed by making the check authoritative than by
   weakening policy.

4. **Passphrase before membership check.** Rejected: it would waste a passphrase
   entry on a wrong key and forgo the chance to show the key's identity before
   asking for its secret.

5. **Detect sub-tree `.gpg-id` partitioning and restrict those stores now.**
   Rejected for v1: a niche gopass feature; deferred to R078, which records the
   write-mis-routing gap and the read-only mitigation to reach for first.

6. **Bundle key generation and recipient-keyring management into v1.** Rejected
   by the phasing decision: key generation serves a user starting fresh rather
   than opening an existing store, and recipient management only matters when
   adding new recipients — both are independently valuable and deferred so the
   open-existing-store path ships first and validated.

## Effort

~medium: the backend is already done; the work is the setup command layer
(clone-time detection and backend persistence, an accepted GPG identity path
replacing today's rejection, a GPG verify step, and a GPG complete branch), the
membership-check and card-stub detection helpers, the frontend setup branch, the
settings-page line, and tests — notably a write-side store integration test,
which spec 004 flags as the remaining coverage gap.

## Depends on / Supersedes

Depends on R036 (GPG crypto backend — implemented and wired through the store).
Relates to R078 (sub-directory recipients, deferred). Serves
`docs/specs/004-encryption-gpg`.
