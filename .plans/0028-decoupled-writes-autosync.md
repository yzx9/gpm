# Decouple sync from writes; add an AutoSync toggle

**Priority:** P1
**Status:** Active (PR1 — behavior-preserving prep — landed; PR2 — the behavior flip — in progress)
**Phase:** Now

## What

`rustpass` writes stop owning git sync. `set` / `create` / `update` / `delete`
become local-only (encrypt → write → local commit, no network). Sync (pull+push)
becomes an independent Application-layer API, wrapped around each write by a
per-device **AutoSync** toggle (default **On**), mirroring how gopass gates its
per-command sync. Conflict detection moves to **sync time**, where gpm keeps its
stricter fast-forward-only discipline and an explicit keep-mine / adopt-remote /
cancel resolve — it does not drop to gopass's merge-and-punt.

## Why

Today `rustpass`'s `Store` owns gopass-style "PushPull" sync **inside the write
path**: `set` pulls, writes, commits, pushes, and — on push rejection — runs a
rollback → fetch-remote → replay-or-`Conflict` dance, with a plaintext
`pending_write` stash to carry the user's edit across the resolution. `delete`
mirrors it. That entire inline conflict machinery exists **only because push
lives inside the write** — push-rejection is pressed into service as the conflict
signal. It is the bulk of the write-path complexity, and it is the thing being
removed. Once push leaves the write, the stash, the `WriteOutcome::Conflict`
variant, and the write-path resolver all dissolve.

Verified against a local gopass clone: gopass's write (`leaf.Store.Set`) is
encrypt → write → `git add` → commit + best-effort `TryPush` with **no inline
conflict detection**; the pull is a separate, `--nosync`-gateable pre-command
hook; gopass marks `*.age` binary and **punts** on same-secret conflicts. gpm is
deliberately stricter (fast-forward-only pulls, an explicit resolve flow), and
that strictness is preserved — just relocated to sync time.

## Context

**The hard part — sync-time "keep mine".** After decoupling, a rejected push
means local and remote have diverged and the user's write is already a commit on
HEAD. "Keep mine" must take the local-only entries, **decrypt** them with the
(unlocked) identity, **re-encrypt** with the **current remote-tip recipients**,
commit on the tip, and push (now a fast-forward). It honors a remote
`.gopass-recipients` change — re-encrypting onto the new recipient set, and
re-adding our own key so we can still read what we kept.

**Why re-encrypt, not rebase.** Replaying the local commits via a git rebase
would preserve the **old ciphertext** — and thus the **old recipient set** —
silently keeping stale recipients after a recipient-list change. For a team
store that is a silent access-control regression (a rotated-out teammate's key
keeps decrypting the re-based entries). Decrypt + re-encrypt is what today's
write-path resolver already does; this generalizes it to the whole local-only
set.

**Never merge `.age` blobs.** When both sides changed the **same** entry, there
is no safe automatic merge — "keep mine" refuses (`PushRejected`) and the user
must adopt the remote or cancel. A local delete vs a remote modify is the same
class of irreconcilable conflict; both-deleted is agreement, not a conflict.

**Spurious-divergence fix.** A strictly-local-ahead repo (an unpushed commit, the
remote unchanged) was reported as `Diverged` by the pull — which would have
popped a divergence modal on every save after an unpushed commit. It is a
no-op pull (then a push to publish). A three-way classification
(equal / remote-ahead-ff / local-ahead / diverged) tells the benign cases apart
from a true split.

**Authenticity stays load-bearing.** Under Enforce, a blocked pre-write pull
**aborts** the write with the real authenticity error — never a misleading
divergence modal. The keep-mine remote-only range is verified under the policy
(no Enforce bypass), and the adopt step reuses the **exact** reviewed tip (a
single fetch; no second fetch that could race past the reviewed tip and bypass
the check).

**AutoSync default On, per-device.** Stored in `repo.json`; omitted from
serialization while on, so an existing file (no `autosync` key) deserializes
**on** — the pre-toggle behavior is preserved across the upgrade, and users who
never toggle it see no shape change. When off, saves stay local until a manual
**Sync**; the manual Sync is pull+push (the old "Pull" button now also pushes)
so an autosync-off device can still publish.

**Serialized mutations.** All repo-mutating operations (writes, pull, push,
divergence resolution) share one Store-level async critical section, so two
in-flight writes can't race the git index and a reviewed divergence can't go
stale vs local HEAD mid-resolution.

**Staging.** Deliberately two PRs, each reviewable and bisectable on its own:

- **PR1 — behavior-preserving prep (landed).** The new primitives land alongside
  the old write path: the keep-mine re-encrypt resolver, the three-way pull
  classification, the on-demand divergence preview, the divergence-choice enum,
  and the (not-yet-acquired) serialization mutex. Old `set` / `delete` still
  drive writes. One user-visible change shipped here: a strictly-local-ahead repo
  no longer reports a spurious divergence on manual pull — it is a no-op pull.
- **PR2 — the behavior flip (active).** Writes become local-only; the
  Application-layer `autosync_write` orchestrator wraps each save in
  pull → write → push and routes a rejected push (or an Enforce-blocked pull) to
  the divergence/authenticity surface; the plaintext stash and the write-path
  conflict types are retired; a context-aware divergence modal replaces the old
  write-conflict modal; the AutoSync toggle and the Pull→Sync relabel land in the
  UI.

## Alternatives considered

- **Keep sync coupled (status quo).** Rejected — the inline push-rejection
  machinery and the plaintext stash are the complexity being removed.
- **git2 rebase for "keep mine".** Rejected — it replays old ciphertext and keeps
  stale recipients after a recipient-list change: a silent access-control
  regression. Decrypt + re-encrypt onto the current recipient set is the
  gopass-compatible and safe choice.
- **gopass's merge-and-punt on same-secret conflict.** Rejected — gpm keeps an
  explicit, decrypt-aware resolve (keep mine / adopt remote / cancel) rather than
  silently picking a side.
- **Pure local-only writes, no auto-resolve at all.** Rejected — it would lose
  gpm's stricter conflict safety and push every divergence to a manual git
  operation, which is a worse UX for a password manager.
- **Periodic/background sync (gopass's 3-day autoSync).** Deferred — per-write +
  manual Sync covers the current need; a background timer is a future option.

## Effort

Large. PR1 (prep + tests) landed at ~the review-cleared estimate. PR2 (the flip:
orchestrator + authenticity-block branch, retiring the stash, the context-aware
modal, the AutoSync toggle, the rewritten conflict tests + frontend tests) is the
active phase.

## Depends on / Supersedes

- **Supersedes `0017-set-auto-sync-removal.md`.** That RFC asked whether to drop
  `set`'s speculative pre-sync; this answers it definitively by removing sync
  from the write path entirely (writes are local-only; sync is an app-layer
  concern).
- **`0026-edit-base-version-aware.md` stays independently needed.** The
  silent-stale-edit-clobber window does **not** collapse under decoupling: a
  pre-write pull still fast-forwards over a teammate's same-entry rotation when
  the local side has no intervening commit. Base-version-aware edit remains a
  separate requirement; this design must not claim otherwise.
- Builds on the pull-sync divergence work (the `SyncOutcome::Diverged` +
  adopt-remote resolution) and the edit/delete secrets work
  (`0020-edit-secrets` / `0021-delete-secrets`); the multi-recipient
  overwrite-safety gate (`remote_decryptable` / `keep_mine_force`) is deferred —
  gpm is single-identity today, so it matters only for future team stores.
