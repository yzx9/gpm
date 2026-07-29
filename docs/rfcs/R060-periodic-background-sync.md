# Periodic background sync

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

A low-frequency background timer that periodically pull+pushes the local store, so a device converges with the remote without the user having to trigger a Sync. Mirrors gopass's 3-day `autoSync`: a best-effort, opportunistic reconciliation that runs even when the app is idle or when per-write autosync is off.

## Why

Today sync is either per-write (AutoSync on — every save pull-write-pushes) or on-demand (the Sync button). A device that sits idle, or one a user only **reads** from, never publishes its local commits and never picks up remote changes until the user acts. For a single active device that is fine; for a multi-device household or a team store it means stale views and surprising divergences later. A background timer catches these up the way gopass already does.

Low priority: per-write + manual Sync covers the current single-user need. This is a convergence/UX improvement, not a correctness gap — divergences are already resolved by the existing sync-time resolve flow.

## Context

- **Building block exists.** The manual Sync path (pull → push, with a push-rejection/divergence surfacing the resolve modal) is the same operation a background tick would run. A periodic sync reuses it; it invents no new sync logic.
- **Serialization.** All repo-mutating work shares one critical section, so a background tick can't race an in-flight write or a manual Sync — it queues behind (or skips if one is already running). The design question is whether a background tick should **skip** when a sync is already in flight (preferred — avoid pile-up) vs. queue.
- **Android background limits.** The OS restricts background work; a reliable periodic sync likely needs the platform background-task primitive (a deferred, network-constrained, battery-aware scheduler), not a plain in-process timer. On desktop, a simple interval suffices. This split is the main implementation cost.
- **Cadence + triggers.** gopass uses ~3 days; gpm could default similarly and/or sync on app foreground / network-restore events (which are cheaper signals than a wall-clock timer and land when the user is actually about to act).
- **Divergence + Enforce.** A background tick that hits a divergence must **not** modal the user out of nowhere — it should leave the store on the reviewed tip and surface a passive badge, deferring the resolve to the next foreground. Same for an Enforce authenticity block.
- **AutoSync-off interplay.** A periodic sync would effectively re-publish for AutoSync-off users, which may contradict their intent (they turned AutoSync off to keep saves local). Likely: periodic sync respects the AutoSync toggle, OR is a separate "background sync" toggle. A design decision to make when this is scheduled.

## Alternatives considered

- **Per-write + manual only (status quo).** Kept as the MVP — sufficient for one active device.
- **Push-only on a timer.** Rejected as the primary shape — it publishes local commits but still leaves the view stale against remote changes. Bidirectional (pull+push) matches gopass and converges both ways.
- **Sync only on foreground/network-restore events (no wall-clock timer).** Attractive — cheaper and lands at useful moments. Could be the whole feature, or combined with a long fallback cadence. Decision deferred to scheduling.

## Effort

~1-2 days (human, mostly the Android background-work plumbing) / ~30 min (CC for the rustpass/frontend tick + toggle wiring).

## Depends on / Supersedes

Builds on the decouple-sync work (the manual Sync path the timer reuses). No other dependency.
