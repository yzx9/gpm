# Cancellable saves — push-phase cancellation + the per-save Cancel button

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Make a save fully cancellable, both halves: plumb the existing cancel token
into the push phase (today only clone and pull honour it), and give the save
button a Cancel affordance that mirrors the manual-Sync button. Together these
close the "stuck save" window the decoupling backend increment left open.

## Why

After git sync moved out of the write primitive into the per-save orchestrator
(pull → write → push, gated by the per-device autosync toggle), every save now
holds the Store-wide serialization lock across a network fetch **and** push. The
pull half is cancellable (the manual-Sync infra is reused), but the push half is
not — `git`'s push has no cancel plumbing. So a save whose push hangs on a slow
or dropping remote wedges the lock until the transport timeout, with no user
escape: no further save, edit, delete, manual Sync, or divergence resolve can
proceed, and there is no per-save Cancel button to abort it.

This is an availability regression on the most common action (saving a secret)
relative to the manual-Sync flow, which has always been cancellable. The fix is
small and reuses the existing cancellation primitive.

## Context

The cancellation mechanism is a single boolean cancel token the git transport
checks inside its progress callback; flipping it makes the in-progress transfer
abort. Pull already wires this (the manual-Sync command arms a token and the
fetch's progress callback polls it). Push runs the same kind of network transfer
and accepts the same progress-callback hook, so the cancellation path generalizes
directly — it is plumbing, not new design.

The save-button affordance is the UI half. The manual-Sync button already toggles
between "Sync" and "Cancel" while a pull runs; the save flows (create, edit,
delete) need the same toggle while the orchestrator runs, plus the error path for
a cancelled save (treat it like the user backed out — the local commit, if any,
stays and syncs on the next manual Sync; nothing is lost).

Two ownership bugs in the current single global "active cancel token" slot that
this RFC must fix (they are latent today only because there is no per-save
Cancel button yet, and the serialization lock prevents any data-corruption
vector — the worst case today is a Cancel hitting the wrong op's _pull_):

1. **Stomp / blind disarm.** `run_cancellable` arms the slot _before_ the
   operation acquires the serialization lock, so a second operation that arms
   while the first is queued overwrites the first's token. A subsequent Cancel
   then targets the queued op, not the running one, and the first op's teardown
   blindly clears whatever newer token is now in the slot. Fix: key the slot on
   an operation id (`{op_id, token}`) — arm under the lock right before the
   cancellable fetch, and only clear/cancel when the id matches.

2. **Cancel armed through uncancellable phases.** The orchestrator arms the slot
   for the whole pull → write → push, but only the pull honors the token. A
   Cancel after the pull "succeeds" while the save still commits and pushes — a
   false cancel. Fix: split the save into phases; disarm immediately after the
   pull returns, and do not expose a Cancel for the commit/push phase until push
   itself becomes cancellable.

Threat-model impact: none. Cancellation aborts a network transfer mid-flight; it
does not bypass authenticity verification (a cancelled pull leaves HEAD
unchanged, a cancelled push leaves the local commit unpublished to sync later).

## Alternatives considered

- **Rely on transport timeouts alone (the backend increment's interim choice).**
  Rejected as the long-term answer — a multi-minute timeout freezing every
  mutation on the most common action is a poor escape, and we already built the
  better path (cancellation) for Sync. Acceptable as a deliberate, time-boxed
  interim until this RFC lands; not acceptable as the resting state.
- **Make the orchestrator's push non-blocking (fire-and-forget).** Rejected — an
  unobserved push can't report a rejection (real divergence) or a network
  failure to the save that triggered it, so the user loses the divergence
  surface and the "syncs later" guarantee. The push must be awaited; the only
  safe way to bound a hung await is cancellation.
- **Per-operation cancel slots instead of one global slot.** Considered and
  deferred — the serialization lock already prevents two network ops running at
  once, so a single in-flight op at a time means a single live token suffices,
  provided arming happens inside the lock. A multi-slot model would only matter
  if operations could overlap, which the lock forbids by design.

## Effort

Small. The push-phase plumbing mirrors the existing pull-phase wiring (callback
hook, token poll, abort mapping). The UI half adds a Cancel toggle to the save
flows and a cancelled-save error path, mirroring the Sync button. (human: ~half
a day / CC: ~20 min)

## Depends on / Supersedes

- Follows the decoupling backend increment (the per-save orchestrator +
  autosync) and its `0028` design. The "stuck save" window this RFC closes is
  the one the backend increment deliberately accepted (D5) and recorded as a
  deferral.
- Independent of `PR2c` (the frontend divergence modal / AutoSync toggle) — this
  RFC can land before or after it, though it shares the save-button surface.
