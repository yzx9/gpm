# Periodic background sync

**Priority:** P3
**Status:** Draft
**Phase:** Next

## What

A low-frequency, OS-scheduled background sync that periodically pull+pushes the local store, so a device converges with the remote even when the app is never opened. Mirrors gopass's multi-day `autoSync`: a best-effort, opportunistic reconciliation run by the platform's deferred, network-constrained, battery-aware scheduler — not an in-process timer. Serves `docs/specs/005-git-storage`.

The foreground sync that shipped first (pull+push on cold-start / resume / unlock) stays alongside it. The two cover different cases and are complementary, not redundant.

## Why

A user who relies heavily on autofill may not launch gpm for long stretches — they fill credentials from the autofill service and never open the app. Foreground sync fires only on cold-start / resume / unlock, so such a device never publishes its local commits and never picks up remote changes until the user happens to open it. For one active device that is tolerable; for a multi-device household or a team store it means silently diverging views and surprising conflicts later. A periodic background tick catches these up the way gopass already does.

This is convergence and UX, not a correctness gap — divergences are already resolved by the existing sync-time resolve flow.

## Context

- **Reuse, don't rebuild.** The sync a background tick runs is the same pull+push the manual and foreground paths run; it invents no new sync logic. That engine already lives in the Tauri-free library core, so a background worker reaches it directly through the native library already shipped in the app — **no new shared "core" layer needs to be extracted for this**. The worker is simply another host over the same library the app uses.

- **Foreground sync stays.** A periodic tick is coarse — the platform enforces a minimum interval and is itself opportunistic under Doze — so it cannot guarantee freshness at the moment the user opens the app. Foreground sync owns that "open right now" refresh. The two deliberately tolerate occasional double-runs rather than coordinate a shared last-sync timestamp.

- **AppLock constraint.** While the AppLock launch-gate is on, the master key sits behind a biometric prompt a background component cannot show, so the sealed repository config (git remote + credentials) is unreadable and a background tick must skip. Background sync therefore runs only on the auth-free master-key path (AppLock off). Under AppLock, the foreground sync (post-unlock) remains the only automatic path.

- **Platform scheduler, not a timer or a foreground service.** A reliable periodic sync needs the platform's deferred, network-constrained, battery-aware primitive — Android's `JobScheduler` (periodic, with a network constraint). A plain in-process timer does not survive the OS suspending or killing the process. A long-running foreground service is the wrong shape here: it needs a persistent notification, drains battery, does not survive a force-kill or reboot, and sits on a known-broken lifecycle seam in the app framework (relaunch and activity-leak defects). It is reserved, at most, for a future "must not be paused mid-push" case that does not exist yet.

- **Divergence + authenticity.** A background tick that hits a divergence or an authenticity block must **not** surface a modal out of nowhere. It leaves the store on the reviewed tip and records a passive "attention" state the next foreground renders as a badge, deferring the decision to when the user is actually there — the same no-modal contract as the foreground sync.

- **AutoSync-off interplay.** A periodic sync would re-publish for users who turned AutoSync off, which may contradict their intent (they turned it off to keep saves local). Open decision: periodic sync respects the AutoSync toggle, or is a separate "background sync" toggle. Settled when scheduled.

- **Cadence.** gopass uses ~3 days; gpm would default similarly. The platform enforces a minimum periodic interval but the chosen cadence is far coarser, landing as an opportunistic catch-up rather than a wall-clock guarantee.

## Alternatives considered

- **Foreground sync only (status quo).** Kept as the on-open path; insufficient alone because it never fires when the app is not opened — exactly the heavy-autofill case this RFC addresses.

- **A long-running foreground service instead of a periodic scheduler.** Rejected: battery cost, persistent notification, no survival across kill/reboot, and the broken lifecycle seam. A foreground service is for continuous, user-visible work; periodic convergence is deferred and opportunistic, which is the scheduler's job.

- **An in-process timer.** Rejected: the OS suspends and kills backgrounded processes, so an in-process timer is unreliable exactly when it matters (app idle). The platform scheduler is what makes the work survive.

- **Extract a shared "core" layer first.** Rejected as a prerequisite: the sync engine is already in the Tauri-free library, reachable through the shipped native library. No new layer is needed for the worker to call it. (A separate foreground-testability refactor of the app shell may extract coordination logic later, but that is independent of this work.)

## Effort

~1–2 days (human, mostly the Android scheduler and native-bridge plumbing) / ~30 min (CC for the worker wiring and toggle).

## Depends on / Supersedes

Builds on the shipped foreground sync (whose rationale now lives in the code) and the manual sync path. Carries forward the not-yet-shipped background-timer rationale of the retired periodic-sync draft.
