# Read Attachments in Legacy-Rendered Secrets

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

gpm should correctly **read** binary attachments that live inside a deprecated
`GOPASS-SECRET-1.0` secret, **read-only**, once attachment support exists for modern
secrets. This is the legacy-format sibling of modern attachment support (R065): same
`Content-Disposition: attachment` + `Content-Transfer-Encoding: Base64` + base64-body
convention, but reached through R054's legacy parse, which flattens the header block into
the body. R065 scopes itself to the modern AKV layout; this RFC resolves the interaction R065
deliberately sidesteps.

## Why

gopass compatibility is a hard constraint, and a legacy secret can carry an attachment
exactly as a modern one can (the attachment convention is format-agnostic — it is two
attribute headers plus a base64 body, layered on whatever the host format is). After R054
ships, opening such a secret in gpm is no longer the silent wrong-password bug R054 fixed —
the password is correctly lifted from the `Password:` header — but the attachment itself is
rendered as two lowercased body lines (`content-disposition: …`, `content-transfer-encoding:
base64`) followed by a wall of base64. That is degraded, not broken, and it is no worse than
today's pre-R054 state (which was equally broken for the whole secret). This RFC exists so
the legacy case is not forgotten when attachment support lands: the detection and display
rules R065 assumes do not transfer verbatim, because R054's render changes where the
attachment signal lives.

## Context

**How gopass encodes attachments (format-agnostic).** Verified against `~/git/gopass`
`internal/action/binary.go`: an attachment is stored inline in the same secret as two
attribute lines (`Content-Disposition: attachment; filename="…"` and
`Content-Transfer-Encoding: Base64`) and a body that is the standard base64 of the file. It
is **not** multipart — gopass never used `multipart/mixed`. The same convention applies to
the everyday text format (AKV) and to the deprecated `GOPASS-SECRET-1.0` format, because
both are "headers + body" at the plaintext level.

**Where R065 stands and what it assumes.** R065 (modern attachments) keys detection off a
leading `Content-Transfer-Encoding: Base64` line in a distinct **attribute region** — the
lines right after the password in AKV — and relies on that region being structurally
separable from free-text body (so a body that merely contains the string
`Content-Transfer-Encoding` is not misclassified). R065 explicitly defers the legacy case
("this one covers modern attachments only").

**The R054 render interaction this RFC must resolve.** R054's `parse_legacy` has no
attribute model: it lifts the `Password:` header into the password slot and renders every
remaining header as a lowercased `key: value` line in the body. So a legacy attachment
secret, after R054, reaches the consumer as a body whose first lines are
`content-disposition: attachment; filename="…"` and `content-transfer-encoding: base64`,
then the base64 blob. The signal R065 detects is present, but the structural guarantee R065
relies on (a distinct attribute region) is not — it is all body text. That changes two
things this RFC must decide:

1. **Detection on a rendered body.** R065's case-insensitive `Content-Transfer-Encoding:
Base64` match still fires on the lowercased render, but the "must not misfire on free
   text" argument is weaker: in a legacy-rendered secret the attribute lines ARE body text,
   so a user's free-text note that happens to start with `content-transfer-encoding:` would
   look identical to a real attachment marker. R066 must either accept that residual misfire
   surface (with the same leading-line restriction R065 uses) or reach behind the render for
   the structured header.

2. **Whether `parse_legacy` should preserve attachment headers specially.** The clean fix is
   for `parse_legacy` to recognize the two attachment headers and either keep their case
   (not lowercased) or stash them as structured markers, so detection is uniform across the
   modern and legacy paths. That is a small follow-up tweak to R054's "render everything
   lowercased into the body" stance, and it is the option that makes R066 trivial rather than
   a parallel detection implementation.

3. **Display.** When an attachment is detected, the two attribute lines and the base64 wall
   should be stripped from the body shown to the user (mirroring what R065 Phase 2 plans for
   modern secrets), and the export-to-file affordance (R065 Phase 1) offered.

**The R054→R066 interim window.** Between R054 and the attachment RFCs landing, a legacy
attachment secret renders its headers lowercased into the body. If a user **edits** such a
secret in that window, gpm rewrites it as modern AKV (`reassemble(pw, body)`), freezing the
lowercased `content-disposition:`/`content-transfer-encoding:` lines and the base64 into a
modern body verbatim. When R065/R066 later try to detect the attachment, they would see those
already-lowercased, already-baked lines. This is recoverable (the base64 and filename
survive) but R066's detector must cope with the baked form, not only the fresh header form.
Documenting this interim is part of this RFC; the cleanest mitigation is to ship attachment
detection (at least R065 Phase 1 export) close to R054 so the window is short.

**Threat model.** No new exposure beyond R065's. Attachments are decrypted, base64-decoded,
and exported in the backend; only a non-secret result crosses to the WebView. The export path
mirrors `copy_password`. R054 already proved the legacy parse adds no write surface and
preserves the `Zeroizing`/wipe/sanitized-error discipline.

## Alternatives considered

- **Detect against the rendered body only (no `parse_legacy` change).** Cheapest: R065's
  case-insensitive detector runs on the legacy-rendered body unchanged. Works for the common
  case but accepts a wider misfire surface (a free-text body line `content-transfer-encoding:`
  looks like a marker) and bakes the lowercased headers on edit. Acceptable only if R065
  lands close enough that the interim is negligible.
- **Preserve attachment headers in `parse_legacy` (recommended).** `parse_legacy` recognizes
  `Content-Disposition` / `Content-Transfer-Encoding` and keeps them as structured markers
  (not lowercased body text), so a single detector serves both modern and legacy secrets.
  Small change to R054's render stance; makes R066 a re-use of R065 rather than a parallel
  implementation. The cost is a narrow exception to "render all headers as body," justified
  because attachments are the one header pair that carries operational meaning gpm must act
  on.
- **Co-ship R066 with R065.** Build legacy attachment support at the same time as modern, so
  the interim window never opens and the bake-on-edit problem never arises. Higher upfront
  cost but the cleanest outcome; the right call if R065 is scheduled soon.
- **Leave as-is and document as a known limitation.** Cheapest, but leaves a legacy
  attachment secret showing a wall of base64 with no affordance — a visible (if non-fatal)
  compat gap on stores that use attachments.

## Effort

~S (human) / ~S (CC) if R065 exists and the "preserve attachment headers in `parse_legacy`"
option is taken (R066 becomes a thin re-use of R065's detector/export). ~M if detection is
re-implemented against the rendered body, because the misfire-surface and bake-on-edit
interactions each need their own handling.

## Depends on / Supersedes

Depends on **R054** (legacy read, which produces the render this RFC detects against) and on
**R065** (modern attachment support, whose detector/export path this RFC reuses or parallels).
Related to, but narrower than, R065: R065 owns the attachment mechanism and the modern layout;
R066 owns only the legacy-render interaction R065 defers. Naturally exercised by the
live-binary interop tests (R053) driving a real gopass to write an attachment into a legacy
fixture.
