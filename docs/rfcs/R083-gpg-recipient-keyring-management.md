# GPG recipient / keyring management

**Priority:** P2
**Status:** Draft
**Phase:** Future

## What

Manage the recipient set of an existing GPG/OpenPGP store from gpm — add and remove recipients, edit the `.gpg-id` index, and manage the `.public-keys/` pool — mirroring `gopass recipients add/remove`. This is the multi-recipient / team scenario that the single-recipient create (R082) and open-existing (shipped) flows do not touch. Serves `docs/specs/004-encryption-gpg`.

## Why

A GPG store's recipient set defines who can decrypt it. A team rotating membership — onboarding a teammate, revoking a leaver — needs to add/remove recipients and re-encrypt the store's secrets accordingly; gopass does this with `gopass recipients add/remove`, and gpm must interoperate with the result. Today gpm uses a store exactly as cloned but cannot change its recipient set, so any membership change forces a trip back to the gopass CLI. This RFC closes that, with gopass's re-encryption semantics as the compatibility bar.

## Context

gopass's recipient model is the contract: `.gpg-id` lists recipient tokens (the `0x`+16-hex long key id, or a full fingerprint), one per line, and `.public-keys/<verbatim-token>` holds each recipient's armored public key. Membership at encrypt time resolves each token through its `.public-keys/` entry by primary fingerprint — the same resolution the shipped backend and membership gate use.

- **Adding a recipient** means: append their token to `.gpg-id`, write their armored pubkey to `.public-keys/<token>`, and re-encrypt every secret so the new member can read existing entries. Re-encryption is the load-bearing, expensive part: decrypt each secret with the current identity, re-encrypt to the new full recipient set, then commit and push.
- **Removing a recipient** means: drop their token, delete their pubkey, and re-encrypt to the reduced set — the only honest way to revoke access, since already-pushed history remains decryptable by the removed key (recoverable in git, surfaced as a limitation).
- **Where a recipient's pubkey comes from**: import via file-pick (the realistic path), with keyserver lookup deferred.

A recipient could itself be a hardware-key identity (an age-plugin-yubikey recipient, or an OpenPGP-card key). Those are recognized but, on Android, not decryptable in-app (R030/R043); the management flow should still accept the recipient's _public_ key for encryption regardless, since encrypting-to does not need the secret.

## Alternatives considered

- **Lazy re-encryption (add the recipient, re-encrypt each secret on its next write).** Rejected: it leaves existing secrets undecryptable by the new member until they happen to be rewritten — a silent inconsistency a team discovers at the worst time. gopass re-encrypts eagerly on add/remove; gpm must match that to stay gopass-compatible.
- **Re-encryption only on an explicit "rotate" command.** Rejected as the default for the same reason, though it may surface as an option for very large stores where the eager re-encrypt is too slow.
- **Keyserver lookup for recipient pubkeys.** Deferred: file-pick import covers the realistic team path (a teammate hands you their pubkey); keyserver integration is a separate concern.

## Effort

~medium-large (human ~3-4 days / CC ~40 min): a recipients-management screen, the add/remove commands, and the eager re-encryption orchestration (decrypt-all → re-encrypt-to-new-set → commit → push) with its conflict/autosync interactions, plus gopass-interop tests.

## Depends on / Supersedes

Depends on the shipped GPG backend (spec 004, A006) and the open-existing flow. Serves `docs/specs/004-encryption-gpg`. Relates to R082 (single-recipient create) and R030/R043 (hardware-key recipients).
