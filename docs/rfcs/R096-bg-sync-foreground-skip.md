# Background sync: skip while the app is in the foreground

**Priority:** P3
**Status:** Draft
**Phase:** Future
**Revision:** 1

## What

The periodic background sync (pull-only, Android WorkManager) should skip its
run when the foreground app is active, instead of always executing. The
foreground already syncs on cold start and resume while AutoSync is on, so a
worker firing mid-session duplicates that work for no benefit — it burns
battery, wakes the network for a redundant pull, and competes for the repo
lock with user-initiated operations. Serves `docs/specs/005-git-storage`
(periodic background sync).

## Why

Today the worker's only gates are: no key, disabled, repo not ready, AutoSync
off. None of them knows whether the foreground app is already running and
syncing. When the app sits in the foreground across a periodic tick, the
worker still fires and pulls — a redundant network round the foreground will
repeat on its next resume anyway, plus a avoidable lock-contention window
(`REPO_BUSY`) against a foreground save or sync happening at that moment.

**Scope guard:** this is a scheduling/battery optimization, not a correctness
mechanism. Correctness against concurrent config writers is carried by the
`repo.json` write-lock (#57), which holds regardless of who is running. The
foreground-detection heuristic may have edge cases (a foreground in teardown,
a race between detection and execution); those degrade to "the worker runs
anyway" — the pre-optimization behavior — which is safe.

## Context

The detection should use the OS's own notion of foreground (process
importance / lifecycle state), not an app-level heartbeat — the worker runs
in a separate process from the UI, so an in-memory flag is invisible to it.
On Android the natural signal is the process-importance query available to
the worker, with the existing gate sequence unchanged otherwise. Desktop has
no worker, so this is Android-only by construction.

Failure mode is deliberately one-directional: when in doubt, run. A false
"background" verdict costs nothing; a false "foreground" verdict only skips a
pull the foreground is expected to cover.

## Alternatives considered

- **Do nothing.** The redundancy is harmless functionally; this is purely a
  battery/network nicety. Acceptable to keep deferring.
- **Cancel the worker while foregrounded, reschedule on background.** More
  moving parts in scheduling state for the same effect; a per-run skip check
  is idempotent and stateless.
- **Rely on this to fix cross-process config races (#57).** Rejected: a
  lifecycle heuristic cannot carry correctness (check-then-act races,
  desktop multi-instance, future platforms). Explicitly out of scope.

## Effort

~S (human) / ~XS (CC)

## Depends on / Supersedes

None. (#57's write-lock is an independent, complementary fix.)
