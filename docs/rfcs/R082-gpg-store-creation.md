# GPG store creation (import one key)

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Create a brand-new GPG/OpenPGP gopass store on device — the create-side counterpart to the shipped open-an-existing-store flow. Scoped narrowly: the user imports a single existing GPG secret key, and that key's recipient seeds the new store. No in-app key generation (R084) and no recipient/keyring management (R083) at create time. Serves `docs/specs/004-encryption-gpg`.

## Why

The open-and-use-an-existing-store flow shipped, so a user with a GPG repo can already use it on their phone. But a user who wants to _start_ a fresh GPG store from gpm has no path — they must create it elsewhere and clone it back. This closes that gap with the smallest honest shape: import your key, seed the store, done. It deliberately mirrors the age create flow's single-identity model rather than front-loading key generation or multi-recipient management.

## Context

The age create flow is the shape to mirror: take one identity, seed the recipients index with it, `git init`, make the "Initialized Store" commit, optionally add a remote. The GPG create flow is the same shape with a different identity model:

- **Identity = import one existing armored secret key** (not a pasted age string, not a generated key). The key is stored S2K-passphrase-locked and AEAD-sealed at rest exactly as the open-existing flow stores it — that machinery is reused.
- **Seed the gopass on-disk markers for a single-recipient store**: the key's gopass recipient id (`0x` + last 16 hex of its primary fingerprint) is written to `.gpg-id`, and its armored public key to `.public-keys/<recipient>`.
- **Backend selection is explicit GPG at create time**, unlike the marker-driven detection on an existing repo — the user is creating a GPG store, so the kind is known.
- **Membership is trivially self**: the sole recipient is the imported key, so the membership gate is a no-op; it only matters once R083 adds more recipients.

A hardware-token key (an OpenPGP-card/YubiKey stub with no usable secret material) is rejected at import where the stub is detectable, with the same fallback error on the decrypt path when it is not — the same handling the open-existing flow established, and the reason this RFC relates to the age-YubiKey work (R030/R043): both are hardware-key identities that cannot live as cached bytes.

Once created, the store behaves identically to an opened one — list, view, copy, create, and sync all run through the store facade.

## Alternatives considered

- **In-app key generation instead of import.** Rejected for this RFC (moved to R084, Blocked): it bundles the cost and concept-onboarding of GPG keygen into the create flow, and a user creating a GPG store almost always already has a key. Import covers the realistic path; R084 records why keygen is deferred.
- **Recipient management folded into create.** Rejected: at create time the store has exactly one recipient (the creator); management only matters once others are added, which is R083. Bundling it would delay the create flow on work that is not yet needed.
- **Paste as well as file-pick for the key.** Rejected (same call as the open-existing flow): a multi-line armored block is error-prone to paste on a phone and routes a full secret key through the UI text field for no strong benefit.

## Effort

~small-medium (human ~1 day / CC ~20 min): the open-existing flow's import + save-identity + seeding machinery already exists; the new work is the create-side branch (explicit-GPG backend selection, seeding `.gpg-id` + `.public-keys/<self>`, the git-init commit) and its tests.

## Depends on / Supersedes

Depends on the shipped open-existing-store flow (in code / spec 004) and A006 (rpgp). Serves `docs/specs/004-encryption-gpg`. Relates to R083 (recipient management), R084 (key generation, Blocked), and R030/R043 (hardware-key identities).
