# Make the error code a meaningful, closed contract on both sides

**Priority:** P1
**Status:** Draft
**Phase:** Next
**Revision:** 1

## What

The user-facing error today is a categorical code paired with a free-form
message. A few of those codes are catch-alls that fold many unrelated failures
together, so for those cases the code carries no usable meaning and the message
— English, developer-facing — becomes the only carrier, which the frontend then
shows verbatim, bypassing localization. This RFC closes that gap on both sides:
the backend stops collapsing distinct failures into catch-alls (each domain owns
a rich internal error type that projects to a **closed** code set at one
explicit boundary, with a dedicated channel for background-task panics), and the
frontend owns a **code → localized-message** map so display is driven by the
code, with the backend message demoted to a diagnostic.

Serves the system-wide sanitized-error contract stated across the encryption and
security specs ("errors carry codes / generic descriptions only, never secrets")
rather than any single feature spec — error handling is cross-cutting and has no
PRD of its own.

## Why

What goes wrong today:

- **The catch-alls erase meaning.** One general-store code alone covers roughly
  a third of every place an error is constructed, spanning at least eight
  failure classes that have nothing in common: lock-state corruption, crypto-
  primitive failures, key generation, internal git operations, invalid input
  data, and platform features wholly unrelated to the store (clipboard, archive
  export, logging). A consumer seeing that code cannot tell any of them apart.
- **Meaning retreats into the message, then leaks to the user.** Because the
  code is meaningless for those cases, the real semantics live only in the
  English message string. The frontend cannot map a catch-all to anything, so
  its universal idiom is "show the backend message, else fall back to a generic
  per-operation string." Every catch-all failure is therefore shown verbatim in
  developer-flavored English — bypassing localization entirely, even though the
  app ships a second locale. Users see "lock poisoned" or "wordlist is empty" for
  failures that should have a friendly, localized line.
- **Panics are disguised.** A blocking-task panic (or any join failure) is
  swallowed into the same general-store code, so a panic originating anywhere is
  reported as an ordinary store failure — the worst possible classification for
  triage.
- **Classification by string-matching.** One blanket conversion from the git
  library's error picks a code by substring-matching its English message text,
  which is fragile against library version or locale changes. This is the
  opposite failure mode from the catch-alls: over-eager guessing instead of
  under-classification.
- **The code set is open.** The code field is an open string rather than the
  enumerated set, so a parallel collection of ad-hoc codes exists outside the
  enum. The enum is not the single source of truth it appears to be.

Net: the error-code design does not earn its keep. Neither side can rely on a
code to mean something specific, and the two gaps reinforce each other — the
backend stuffs meaning into the message because the code is generic, and the
frontend shows the message verbatim because it cannot map the generic code.

## Context

**The contract — closed and categorical.** What crosses the IPC boundary is a
closed set of categorical codes. _Closed_ means the backend can only emit codes
from this set; no open strings, and the ad-hoc codes outside the enum are folded
in. _Categorical_ means each code names a failure class specific enough to (1)
branch on for control flow and (2) map to a distinct user-facing string. The
wire format of codes the frontend already branches on is preserved, so there is
no forced backend-and-frontend cutover; new codes simply get new translations.

**Domain ownership (the backend half).** Each backend domain — at-rest sealing,
crypto, git storage, identity, app config, and so on — keeps its own rich,
cause-preserving error type internally. Conversion to the closed code set happens
at **one explicit boundary**, the app's command layer, not through scattered
blanket conversions. Centralizing the rich→flat mapping is what makes it
auditable: a test can assert that each domain failure projects to the intended
code. That projection test is the discipline that keeps catch-alls from
regrowing — it is the substitute for the codegen enforcement a more elaborate
contract would provide (see Alternatives).

**Killing the catch-alls.** The general-store, generic-IO, and generic-config
codes are replaced by specific categorical codes — lock-state corruption,
crypto-primitive failure, key generation, an internal-git-operation code kept
distinct from the existing missing-repo and non-fast-forward codes,
invalid-input-data, and per-platform-feature codes for the things that are not
store failures at all. Every current catch-all site maps to one of these.

**A dedicated panic channel.** Blocking-task panics and join failures get their
own code, distinct from every domain failure, so a panic is never reported as an
ordinary error. The diagnostic message is retained for logs.

**The git-library conversion** stops substring-matching message text.
Classification happens where the call has context (clone, push, and network
already classify explicitly today); the blanket conversion is demoted to a last
resort or removed.

**Frontend message ownership (the frontend half).** The frontend gains a single
code → localized-message lookup, with a curated generic default, replacing the
per-call-site "message-or-fallback" idiom everywhere. The backend message is
kept but demoted to a diagnostic — surfaced only behind a details affordance or
in the diagnostics export, never as the primary visible text. This is what makes
localization actually reach the user for the codes that currently bypass it.

**Threat model — unchanged.** Errors still carry codes and generic descriptions
only, never secrets. Moving the user-facing string to the frontend does not
change what crosses the boundary; the diagnostic message remains sanitized. No
new secret surface is introduced.

## Alternatives considered

- **Backend-localized messages (frontend stays dumb).** Let the backend produce
  the final localized string and keep displaying `message`. Rejected: it
  relocates the meaning back into the message instead of into the code, leaves
  the code meaningless, and splits translation ownership between the backend and
  the frontend's existing i18n — two places to keep the same strings in sync.

- **Code plus a structured detail payload.** Carry a typed context object per
  code so the frontend can interpolate ("push rejected for branch X"). Genuine
  value, but it adds per-code schema versioning and template-interpolation cost
  across the boundary. Deferred, not rejected: it layers cleanly on top of the
  closed-code-set and frontend-map foundation, and is worth revisiting for the
  handful of codes that genuinely need context. Landing the foundation first
  avoids designing schemas before the code set has settled.

- **Single-source contract plus dual codegen.** Define every code once — with
  metadata such as retryable and severity, and both locales — in one neutral
  source, and generate the backend enum, the frontend type union, and the locale
  files from it, with a CI assertion that every code has a translation. This is
  the most thorough option: it makes "the frontend is missing a translation for
  a code the backend emits" structurally impossible. Deferred, not rejected: it
  is the natural evolution once the code set is closed and stable, but it adds
  build tooling whose cost is not justified until the set has settled. The
  chosen design reaches backend honesty and frontend coverage through domain
  ownership, projection tests, and a frontend map instead — the same guarantees,
  discipline over tooling.

## Effort

Medium-large.

- _Human:_ carving the backend into domain error types and designing the single
  projection boundary; deciding the categorical code set and where every current
  catch-all site lands; reviewing that the closed set does not quietly reintroduce
  a catch-all.
- _CC-amenable:_ the mechanical split-and-remap of catch-all sites to specific
  codes; retargeting every frontend call site to the shared lookup; writing the
  projection tests and a translation-coverage check.

## Depends on / Supersedes

None. Cross-cutting; serves the sanitized-error contract in the encryption and
security specs.
