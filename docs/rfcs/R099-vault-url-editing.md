# Edit a vault's remote URL in place — same repository, different transport

**Priority:** P2 **Status:** Draft **Phase:** Next **Revision:** 1

## What

Let a configured user edit an existing repository's remote URL in place when it is the same
repository reached differently — an HTTPS → SSH transport switch on the same host, or a host
migration. Pointing the vault at a _different_ repository is not an edit — under multi-repository
that is remove + re-add (a fresh clone is inherent to a different repository). Carries the
URL-editing scope previously tracked in `R098`, deleted once its SSH credential-rotation slice
shipped.

Serves `docs/specs/005-git-storage` (the repo-connection lifecycle: reconfigure).

## Why

Under multi-repository (`R080`) the URL question is reachable and needs an owner: "same repository,
different URL" is an in-place edit — history is unchanged, only the transport config moves — while
"different repository" is remove + re-add. No document currently owns either half. Transport
switches and host migrations are routine (moving off token auth, a provider change), and today the
only path is remove-vault + re-add, which destroys the local clone (full re-download on mobile), the
sealed identity, and the authenticity trust-set for a change that touches none of them.

## Context

**Two sources of truth for the remote.** Sync and push resolve `origin` from the clone's git config
(`find_remote("origin")` on the pull and push paths), not from the sealed `repo.json` URL — that
field is display and derivation state. An edit that rewrites only the config URL leaves the settings
card and the actual sync target pointing at different hosts. The edit must write both — the sealed
config field and the clone's remote — as one change.

**`remote_add` is not enough.** The existing remote-recording primitive (`repo.remote(name, url)`)
is setup-shaped: it creates `origin` on a fresh repository and errors when a remote of that name
already exists. In the edit case `origin` already exists, so a set-url (or delete + re-add)
primitive is needed at the storage layer — small, but new.

**Probe after the swap.** A transport switch usually rewrites the credential alongside the URL
(HTTPS+PAT → SSH+key). The same read-only probe that validates a candidate credential (a fetch into
a throwaway ref, nothing checked out, cancellable) proves the new URL + credential pair before
anything is persisted — probe-then-swap, extended from credential to credential + URL. Credential
rotation itself (PAT, SSH key + passphrase) has already shipped; this RFC only pairs the URL with
it.

**Division of labor.** Adding the _first_ remote to a local-only vault stays in `R071` (its first
publish must be push-only, and the pull phase of a sync would error on the not-yet-existing branch);
this RFC owns the URL half of an existing connection. Removing the remote entirely (back to
local-only) is not in scope — no user pull has surfaced, and re-adding goes through `R071` again.

## Alternatives considered

- **In-place URL edit for a different repository.** Rejected — a different URL is a different
  repository; pretending otherwise drags in a re-clone decision tree and a mid-migration config
  state that the add flow already expresses cleanly as remove + re-add.
- **remove-vault + re-add for the same-repo change.** Rejected — it destroys the local clone, the
  sealed identity, and the trust-set for a change that touches none of them.
- **Rewrite the config URL only.** Rejected — the two-sources-of-truth fact above; display and sync
  would silently diverge.

## Effort

Small–medium. One set-url primitive at the storage layer, the dual write (sealed config + clone
remote) on the load → mutate → save pattern, the probe surface for URL + credential, and an editable
URL on the per-repo connection card. The probe, the mutation pattern, and IPC masking all exist.
(human: ~1–2 days / CC: ~1 session)

## Depends on / Supersedes

- Serves `005-git-storage` (repo-connection lifecycle: reconfigure).
- Builds on `R080-multi-repository` — the per-vault connection card this surface lives in.
- Coordinates with `R071` (the no-remote → has-remote edge of the same card) and with the shipped
  credential-rotation primitives.
- Carries the URL slice of the retired `R004`/`R098`: R004's premise went stale after the PAT page
  shipped; R098 was deleted once its SSH slice shipped in-tree (the numbering gap is expected).
