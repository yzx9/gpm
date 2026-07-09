# Gate hover feedback to pointer devices

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Pressable elements whose `:hover` style is not restricted to pointer-capable
devices can be left in a stale "hovered" visual state after a tap on Android
WebView. Gate every pressable's `:hover` feedback behind `(hover: hover)` so
it only applies on mouse/trackpad, making the pointer-only hover gate a
project-wide invariant.

## Why

On a touchscreen, the browser fires `:hover` after `touchend` and the element
often stays "hovered" until the user taps somewhere else. A button that stays
highlighted after a single press reads as stuck or broken. Today only some
pressables gate their `:hover` (the shared button and copy-button primitives
do); others define a bare `:hover`, so the stuck-hover artifact shows up
inconsistently across the app. Closing the gap removes a confusing visual bug
that is distinct from (and was surfaced alongside) the press-feedback work.

## Context

The correct pattern already exists in the codebase: `:hover` rules wrapped in
`@media (hover: hover)` so touch never sees them. This RFC is about adoption
— applying that gate everywhere a pressable defines `:hover` — not about
changing any visual design. Touch press feedback comes from `:active`, which
is the reliable signal on Android and is unaffected. The work is a mechanical
audit: find every bare `:hover` on an interactive element and move it under
the media query, leaving non-interactive `:hover` (e.g. purely decorative)
alone. No data-flow, storage, or threat-model impact; presentation only.

## Alternatives considered

- **Drop `:hover` rules entirely, rely on `:active` only.** Rejected: desktop
  users lose the hover affordance that signals a control is interactive before
  they click.
- **Accept the sticky state.** Rejected: it reads as a bug to users and
  undermines trust in the touch UI.

## Effort

~1-2h human / ~10min CC — mechanical audit-and-wrap across the interactive
surface, plus an on-device pass to confirm no touch path still sticks.

## Depends on / Supersedes

None.
