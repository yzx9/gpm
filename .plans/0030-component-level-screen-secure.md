# Component-level screen-capture protection

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Today screen-capture protection (Android `FLAG_SECURE`) is route-level: a page
is either fully secured or fully capturable. This RFC proposes component-level
granularity — securing only the moments a secret is actually on screen (a reveal
action, an export, a generated-password display) while leaving the rest of the
page capturable.

## Why

Route-level granularity forces a coarse choice: secure the whole page (losing
the ability to screenshot anything on it) or leave it open (risking the secret).
Mixed-sensitivity pages get the worst of both. The settings page is the live
example — it is secured page-wide today only because its SSH-private-key export
paints a secret; everything else on it (repo URL, commit identity, public key)
could safely be capturable. Component-level protection lets each secret-reveal
opt into `FLAG_SECURE` for just its duration, so non-secret content stays
shareable.

## Context

`FLAG_SECURE` is a window-level flag on the host activity's window. Toggling it
around a reveal is a route baseline plus a per-component lifecycle hook: on
reveal, raise the flag; on hide or unmount, restore it to whatever the current
route's baseline calls for. The threat-model impact is a tighter guarantee
(only the secret-bearing frames are protected) with better UX (the surrounding
page is screenshot-safe). The risk is a reveal/restore race that briefly leaves
the flag in the wrong state — the same class of timing the route-level guard
already handles, now per-action, so the same await-before-paint discipline
applies.

## Alternatives considered

- **Keep route-level only** (today): simplest, but over-secures mixed pages and
  blocks legitimate screenshots of non-secret content.
- **Per-page user toggle** (a switch per page in Settings): pushes the
  mixed-sensitivity problem onto the user and still cannot resolve a single page
  that has both secret and non-secret moments.
- **Secure only on explicit reveal, per-component** (this RFC): the minimal
  surface that matches where secrets actually appear.

## Effort

~1–2 days (human) / ~1 hour (CC) — a small raise/restore composable plus wiring
each reveal action (the settings key export today, any future entry-list reveal).

## Depends on / Supersedes

None. Builds on the route-level guard shipped alongside this RFC.
