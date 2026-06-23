# No-cache (per-operation) unlock mode

**Priority:** P1
**Status:** Draft
**Phase:** Next

## What

Add a per-operation, no-cache unlock mode: each secret access decrypts only the
one entry it needs and wipes the master identity immediately afterward, instead
of unlocking the store once and keeping the identity cached for the whole
session. Browsing the entry list stays frictionless (it never needs the
identity). Make this the default auto-lock behavior, alongside the existing
idle-timeout and a "never" option, all configurable from Settings.

## Why

Today the app is session-based: one unlock derives and caches the decrypted
identity, and every copy/show/create reuses it until an idle timer (or manual
lock) wipes it. That means the master identity — the key to every secret in the
store — sits decrypted in process memory for minutes at a time. A no-cache mode
holds that key for the briefest possible window — derive it, decrypt exactly one
entry, drop it — shrinking the in-memory exposure of the master key to a single
operation. It tightens only that key's window; plaintext that has already
reached the UI (a copied password, a shown one) is still governed by the
existing view/clipboard auto-clear timers, which this mode leaves alone. The
cost is re-authenticating per operation, which biometric unlock makes tolerable
on mobile (a fingerprint tap per access). Offering it as the default nudges
users toward the stronger posture without removing the friendlier session mode
for those who prefer it.

## Context

The current model couples three things into a single "lock" transition: wiping
the cached identity, raising the unlock overlay, and clearing any secret
currently shown on screen. That coupling is fine for a session model, but it
breaks the no-cache mode's most important case — _showing_ a password. If
viewing a secret and then immediately wiping the identity also raised the
overlay and cleared the screen, the just-revealed password would vanish
instantly and the feature would be useless.

The decision is to **split that transition into two paths**:

- A _hard_ lock (manual lock, idle-timeout expiry) wipes the identity **and**
  raises the overlay and clears revealed secrets — today's behavior, unchanged.
- A _soft_ wipe (the no-cache mode's post-operation step) wipes the identity
  **only**, leaving the overlay down and any currently-revealed secret on
  screen. The secret still clears on its own auto-clear timer; it was already
  delivered to the UI and no longer needs the identity to stay visible. The next
  operation that needs the identity finds the cache empty and re-prompts.

This splits the concept the UI tracks into two: _is the identity cached?_
(drives whether the next operation needs authentication) versus _should the lock
overlay be up?_ (drives the modal). In session mode these are the same; in
no-cache mode they diverge exactly for the duration a secret is being viewed.

Browsing is unaffected because listing entries only reads file names — no
identity required — so the no-cache default doesn't make the app prompt on every
scroll.

Threat-model impact is the point: the window during which a decrypted identity
is recoverable from a memory snapshot shrinks from "the whole unlocked session"
to "one operation." An attacker who grabs a process dump between operations gets
nothing. The at-rest encryption story is unchanged (identity and config stay
sealed on disk as today); this only tightens the in-memory exposure.

**Upgrade impact:** existing users on the session default move to per-operation
re-authentication on upgrade. That's a noticeable friction change and must be
called out in the release notes, with the one-tap path back to a session-style
idle timeout.

## Alternatives considered

- **A near-zero idle timeout.** Rejected: an idle timer of ~0 races the unlock —
  it fires on the next scheduler yield, before the user can act, locking the
  instant they unlock or non-deterministically. It can't express "wipe after
  this one operation," only "wipe after this much idle," which is the wrong
  abstraction.
- **A short idle timeout (e.g. 30s) as the "strong" default.** Rejected: it
  still caches the identity for that whole window on every unlock — exactly the
  exposure the no-cache mode exists to eliminate. A dial on the existing model,
  not the model change being asked for.
- **Keep the session model; only make the timeout configurable.** Rejected as
  incomplete: it satisfies "configurable auto-lock" but not "don't cache the
  identity." The no-cache mode is the user-facing point.
- **For the _show_ case, keep the identity cached until the view-clear timer
  instead of soft-wiping.** Noted as a **fallback** if the hard/soft split proves
  too costly: it sacrifices "wipe immediately after show" (the identity lingers
  for the view window) but avoids splitting the lock transition. The preferred
  design is the split; this is the cheaper retreat.
- **Drop the _show_ action entirely in no-cache mode (copy-only).** Rejected:
  viewing a credential is legitimate and the soft-wipe path makes it work
  cleanly; removing it needlessly cripples the mode.

## Effort

~2 days (human) / ~moderate (CC). Most of the cost is the UI-side split of
"identity cached" from "lock overlay up," and re-plumbing each secret operation
through an ensure-authenticated → act → soft-wipe flow. The backend wipe
primitive already exists; the timer/config plumbing is shared with the simpler
configurable-timeout work.

## Depends on / Supersedes

Depends on the biometric unlock path (to keep per-operation re-auth bearable on
mobile) and on the configurable auto-lock timeout plumbing (which carries this
mode as one option). Composes with the existing at-rest encryption and
clipboard/view auto-clear behaviors; does not change them.
