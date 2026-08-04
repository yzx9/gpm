# Per-secret divergence resolve — an optional entry-by-entry picker

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Add an **optional** per-secret selection mode to the git-divergence resolve
flow. Today a divergence is resolved all-or-nothing — _Keep Mine_ (replay every
local-only and locally-changed secret onto the remote tip) or _Adopt Remote_
(discard every local-only secret and accept every remote change). This RFC adds
a third, opt-in affordance: a picker that lets the user decide, secret by
secret, which side wins for the entries the two branches genuinely disagree on.
The current two-button flow stays the default; the picker is a deliberate
drill-in for users whose divergence is too lopsided for either blanket choice.

Serves `docs/specs/005-git-storage` (the conflict-resolution mechanism); the
conflict _experience_ is shared with `docs/specs/002`.

## Why

The binary choice is correct for the common case — a one- or two-secret
divergence where one side is plainly right — but forces a destructive
compromise everywhere else. A user who diverged on three secrets out of fifty
must either keep all fifty and overwrite the three remote changes, or discard
all local work to accept the three. Neither reflects intent, and both are data
loss in one direction. The cost is exactly the kind of loss divergence resolve
exists to prevent, just at branch granularity instead of per secret.

The ingredients to do better already exist: the resolve preview already
enumerates, by name, which secrets a blanket adopt would lose and which a
blanket keep would overwrite; and the _Keep Mine_ pipeline already moves secrets
through decrypt → re-encrypt → commit one at a time. So a per-secret choice is a
contract and UI change that reuses the existing crypto/sync path — it is not new
cryptography, not a new merge strategy, and not a change to the on-disk format
or gopass compatibility.

## Context

**The current resolve is a flat two-way choice with no payload; "cancel" is
client-side only.** The preview the modal opens with carries three named
buckets — secrets that exist only locally (lost on adopt), secrets that differ
between the two trees (overwritten on remote by a keep), and other changed files
(templates, recipients) — plus how many commits each side is ahead. The frontend
renders these as read-only lists, then two neutral buttons, then a contextual
confirm. There is no per-row state, no selection model.

**Keep Mine is already a per-secret pipeline.** It advances to the reviewed
remote tip, then replays each local-only and locally-changed secret through
decrypt → re-encrypt-to-current-recipients → write → commit, refusing if any
single secret was changed on _both_ sides since the merge base (the same-secret
conflict guard). A "keep only these secrets" mode is therefore a filter on that
pipeline: advance to the remote tip (adopt), then replay only the chosen subset.
The two existing choices become its degenerate cases — _Adopt Remote_ keeps the
empty subset; _Keep Mine_ keeps every local entry.

**The picker makes the same-secret overwrite an explicit user choice.** Today a
same-secret-both-changed conflict is a blanket refusal, because _Keep Mine_
would silently overwrite a remote change to a secret the user cannot see is
contested. In a per-secret picker the user _is_ looking at that secret and
choosing — so an explicit "keep mine" there is a deliberate overwrite, which the
picker must flag ("both you and the remote changed this; keeping yours discards
their change") rather than refuse.

**That flag needs information the preview does not yet carry.** The "differs
between trees" bucket conflates three cases the user must distinguish to choose
honestly: only-you-changed, only-they-changed, and both-changed. The current
buckets are a two-way tree diff, not a base-relative three-way classification;
the both-sides classification exists only inside the resolve step, after the user
has already committed to a choice. A safe picker requires the _preview_ to carry,
per entry, who changed it since the merge base — an enrichment of what crosses
to the frontend, not a change to what is stored. This is the design's load-bearing
open question (below).

**Two threat-model invariants the design must keep.** Plaintext never crosses
into the storage/merge layer — the picker chooses between whole encrypted
secrets (your version or theirs), never fields within one, so no secret is
decrypted for the purpose of choosing. And the resolve stays identity-gated
exactly when any local secret is kept (the kept subset needs decrypt →
re-encrypt, as _Keep Mine_ does today); a pure adopt needs no identity, as today.

**Foreground-only.** Per spec 005 / R061, a background sync tick never surfaces
an interactive resolve — divergence becomes a passive badge. The picker is a
foreground affordance behind that same badge; headless sync behavior is
unchanged.

## Alternatives considered

- **Make the picker the default (always show per-secret).** Rejected. Most
  divergences are tiny and the binary flow is the right answer in one tap;
  forcing a picker adds friction to the common case. Worse, a recipient-rotation
  divergence re-encrypts every secret, so the "differs" bucket would list the
  entire store — a picker there is noise, while _Adopt Remote_ is obviously
  correct. Keep the binary default; offer the picker as a drill-in.
- **Field-level three-way merge per secret.** Rejected. gopass secrets are
  opaque encrypted blobs and gpm deliberately never merges blob _contents_ (the
  same-secret refusal exists precisely to avoid this). Field-level merge would
  require decrypting into the merge layer, parsing structure, and re-encrypting —
  blowing the "plaintext never crosses to the untrusted layer" invariant and
  gopass compatibility. The unit of choice stays the whole secret: your version
  or theirs.
- **Ship the picker on today's name-only buckets, fix accuracy later.**
  Considered and rejected as the resting state. Without per-entry "who changed
  it," a user selecting "keep mine" on an entry only the remote changed would
  silently revert their teammate's change — the R026 silent clobber, now
  user-initiated and harder to blame. Acceptable only as a clearly-labeled
  interim; the safe default requires the preview enrichment. (Cross-ref R026,
  which _prevents_ the clobber on a single edit; this RFC is the _resolve_-side
  complement — both are wanted, and neither subsumes the other.)
- **Rely on R026 alone and never offer granular resolve.** Rejected. R026 stops
  a stale edit from clobbering in the first place, but divergence still arises
  from concurrent edits, multi-device work, and offline-then-sync; R026 does not
  resolve a branch that is _already_ divergent. Resolve-time granularity is
  still needed.

## Open question (gates Acceptance)

How does the picker learn, per entry, whether a change is one-sided or
two-sided since the merge base — without making the _preview_ identity-gated?
Today preview is deliberately identity-free (browsing the list needs no unlock).
The accurate classification is a base-relative tree comparison the resolve step
already computes; the question is whether to (a) move that classification into
the preview (richer IPC, still no identity), (b) compute it lazily when the
picker opens (one extra round-trip, still no identity), or (c) accept a
caveat-labeled interim. Resolving this — and confirming the recipient-rotation
false-positive (identical plaintext re-encrypted reads as "differs") is tolerable
or also needs a decrypt-compare — is what moves this from Draft to Accepted.

## Effort

Medium. Backend: one new resolve choice carrying a set of entry names, one new
backend method that filters the existing keep-mine pipeline by that set, and the
per-entry both-sides classification surfaced to the preview — all reusing the
current crypto path and conflict guard. Frontend: a new picker step in the shared
resolve modal (per-row selection, a selection set, back-stack coordination), the
resolve call extended to carry the chosen set, and new copy in both locales —
the modal is reached from five callers across two code paths, so the shared
surface is the bulk of the work. (human: ~2–3 days / CC: ~1 session)

## Depends on / Supersedes

- Serves `005-git-storage` (conflict mechanism); experience shared with `002`.
- Complements `R026` (edit-base-version-aware) — prevent-side vs resolve-side of
  the same clobber.
- Preserves invariants from `R027` (path-bound per-secret history; plaintext
  never crosses to the untrusted layer), the shipped cancel-slot contract (a
  subset resolve stays cancellable under the same lock-scoped token a full sync
  uses), and `R061` (background sync stays non-interactive; the picker is
  foreground-only).
