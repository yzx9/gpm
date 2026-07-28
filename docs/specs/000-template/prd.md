---
pm: <name>
created: YYYY-MM-DD
version: 1.0.0
---

# <Feature Title>

> Status: Shipped | In flight | Partial | Planned | Blocked
> Related: ANNN · Last verified: YYYY-MM-DD

<!--
A PRD captures product REQUIREMENTS — functional + non-functional — and USER
characteristics. It is NOT implementation: "how it's built" lives in the code/git,
not here. Keep "current state" to a few sentences; detail belongs to the product.
Stay at product altitude — capabilities and user-facing behavior, not APIs, protocol
strings (e.g. `otpauth://`), file formats, crypto algorithm names, or layout / bug-level
details; those are implementation.

Only `prd.md` is required. `design.md` / `security.md` / `research.md` are optional
companions — see 000-template/README.md. Delete this comment when filling it in.
-->

## 1. Introduction

One paragraph: what this feature is and the user job it serves.

## 2. Motivation / Objective

Why it exists — the need it filled (shipped) or the gap it closes (forward).

## 3. Use Cases

For each relevant persona (see `../../personas.md`), write a **complete usage scenario** —
the concrete end-to-end flow they go through in THIS feature, grounded in their real
workflow (e.g. Jordan's desktop-gopass + mobile-gpm + git-sync + gopass-compat; Casey's
create-from-scratch + daily mobile access). Don't re-describe the persona; that lives in
personas.md. Skip a persona with no notable behavior here.

## 4. Key Aspects

### Product Design

### Functionality

### Compatibility

### Interactive

### Adaptive

### Security

### Reliability

<!--
Functionality     = functional requirements.
Compatibility     = interop / gopass-compat constraints.
Adaptive          = platform / device / state (incl. Android<->desktop asymmetry).
Security/Reliability = non-functional requirements.
Fill what's relevant; thin or omit an aspect that doesn't apply.
-->

## 5. Open Questions & Key Decisions

Decisions made (link the ADR for rationale) + unresolved tradeoffs.

## 6. Roadmap

Shipped milestones → Now → Next → Future, with ADR references.
Current state in a few sentences — no detail.
