# Extend the single-source codegen to the other Rust↔TypeScript unions

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Several other values cross the Rust↔TypeScript boundary as a Rust enum
hand-mirrored into a TypeScript string union — biometric availability,
secure-screen mode, identity lock mode, commit-authenticity verify mode, and
commit-signature status. Each is the same drift class as the error codes: a
rename on one side slips past the TypeScript checker. This RFC extends the
single-source codegen pipeline — the Rust enum as the sole source, generated
checked-in TypeScript, a lint-gate freshness check — to those unions, so the
tooling built once for error codes amortizes across the rest.

Serves the system-wide sanitized-error and cross-layer-type contracts rather than
any single feature spec — these are cross-cutting boundary types with no PRD of
their own.

## Why

The error-code consolidation establishes the pattern and the generator, and
proves the freshness check catches drift at the local lint gate before it reaches
CI. The other Rust↔TypeScript unions are leaves on the same tree, and leaving
them hand-mirrored preserves the exact silent-rename risk the consolidation was
built to remove — just in smaller surface area. Extending is low-risk
amortization: each union is a small, stable set, and the generator is already
generalizable.

The cost of not doing it is the cost of the next rename that quietly desyncs a
status or mode the frontend branches on.

## Context

Each union receives the same treatment as the error codes: the Rust enum is
annotated for variant iteration and serialization, the generator emits a
checked-in TypeScript constant (and a snapshot) from it, and the lint gate's
freshness check covers every generated artifact together. Generated files stay
checked in, so normal frontend development needs no pre-build generation step —
the same convention the project already uses for its checked-in Android build
tree.

One union needs marginally more care than the others: commit-signature status is
a tagged shape (a status kind paired with per-variant detail), not a flat
string. The generator must emit a TypeScript discriminated union that mirrors
the Rust tagged enum, including the per-variant payload, rather than a bare
constant. This is the one place the generalization earns its keep — it is also
the one place a hand-mirror is most likely to drift.

No new threat surface: these values are non-secret configuration and status
labels.

## Alternatives considered

- **Keep each union hand-mirrored with per-union tests.** Rejected: it is the
  status quo that drifts, and a per-union test written by hand on both sides of a
  language boundary is the same theater as the cross-layer error-code test the
  consolidation replaces — both lists pass whenever both are updated together.

- **A single big-bang codegen pass over every cross-language string at once.**
  Rejected as the _first_ move: proving the pattern on the error codes first (the
  consolidation, this RFC's prerequisite), then extending union by union, is
  lower-risk than a sweeping pass and lets each union's edge cases surface
  individually. The big-bang version is available later if the per-union cadence
  proves tedious.

- **Only ever codegen the error codes.** Rejected: it leaves the known drift
  class alive everywhere else, which undercuts the premise of doing the error
  codes at all.

## Effort

~small–medium.

- _Human:_ generalizing the generator beyond a single enum; handling the
  tagged-union case for commit-signature status.
- _CC-amenable:_ the mechanical, per-union repetition once the generator is
  generalized.

## Depends on / Supersedes

Depends on the codegen generator and the lint-gate freshness check from the
error-code consolidation. Related to `R081` (typed-error-contract), whose
closed-code-set foundation this builds on.
