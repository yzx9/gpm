# Unlock modal overlay

**Priority:** P3
**Status:** Deferred (split out of [0002-keystore-biometric.md](./0002-keystore-biometric.md))
**Phase:** Post-MVP (v1.2)
**Depends on:** [0002](./0002-keystore-biometric.md) — needs the `is_unlocked()` fix and the biometric commands to already exist.

> High-level intent only. No implementation detail — that comes when this plan
> is picked up. Sections per the split request.

## Context

Today the app uses a dedicated `/unlock` route: on auto-lock (or launch of an
encrypted identity), the router redirects to a full-page unlock screen, which
unmounts whatever page the user was on. Biometric (plan 0002) ships on that
route first. This plan replaces the route with a global modal overlay so the
user re-authenticates **without leaving the page they were on**.

## Process

Move the unlock surface from a routed page (`UnlockPage.vue` + the `/unlock`
route + the `encrypted && !unlocked → /unlock` redirect in `main.ts`) to a
global `UnlockModal` mounted in `App.vue` that overlays whatever page is
current. The `identity-locked` event flips the modal's `locked` ref instead of
pushing a route; the router guard becomes configured-only. The biometric
auto-prompt, cancel, and key-invalidation logic that plan 0002 puts in
`UnlockPage.vue` moves into the modal. A blocking backdrop prevents interaction
with the page behind it while locked.

## Purpose

For a password manager that auto-locks every 5 minutes, the redirect-to-unlock
jarring the user out of the entry they were reading is itself friction — the
very friction biometric is meant to remove. A modal lets the user authenticate
in place and land back on the exact entry they were viewing, which is the
experience that makes biometric unlock feel worth it. It also collapses two
surfaces (the page and the future overlay) into one.

## Gains

- Re-authentication preserves the user's current page and scroll position — no
  losing your place in a long entry list to a route change.
- One unlock surface to maintain instead of a routed page plus a future overlay.
- Auto-lock becomes less disruptive: the modal appears, you tap fingerprint, you
  are exactly where you were.
- Cleaner router: the guard becomes configured-only, removing the
  `encrypted && !unlocked` redirect.

## Drawbacks

- Larger, riskier change than biometric on the existing route: touches `App.vue`,
  the router, and replaces `UnlockPage.vue`.
- Secret-containment responsibility shifts: a route unmounts the detail page
  (clearing its revealed password on unmount); a modal keeps it mounted behind
  the overlay. This requires new "clear revealed secrets on lock" wiring (see
  Blockers) that the route gave us for free.
- A blocking backdrop must genuinely capture all interaction — a half-baked one
  leaks taps to the locked page behind it.
- More frontend test surface (modal state machine: show/hide/auto-prompt/re-lock).

## Blockers

- **Clear revealed secrets on lock (codex Finding 4, hard blocker).** The modal
  keeps `EntryDetailPage` mounted behind the overlay, so a password revealed via
  `show_password` stays in the DOM (just covered) on auto-lock. The route path
  unmounted the page and cleared it; the modal does not. Before this plan can
  ship, there must be an explicit "on lock, zero/clear all currently-revealed
  secrets across the app" path — not just a backdrop. Concretely: the
  `identity-locked` event must drive a global clear of any in-DOM secret state,
  not only a UI overlay.
- **Depends on 0002.** The `is_unlocked()` SSH fix, the five biometric commands,
  and `biometric.ts` must exist (built in 0002 on the route). This plan moves
  them into the modal; it does not re-derive them.
- **Lock-timer race (codex Finding 6) becomes more visible.** With a modal that
  keeps the user on-page and auto-prompts, a stale `identity-locked` firing right
  after a fresh unlock (abort is not a generation check) re-shows the modal
  spuriously. Prefer resolving the TODO (monotonic session token on the timer)
  before or alongside this plan.
