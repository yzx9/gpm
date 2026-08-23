# Rotate git credentials without re-setup

**Priority:** P2
**Status:** Deprecated
**Phase:** Future
**Revision:** 2

> **Deprecated — superseded by `R098-vault-connection-editing`.** The premise went stale after
> this RFC was written: the PAT slice — the headline case — shipped with the PAT management page
> (probe-then-swap exactly as specified here); the repo-URL slice was deferred to
> multi-repository, which now exists as `R080` and expresses "different repository" as
> remove + re-add (same-repo transport changes land in R098); and the identity slice had already
> moved to `R005`. The one part left unimplemented — SSH key + passphrase rotation in place — is
> re-scoped, in the multi-repository frame, in R098. The analysis below is retained as the
> record; its "only lever today" description predates the PAT page.

## What

Let a configured user rotate the git auth credential used to sync their password store — the HTTPS PAT, or the SSH private key and its optional passphrase — in place, without clearing the local repo, re-cloning, or re-entering the age identity. This is the credential-rotation slice of the broader "re-configuration" idea; the other two slices (repo URL, age identity) are deferred to other work (see Context).

## Why

Credential rotation is routine security hygiene — a PAT expires or is revoked, an SSH key is rotated — yet today the only lever a configured user has is the destructive "Reset All Data": wipe the local store and identity, then re-run the full setup (re-enter URL + new credential + identity, and re-clone the entire password store). That is wildly disproportionate to changing a single token. Rotating just the credential should be a one-field edit that re-writes the stored repo auth and leaves the clone, the identity, and all other config untouched.

## Context

The git auth credential (PAT, or SSH key + passphrase) lives in the repo-scoped config alongside the repo URL and the local clone path. The setup flow writes it once at clone time; there is no path to update it afterward, so any change routes through the full reset. The clone and the age identity are both independent of the auth credential — a credential change needs neither a fresh clone nor any identity work — so an in-place update is safe and self-contained.

Before committing a new credential, probe it against the existing remote (a probe fetch / ls-remote) so a typo'd or revoked token is rejected up front rather than failing on the next sync; a failed probe leaves the prior credential intact (atomic swap). The natural UI home is the existing repository settings view, where the repo URL and current auth method are already shown — the credential becomes editable there.

### Scope deferred to other work

- **Repo URL change + re-clone.** A different URL is a different repository, so a fresh clone is inherent to the operation. "Point at a different repo" is better served by the planned multi-repository feature, which will own repo lifecycle wholesale, so URL re-pointing is deferred there rather than built as a one-off here. (No multi-repo RFC yet.)
- **Age identity change.** Swapping the decryption identity is only meaningful once gpm has identity _management_ — and that belongs with multi-recipient support (R005), where identities become first-class (add / remove / label). A single-identity swap here would be throwaway work subsumed by R005.

## Alternatives considered

- **Keep funneling everything through full reset + re-setup.** Rejected — it is the exact friction this RFC exists to remove, and it needlessly re-downloads the whole store and re-enters the identity for a one-field credential change.
- **A general "edit any repo-config field" control.** Rejected for now — URL and identity edits each carry their own concerns (re-clone; identity management) that are handled elsewhere, so a broad editor would pull in deferred scope. A credential-only control is the slice that stands cleanly on its own today.

## Effort

~0.5 day (human) / ~15 min (CC)

## Depends on / Supersedes

None — independent of other RFCs. R005 previously depended on this one to establish the identity type system first; with identity work moved entirely to R005 that rationale no longer holds, so the two are now independent.
