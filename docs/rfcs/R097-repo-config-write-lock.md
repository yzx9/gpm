# repo.json write serialization via a cross-process file lock

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Every writer of `repo.json` serializes through a dedicated cross-process
advisory file lock (flock on a lock file next to `repo.json`), with the whole
read-modify-write performed inside the critical section: acquire → load →
apply only one's own field change → save → release. Closes #57 (silent
lost-update races; worst case a store bricked by the crypto migration
clobbering a concurrent identity save). Cross-cutting infrastructure — the
repo-scoped config underpins the git-storage sync writers and the identity
setup writers alike (specs 005 / 006).

## Why

`repo.json` is written by read-modify-write from many call sites with nothing
serializing them: settings mutations (credentials, commit identity,
authenticity keys, unlock opt-in), the identity save's crypto-kind persist,
setup full writes, and content migrations. Atomic-rename saves make each
single write tear-free, but two interleaved RMWs silently drop one side's
field change — a freshly saved PAT overwritten (next sync fails auth), or a
GPG identity's backend selection reverted to age (store bricked at next
restart). The risk was accepted on the crypto migration (#57) with the
agreement to build the lock "when the next content migration lands — or
sooner"; every future field rename or default change joins the same
unserialized set, so this recurs until fixed structurally.

The same window also lets two concurrent saves collide on the shared
temp-file path of the atomic write, which can commit one writer's content
under another's rename — worse than a lost update.

## Context

**The critical section is the RMW, not the write.** Locking only the save
leaves the load outside: a writer still acts on a stale snapshot and the
lost update survives. The fix is one lock-scoped entry point that loads,
applies a caller-supplied field mutation, and saves — writers stop doing
their own load/save. Pure reads that never feed a write stay lock-free:
atomic rename already guarantees they observe some fully-committed version,
which is fine for display; the only dangerous pattern is a stale read
feeding a later write, which the entry point eliminates. A shared-mode
(flock `LOCK_SH`) read can be added later if a caller ever truly needs
read-your-own-prior-write consistency.

**A dedicated lock, not the sync lock.** The existing cross-process repo
lock is non-blocking with a short retry and reports busy — right for
"background sync yields to the foreground", wrong for config writes, which
are tiny, must-succeed user actions. The config lock blocks with a bounded
timeout and fails loudly (a busy-style error surfaced to the user) rather
than silently. Blocking acquisition runs off the async runtime, and the
guard releases on drop. Since flock mutual exclusion is per open file
description, the same lock also serializes two concurrent handles inside
one process — covering the foreground app's own concurrent commands, the
foreground/headless-worker process split, and multiple desktop instances
pointed at one config directory, without any per-topology reasoning.

**Identity save keeps slow work outside the lock.** Its membership and
flip-guard checks involve decryption and repository scans; those run
lock-free on a possibly-stale snapshot (they only gate validity), and only
the final crypto-kind field change is applied through the lock-scoped
re-read/write. Writers that merely read to decide, then write through the
entry point, all follow this shape.

**All writers participate, including migrations.** Setup full writes, the
seal-envelope migration, and content migrations take the lock too — "lock
only during migration" was explicitly rejected in #57 as the recurring trap.
Lock ordering is fixed: sync lock before config lock, never reversed; the
existing migration paths already take them in this order. The lock is not
reentrant, so acquisition happens exactly once per public operation, at the
lowest sink.

## Alternatives considered

- **Accept the risk indefinitely.** The user base is small and the window
  narrow, but the failure modes are silent (lost credential, bricked store)
  and every future config migration re-opens the wound. Rejected as a
  permanent stance.
- **Reuse the sync lock for config writes.** Its non-blocking try semantics
  would turn user settings changes into random busy failures, and callers
  that already hold it during migrations would need reentrancy — a
  deadlock-ordering audit for no benefit over a dedicated lock.
- **In-process cached config + mutex, plus suppressing the background
  worker while the foreground is active.** Rejected: a lifecycle heuristic
  (check-then-act on "is the foreground alive") cannot carry correctness;
  desktop multi-instance and future platforms break the single-writer
  assumption; and a session-resident cache of the config means plaintext
  credentials (PAT, SSH key material) resident in memory for the whole
  session, contradicting the wipe-after-use posture. See R096 for the
  worker-suppression piece as a pure optimization.
- **Optimistic concurrency (generation counter / compare-and-swap on
  rename).** No portable conditional-rename primitive; retry loops for what
  an flock settles in microseconds. Complexity without benefit.

## Effort

~S (human) / ~S (CC) — one new lock module, one RMW entry point, mechanical
migration of the writers, plus concurrency tests (interleaved writers keep
both fields; lock ordering; identity-save increment).

## Depends on / Supersedes

None. Complements R096 (that one is a scheduling optimization; this one is
the correctness mechanism).
