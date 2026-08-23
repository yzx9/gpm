# Add a remote to a local-only store — the first slice of repo reconfiguration

**Priority:** P2
**Status:** Draft
**Phase:** Next
**Revision:** 1

## What

Let a store that was created (or cloned) without a remote gain one later, from
**Settings → Repository**. Today the repo connection (URL + credentials) is fixed
at setup; a local-only store has no in-product path to add `origin`, so the only
way to start syncing is to wipe and re-clone — which loses any local-only
commits. This RFC adds that path as the first concrete slice of the broader
reconfiguration flow (R004), which today is an unimplemented draft that does not
contemplate the no-remote → remote transition at all.

Serves `docs/specs/005-git-storage` (the repo-connection lifecycle: setup /
reconfigure).

## Why

It closes a promise the UI already makes — the create-flow hint reads "without
[the URL] the store is local-only and **can be synced later**" — and a real
workflow: a user who starts local (Casey) and later adds a second device has no
way to connect the existing store to a remote without destroying it. The sync
engine already gates everything on whether `origin` exists, so adding it simply
re-activates pull / push / divergence with **no new sync logic** — the work is
the connection-management command, sealed-config persistence, and the Settings
surface. Of the spec 005 roadmap items, repo reconfiguration is the one with the
clearest user pull and the smallest blast radius, and adding the _first_ remote
is its most valuable slice.

## Context

**The remote-recording primitive already exists and is local-only** — it records
the `origin` config without contacting the host, and setup already uses it. The
new command reuses it; nothing new is built at the storage layer.

**Config persistence is the first subtlety.** The connection (URL + PAT or SSH
key/passphrase) lives in the sealed repo config alongside the commit identity,
authenticity mode, and backend choice. The setup-time save helper intentionally
_blanks every other field_ — correct at setup (nothing else exists yet), fatal
post-setup (it would wipe the user's commit identity and verification policy).
The new command must therefore use the load → mutate → save path that the
existing commit-identity and verification-mode setters already use, touching only
the connection fields.

**The first-publish path is the load-bearing detail.** After the remote is
recorded, the first publish must be **push-only**, not a pull-then-push sync.
The pull phase of a sync fetches the remote's branch ref, which does not exist
yet on a brand-new empty remote, so the pull errors before the push ever runs.
The create flow already navigates exactly this — its first publish is push-only
by design — and add-remote must mirror it. What happens next depends on the
remote's state, each case already handled by existing primitives:

- **Empty / brand-new remote** (the common local → remote case): the push-only
  publish creates the remote branch as a clean fast-forward. No modal.
- **Remote is an ancestor of local** (e.g. the store was once shared, went
  local-only, and is now reconnected): the push fast-forwards. No modal.
- **Remote has unrelated / divergent history** (e.g. pointing at an existing
  gopass repo): the push is rejected, and the existing divergence resolve modal
  surfaces (Keep Mine / Adopt Remote, and per R067 eventually per-secret). No
  new resolution logic.

**Credential validation is absent today.** There is no "test connection"
operation; credentials are validated only during a real fetch/push's auth
negotiation. Add-remote can mirror the create flow (attempt the push, surface
the auth error) or add a lightweight probe — an open choice, not a blocker.

**Background-sync side effect.** Once `origin` exists, the periodic background
pull (R061) stops silently no-op'ing and starts targeting the remote. The AppLock
gate still holds (the sealed config cannot be read under lock), so the first
background pull runs only post-unlock — but the transition from "never syncs" to
"syncs on a cadence" is a behavior change worth surfacing to the user when they
add the remote.

**Cancellation blind spot (inherited).** A mistyped URL during add-remote
contacts an unverified host, and the first push to it is only best-effort
cancellable — the bulk-upload window has no abort checkpoint until `R034`
lands. This is the same first-run-to-an-unverified-host pain the create flow
already has; add-remote inherits it, it is not new.

**Local-only is not a first-class UI state.** The only signal that a store has
no remote is an empty connection URL; there is no badge, and the manual Sync
button is not gated on having a remote — so a local-only user who pulls to
refresh sees a "synced" timestamp with no indication that both phases silently
no-op'd. Add-remote should at least show an "Add remote" affordance when the URL
is empty, and ideally badge the local-only state so the no-op sync stops looking
like success.

## Alternatives considered

- **Fold into R004 (broader reconfigure) and skip a separate RFC.** R004 is the
  umbrella, but it is Draft / unimplemented and scoped to _changing_ an existing
  remote, not _adding_ the first one to a local-only store; it omits the
  empty-remote first-publish gotcha, which is the load-bearing detail here. A
  focused RFC captures that; R004 remains the umbrella for the later
  change-URL / change-credentials cases, which are strictly larger (a URL change
  forces a re-clone decision; an identity change forces a re-encryption
  decision).
- **Keep "wipe and re-clone" as the only path (status quo).** Rejected — it
  loses local-only commits, and the create flow already proves the pieces
  (record remote + push-only publish) work standalone.
- **Use the full pull → push sync for the first publish.** Rejected for the
  empty-remote case — the pull phase errors before the push runs. First publish
  is push-only, matching the create flow; every sync _after_ the first is a
  normal pull → push.
- **Require a "test connection" probe before committing the config.** Considered
  and deferred — the create flow ships without one and the transport already
  surfaces auth errors on the real push. A probe is a nice-to-have polish, not a
  blocker, and can land later.

## Open question (gates Acceptance)

Credential handling on add: attempt-and-surface (create-flow parity, the config
is committed and the push reports any auth failure) versus an explicit probe
before the config is committed (no config change if the host is unreachable or
credentials fail). And, for case (c) above: when the remote already has
divergent history, should add-remote go straight into the divergence modal, or
warn the user _before_ recording the remote that the chosen remote is not empty
and will need resolution? The latter is friendlier but needs the probe this RFC
otherwise defers.

## Effort

Small–medium. Backend: one new connection-setter command (load → mutate → save
on the sealed config, reusing the existing remote-recording primitive) plus
wiring the push-only first publish. Frontend: make the Repository settings card
editable when the connection URL is empty (reuse the existing setup auth-fields
component), an "Add remote" affordance, and a local-only badge. The genuinely
hard parts — sync gating, divergence resolve, the sealed-config mutation pattern
— are already built. (human: ~1–2 days / CC: ~1 session)

## Depends on / Supersedes

- Serves `005-git-storage` (repo-connection lifecycle). First concrete slice of
  `R004` (reconfiguration flow), which stays the umbrella for change-URL /
  change-credentials.
- Side effect on `R061` (background sync begins targeting the new remote).
- Inherits the remaining cancellation blind spot from `R034` (the bulk-upload
  window of a first push to an unverified host has no abort checkpoint).
- Reuses the push-only first-publish pattern the create flow established.
