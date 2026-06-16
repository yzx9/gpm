# Plan: Pull / Sync Divergence Detection & Conflict Handling

## 1. Context (current state)

`pull_repo` → `Store::sync()` → `git::pull_repo` is currently **fast-forward only**: in Off mode it does an in-place fetch + checkout; in Audit/Enforce mode it fetches into a temp ref, verifies signatures, then conditionally fast-forwards. Both paths use `graph_descendant_of` to check whether the fetched tip is a descendant of the current HEAD, and **if it isn't (i.e. the branches have diverged) it returns a hard error**:

> `PullFfFailed` — "Cannot fast-forward: branches have diverged. Resolve on desktop."

1. `SyncResult` only carries `changed / head / authenticity` — **no conflict or divergence field**.
2. The git layer has **no merge / rebase capability** — only fetch, fast-forward, and hard-reset.
3. The frontend surfaces this as a generic error with **no resolution UI**.

## 2. Why it matters now

gpm has gone from read-only to **writable** (`create` / `create_from_preset`). Multi-device scenario: phone A adds a secret, laptop B adds another, each commits + pushes → the next pull on either side finds local commits the remote doesn't have → divergence → currently a hard error that kicks the user back to a desktop client. Writing capability turns divergence from "theoretically possible" into "routinely hit."

## 3. Key insight

Secrets are **one file per secret** (`.age` binary encrypted blobs). Therefore:

1. Two devices each adding/modifying **different** secrets → git can **auto-merge cleanly**, no conflict.
2. Only when **both sides modify the same secret file** is there a real git conflict — and since `.age` is a binary encrypted blob, git can't do a content-level 3-way merge. This is exactly the **decrypt-aware "same-name" conflict** the write path already handles (existing `ConflictChoice` / `fetch_remote_blob` framework).

So most "pull divergences" could actually **auto-merge**; real conflicts are confined to same-name secrets, and a decrypt-aware resolution framework already exists to reuse.

## 4. Design options

| #   | Option                              | Approach                                                                                                                                      | Pros                                                                 | Cons                                                                                                                |
| --- | ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| 1   | Detect + destructive "adopt remote" | Return a structured result on divergence; frontend modal: [Adopt remote (discard local)] / [Cancel], reusing the existing fast-forward helper | Small change, reuses existing capability                             | **Loses local-only commits (local new secrets gone)** — dangerous under multi-device writes                         |
| 2   | Merge                               | Add merge: different secrets auto-merge; same-name secret falls through to per-secret decrypt-aware conflict                                  | Closest to gopass, truly solves multi-device                         | Most work; authenticity verification range must be redefined; binary-blob same-name conflicts still need a fallback |
| 3   | Detect + report only                | Return structured divergence info (N local-ahead / M remote-ahead), tell the user to resolve on desktop                                       | Smallest change                                                      | Doesn't actually solve it — just a friendlier error                                                                 |
| 4   | Rebase local onto remote            | Add rebase: replay local-only commits onto the remote tip; same-name conflicts fall through to per-secret resolution                          | Linear history, closest to "pull-then-push"; different secrets clean | Comparable work to #2; rebase rewrites commit SHAs, so signed commits need care                                     |

## 5. Recommended direction: phased

### 5.1 Phase 1 — Structured divergence detection + explicit safe resolution

Upgrade the "hard error" into a "resolvable result," **without introducing merge/rebase — zero binary-merge risk**:

1. On divergence, don't error — return structured divergence info (N local-ahead, M remote-ahead).
2. Frontend shows a modal offering a binary choice: **"Adopt remote (discard local changes)"** vs **"Cancel."**
3. Deliberately only these two options: no merging; worst case is "discard local" and it requires explicit confirmation.
4. Any resolution that changes HEAD must re-run signature verification and refresh the badge — Enforce must not be bypassed via "resolve divergence."

This turns "unhandleable in-app" into "safely handleable in-app (pick one)" — low risk, high value.

### 5.2 Phase 2 (optional) — Rebase auto-merge

On top of Phase 1, add **rebase** alongside "adopt remote": replay local-only commits onto the remote tip; different secrets merge cleanly; same-name binary conflicts fall through to per-secret decrypt-aware resolution (reusing the existing conflict framework). This is the "fully automatic multi-device sync," but binary-merge / signature interactions are complex, so it's recommended only after Phase 1 lands and the UX is validated.

## 6. Security considerations

1. **"Adopt remote" discards local-only commits** — must require explicit user confirmation; the UI copy must state "this will lose your N local commits."
2. **Invisible remote secrets**: after adopting remote, entries not encrypted to me remain unreadable — no more dangerous than a normal pull. But if Phase 2 touches same-name conflicts, it must reuse the write path's `remote_decryptable` / `KeepMineForce` logic to avoid silently destroying data we can't read.
3. **No plaintext over IPC**: any resolution involving plaintext (e.g. Phase 2 same-name replay) must stash it in memory the way the write path does, never re-crossing IPC.

## 7. Open questions

1. Scope: do Phase 1 (detect + binary choice) first, or go straight to Phase 2 (rebase auto-merge)?
2. Beyond "adopt remote," should Phase 1 also offer "keep local, push later"? Recommend Phase 1 only offers "Cancel."
3. Should "adopt remote" offer a **preview** of which local secrets will be lost (list local-only entry names)?
4. gopass compatibility target: strictly align with gopass's pull-before-write + merge semantics (gopass uses git merge, not rebase)?
