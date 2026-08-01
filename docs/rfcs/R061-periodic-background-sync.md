# Periodic background sync

**Priority:** P3
**Status:** Shipped
**Phase:** Done

**Retained** past the usual ship-and-delete RFC convention: [R063](R063-decouple-at-rest-from-app-lock.md) actively extends this design (it lifts the AppLock-off limitation below), so the rationale stays here as the reference R063 builds on.

## What

A low-frequency, OS-scheduled background sync that periodically **pulls** the remote into the local store, so a device picks up changes even when the app is never opened. Mirrors the opportunistic, network-constrained catch-up of gopass's multi-day `autoSync` — run by the platform's deferred, battery-aware scheduler, not an in-process timer. Serves `docs/specs/005-git-storage`.

**Pull-only.** The persona this serves is read-only: a heavy-autofill user fills credentials from the autofill service and never creates secrets in-app, so a background push carries nothing of theirs and is dead weight. Publishing local creates stays with the foreground path (the per-save autosync publish and the cold-start / resume / unlock sync). The design extends to push if a write-heavy persona ever needs it; until then, pull is the whole job.

The foreground sync that shipped first (pull+push on cold-start / resume / unlock) stays alongside it. The two cover different cases and are complementary, not redundant.

## Why

A user who relies heavily on autofill may not launch gpm for long stretches — they fill credentials from the autofill service and never open the app. Foreground sync fires only on cold-start / resume / unlock, so such a device never picks up remote changes until the user happens to open it. For one active device that is tolerable; for a multi-device household or a team store it means silently falling behind and surprising conflicts later. A periodic background pull catches these up the way gopass already does.

This is convergence and UX, not a correctness gap — divergences are already resolved by the existing sync-time resolve flow.

## Context

- **Reuse, don't rebuild.** The sync a background tick runs is the same pull the manual and foreground paths run; it invents no new sync logic. That engine already lives in the Tauri-free library core, so a background worker reaches it directly through the native library already shipped in the app — **no new shared "core" layer had to be extracted**. The worker is simply another host over the same library the app uses.

- **Foreground sync stays.** A periodic tick is coarse — the platform enforces a minimum interval and is itself opportunistic under Doze — so it cannot guarantee freshness at the moment the user opens the app. Foreground sync owns that "open right now" refresh. The two deliberately tolerate occasional double-runs rather than coordinate a shared last-sync timestamp.

- **AppLock constraint (the limitation R063 lifts).** While the AppLock launch-gate is on, the master key sits behind a biometric prompt a background component cannot show, so the sealed repository config (git remote + credentials) is unreadable and a background tick must skip. Background sync therefore runs only on the auth-free master-key path (AppLock off). Under AppLock, the foreground sync (post-unlock) remains the only automatic path. R063 decouples at-rest encryption from the app-lock biometric precisely so a background tick can run under AppLock too.

- **Platform scheduler, not a timer or a foreground service.** A reliable periodic sync needs the platform's deferred, network-constrained, battery-aware primitive — Android WorkManager (periodic, with a network constraint; backed by JobScheduler). A plain in-process timer does not survive the OS suspending or killing the process. A long-running foreground service is the wrong shape here: it needs a persistent notification, drains battery, does not survive a force-kill or reboot, and sits on a known-broken lifecycle seam in the app framework (relaunch and activity-leak defects). It is reserved, at most, for a future "must not be paused mid-push" case that does not exist yet.

- **Cross-process lock.** The Worker runs in its own process with its own store instance, so the in-process write mutex that serializes repo mutations inside one app instance cannot reach it. A flock-style advisory lock on a lockfile next to the repo serializes the two processes on the git index; it is non-blocking with a brief bounded retry (a background tick is best-effort and yields to user-initiated work), and OS-owned so a killed Worker never leaves a stale lock. This is the one piece that materialized only once there were two store instances.

- **Divergence + authenticity.** A background tick that hits a divergence or an authenticity block must **not** surface a modal out of nowhere. It leaves the store on the reviewed tip and records a passive "attention" marker the next foreground renders as a badge, deferring the decision to when the user is actually there — the same no-modal contract as the foreground sync. The marker is a dedicated file, not a preference field, so the headless write can't race a foreground preference write; the full sync outcome (which carries secret entry names) is never persisted.

- **Linked to AutoSync.** Background sync runs only when AutoSync is on. A user who turned AutoSync off did so to keep saves local, and a background tick that fired every interval would contradict that intent. Turning AutoSync off cancels the schedule outright so the Worker doesn't wake every interval just to skip on the AutoSync gate and waste battery; the cadence setting is shown only when AutoSync is on.

- **Cadence.** Off / 1h / 6h / 12h / 1d / 3d, default Off. gopass uses ~3 days; gpm offers the same top end plus finer-grained options. The platform enforces a minimum periodic interval, so the finer cadences land as an opportunistic catch-up rather than a wall-clock guarantee.

- **Defense-in-depth gates.** The Worker re-checks every gate itself (cadence, AppLock-off, repo-ready, AutoSync) rather than trusting the schedule — a stale schedule, a preference change, or an AppLock toggle between enqueue and fire still skips cleanly without touching the network.

## Alternatives considered

- **Foreground sync only (status quo).** Kept as the on-open path; insufficient alone because it never fires when the app is not opened — exactly the heavy-autofill case this RFC addresses.

- **A long-running foreground service instead of a periodic scheduler.** Rejected: battery cost, persistent notification, no survival across kill/reboot, and the broken lifecycle seam. A foreground service is for continuous, user-visible work; periodic convergence is deferred and opportunistic, which is the scheduler's job.

- **An in-process timer.** Rejected: the OS suspends and kills backgrounded processes, so an in-process timer is unreliable exactly when it matters (app idle). The platform scheduler is what makes the work survive.

- **Extract a shared "core" layer first.** Rejected as a prerequisite: the sync engine is already in the Tauri-free library, reachable through the shipped native library. No new layer was needed for the worker to call it.

- **Pull+push instead of pull-only.** Rejected for the read-only heavy-autofill persona: there is nothing of theirs to push, so a background push is dead weight. Reassess if a write-heavy persona emerges.

## Effort

~1–2 days (human, mostly the Android scheduler and native-bridge plumbing) / ~30 min (CC for the worker wiring and toggle).

## Depends on / Supersedes

Builds on the shipped foreground sync (whose rationale now lives in the code) and the manual sync path. Carries forward the not-yet-shipped background-timer rationale of the retired periodic-sync draft. Its AppLock-off limitation is lifted by [R063](R063-decouple-at-rest-from-app-lock.md).
