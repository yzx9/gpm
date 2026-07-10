# <Title>

**Priority:** P0 | P1 | P2 | P3
**Status:** Draft | Accepted | Blocked | Deprecated
**Phase:** Now | Next | Future

> ## Metadata
>
> The three header fields are independent — **Priority** is importance, **Phase** is timing (a `P1` can be `Future`; a `P3` can be `Next`).
>
> - **Priority** — `P0` blocking / must-do · `P1` high · `P2` medium · `P3` low or nice-to-have.
> - **Status** — `Draft`: written, under consideration, not yet committed to · `Accepted`: decided to do and scheduled · `Blocked`: wanted and analyzed, but gated on something external we don't control (an upstream library, a second consumer, a prerequisite change) — reassess when the blocker resolves · `Deprecated`: reassessed and set aside (parked or decided against); keep the file as the record of why not.
> - **Phase** — `Now`: current focus · `Next`: right after the current focus · `Future`: later, no immediate plan.
>
> When the RFC's feature ships, delete the file — the rationale then lives in the code docs / threat model, and the numbering gaps this leaves are expected.

> ## Naming & numbering
>
> One file per RFC: `NNNN-kebab-title.md`. `NNNN` is 4-digit zero-padded; **next number = current max + 1**. The "current max" is the highest `NNNN` on **any branch** (local or remote), not just the local checkout — scan every branch tip's `docs/rfcs/` (e.g. `git ls-tree -r --name-only <ref> -- docs/rfcs/` over each `git branch -a` ref), since an in-flight branch may already hold a higher number than your local `main`.

> ## Altitude — the one rule
>
> If you are writing file paths, line numbers, struct fields, function signatures, or code, you have dropped below RFC altitude — move it into the implementation. An RFC should still read cleanly after the code it describes has been rewritten twice. The RFC records _why_; the implementation records _how_.
>
> The Metadata, Naming & numbering, and Altitude notes above are author guidance — delete all three when filling the RFC in.

## What

One paragraph: the problem and the proposed shape of the solution.

## Why

Motivation — what goes wrong today, or what this unlocks.

## Context

Background, current behavior, relevant prior art (gopass / age / others).
Design-level notes only: interfaces, data flow, threat-model impact.
**No file paths, line numbers, type signatures, or code.**

## Alternatives considered

Other approaches and why they were rejected.

## Effort

~size (human) / ~size (CC)

## Depends on / Supersedes

NNNN-titles, if any.
