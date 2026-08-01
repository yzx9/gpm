# Read Deprecated GOPASS-SECRET-1.0 Secrets

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

gpm should **read** (display correctly) the deprecated `GOPASS-SECRET-1.0` secret format that gopass wrote for a window in 2020–2021 and still reads today — **read-only, never write**. Today gpm silently misreads these: it treats the `GOPASS-SECRET-1.0` magic line as the password and shows the header block as the body, so copy-password copies the wrong string. The fix detects the magic first line, extracts the password from the `Password:` header where this format stores it, and renders the rest sensibly. This is the compatibility gap flagged separately from attachment handling (R065): the two share a root cause (gpm has no attribute/structured-field awareness) but are independent to ship.

## Why

gopass compatibility is a hard constraint, and gopass itself still reads `GOPASS-SECRET-1.0` — its parser tries the legacy format _first_, before YAML and the modern text format. Any store created or last edited during the roughly six months gopass emitted this format (mid-2020 to v1.13, January 2021) can still contain such entries; they survive untouched because gopass only converts on write. A gpm user opening one of these gets a broken experience with no warning: the displayed password is the literal `GOPASS-SECRET-1.0`, and copying it fails silently. Because gpm never produces the format, its own test suite never sees it — the gap only shows against a real older store (the live-binary interop tests, R053, would surface it).

The scope is deliberately **read-only**. gopass removed the writer in v1.13 and never emits the format again; gpm matching that means gpm also never writes it — an edit through gpm normalizes the secret to the modern text format on save, exactly as gopass does. There is no write-side compatibility to preserve.

## Context

**The format.** `GOPASS-SECRET-1.0` is a single-part RFC-822-style document, _not_ multipart and _not_ the everyday first-line-is-password layout. Its shape is: a magic first line (the literal `GOPASS-SECRET-1.0`), then a header block of `Key: Value` lines terminated by a blank line (case-insensitive keys, folded continuation lines per RFC 822), then a single body. The password is carried as a `Password:` **header**, not as the first line; other headers are named attributes; the body is free text. There are no attachments in this format — binary attachments (R065) are a separate, modern mechanism and never appear here.

**How gopass reads it.** gopass's parse step is a cascade: try legacy `GOPASS-SECRET-1.0` first, then YAML, then the modern text format (which always succeeds). On the legacy branch it recognizes the magic first line, parses the header block, lifts the `Password:` header into the password slot, lowercases the remaining header keys into attributes, and takes everything after the blank line as the body. The result is the same in-memory shape as a modern secret, tagged as converted-from-legacy. The format is explicitly documented in gopass as deprecated; only this read path survived the v1.13 cleanup.

**Where gpm stands and the shared root cause.** gpm's parse step assumes exactly one layout: first line is the password, everything after is an opaque body. It has no attribute/structured-field awareness at all. This single gap has two visible consequences: legacy `GOPASS-SECRET-1.0` secrets are misread (this RFC), and modern text secrets that carry `Key: Value` attribute lines have those attributes silently folded into the body. The two are the same underlying missing capability. This RFC therefore defines a **minimal** read fix that does _not_ require building a general attribute model: detect the magic, lift the `Password:` header to the password, and render the remaining headers plus body as the body text — enough to make copy/show correct without committing yet to structured attribute display. A fuller fidelity option (introduce an attribute model so legacy headers and modern attribute lines render as named fields) is larger, is shared with modern-secret display, and is left as an explicit alternative rather than folded in.

**Detection and false positives.** The discriminator is the trimmed first line equaling the literal `GOPASS-SECRET-1.0` — the same signal gopass uses. A real password is never this literal, so the risk of misclassifying a normal secret is negligible, and matching gopass's own discriminator is the correctness target regardless.

**Threat model.** No new exposure. This is a different parse of already-decrypted plaintext; the same `Zeroizing`/wipe, sanitized-error, and password-never-reaches-the-WebView discipline applies unchanged. Read-only support adds no write surface.

**Why a cascade, eventually.** gpm's single-layout parser is the deeper issue this gap points at: gopass has a multi-format cascade and gpm does not. The minimal fix handles `GOPASS-SECRET-1.0` as a special case at the front of the existing parser; a cleaner later step — its own decision, not assumed here — is to give gpm a real format-dispatch cascade mirroring gopass's, so legacy, YAML, and the modern format each get a first-class reader. That architectural step is only worth taking once a second non-default format (YAML) is actually in scope; today only the legacy format is.

## Alternatives considered

- **Full attribute model now.** Render legacy headers (and modern `Key: Value` lines) as structured named fields. Higher fidelity and fixes the shared root cause, but it is a larger change that also rescopes how every modern secret displays, and it is not necessary to stop the silent password corruption. Defer; take the minimal extraction now.
- **Detect-and-warn only.** Surface that a secret is the legacy format without parsing it. Cheaper than extraction, but copy-password still yields the wrong string — the most damaging symptom remains. Extraction is barely more work and actually fixes it.
- **Generalize to a format cascade before handling this.** Build gpm's multi-format dispatcher first, then add the legacy reader as one branch. Over-engineered for a single deprecated format; the special-case read is small, and the cascade only earns its keep when YAML support is also wanted.
- **Leave as-is and document as a known limitation.** Cheapest, but leaves a silent wrong-password bug on real stores and undercuts the gopass-compatibility claim.

## Effort

~S (human) / ~S (CC) for the minimal read fix (detect magic, lift `Password:` header, render rest as body). ~M–L if expanded to the full attribute model (shared with modern-secret attribute display).

## Depends on / Supersedes

Shares its root cause — gpm's lack of attribute/structured-field awareness — with modern-secret attribute display, but neither blocks the other. Naturally exercised by the live-binary interop tests (R053), which can drive a real gopass to emit a `GOPASS-SECRET-1.0` fixture. Related to, but independent of, attachment handling (R065): attachments never appear in the legacy format.
