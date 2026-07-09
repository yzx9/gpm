# Consolidate two-option toggles onto the segmented control

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

The hand-rolled two-option toggle pills used to switch identity kind
(age/ssh) and SSH key source (paste/generate) re-implement the job of the
existing segmented-control component. Replace them with that shared
component so there is one segmented-selector implementation.

## Why

DRY and consistency. The hand-rolled toggles are ad-hoc styled buttons that
each reproduce selected/unselected appearance, keyboard semantics, and press
feedback that the segmented control already provides correctly — and just
required their own ad-hoc tap-highlight and press-feedback fixes that the
shared component never needed. Two implementations mean double the
maintenance and a place for the two to drift apart subtly over time.

## Context

The segmented control is a generic selector backed by accessible sr-only
radios inside a fieldset+legend, with a themed selected-pill style and its
own press feedback; it takes a model value and emits changes. The two
hand-rolled toggles are conceptually identical: a required choice between two
options driving a local piece of state. Migration is a 1:1 replacement — bind
each toggle's state to the component's model/change — keeping the same two
options and the same behavior, just expressed through the shared primitive.
Design-level only; the result is visually and behaviorally equivalent. No
data-flow, storage, or threat-model impact.

## Alternatives considered

- **Keep the hand-rolled toggles.** Rejected: ongoing duplication, and they
  are the kind of ad-hoc control that accumulates one-off touch-feedback
  fixes the shared component already handles.
- **Build a separate smaller "two-option toggle" primitive.** Rejected: the
  segmented control is already generic over any number of options, so a
  two-option-only primitive would be redundant.

## Effort

~1-2h human / ~15min CC per toggle — wire up the model value and change
handler, replace the inline buttons, and verify selected-state, disabled
behavior, and press feedback match.

## Depends on / Supersedes

None. Lands more cleanly after the press-feedback / text-selection fix, since
the shared component's pills are already correct.
