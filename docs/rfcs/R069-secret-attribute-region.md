# Secret Attribute Region (model gopass AKV headers as a first-class field)

**Priority:** P2
**Status:** Draft
**Phase:** Future

## What

gpm should model gopass's **attribute region** — the `Key: Value` header lines that AKV secrets carry between the password and the free-text body — as a first-class part of `Secret`, instead of the current `password + opaque body` collapse. Today `Secret { password, body }` folds every attribute line into `body`, and each attribute consumer (TOTP presence, attachment detection, and — eventually — named-field metadata display) re-derives what it needs by re-scanning that body string. This RFC introduces a structured `attributes` representation so those consumers read from the model once, and so the legacy `GOPASS-SECRET-1.0` parse preserves attributes as structure instead of flattening them into body text.

This is a model-level refinement of the internal `Secret` representation that [001 Entry Access](../specs/001-entry-access/prd.md) relies on, not a standalone product feature; this RFC is its design home.

## Why

The attachment work exposed a single shared root cause: **gpm has no attribute / structured-field awareness.** The decrypted-secret model assumes exactly one layout — first line is the password, everything after is an opaque body — so it cannot tell an attribute line from a free-text note. That one gap has three visible consequences, all worked around separately:

1. **Modern secrets' `key: value` metadata** (gopass conventions like `user:`, `url:`, `note:`) is folded into `body` and shown to the user as one text blob, not as named fields.
2. **Attachments** are detected by `attachment::kv_value` re-scanning the body for `Content-Transfer-Encoding` / `Content-Disposition` on every call (`extract`, `has_attachment`, `metadata` each split the body again) — because the model has no attributes to read.
3. **Legacy `GOPASS-SECRET-1.0` attributes** are rendered as lowercased `key: value` body text by `parse_legacy`. The legacy-attachment case is the sharpest edge: once rendered, the attachment headers ARE body text, so the "must not misfire on free text" guarantee weakens and an edit bakes the lowercased headers in place.

A first-class attribute region dissolves all three: TOTP and attachment detection read `attributes`, metadata displays as named fields, and legacy attributes stay structured (the legacy-rendered-body detection problem never arises). gopass itself treats attributes as the load-bearing structure of a secret (AKV = Attribute-Key-Value); gpm aligning with that is the compatibility-conservative move, not a divergence.

## Context

**gopass's AKV layout.** A modern gopass secret is: line 1 = password; then zero or more `Key: Value` attribute lines; then the remaining free-text body. Verified against `~/git/gopass` `pkg/gopass/secrets/akv.go`: the parser keys off the `": "` separator — any line containing it is an attribute, every other line is body. It is **position-agnostic** (attributes are recognized anywhere, not only in a leading block), preserves insertion order, and allows duplicate keys (the legacy format leans on this — multiple `Password:` headers). `Body()` returns every non-`": "` line; `Get(key)`/`Set(key,val)`/`Keys()` are the attribute surface. Two gopass features gpm already cares about are _just attributes_: TOTP (`totp: otpauth://…`) and binary attachments (`Content-Disposition:` + `Content-Transfer-Encoding: Base64` + a base64 body).

**Where gpm stands.** `Secret { password: Zeroizing<String>, body: Zeroizing<String> }` (`crates/rustpass/src/secret.rs`). Both parse paths produce this shape:

- Modern `Secret::parse`: first line → `password`, the rest → `body` verbatim (attribute lines and notes together).
- Legacy `parse_legacy` (R054): parses the `GOPASS-SECRET-1.0` header block, lifts `Password:` → `password`, and renders every remaining header as a lowercased `key: value` line **into `body`**.

So attributes exist only in gopass's format; gpm neither models nor enforces them, and recovers them on demand. `attachment::kv_value` and `totp::has_totp` are two independent ad-hoc scanners over the same body string, each re-splitting every call.

**The load-bearing decision: what `body()` means after.** Today `body()` returns "everything past the password," including attribute lines. Giving `Secret` an attribute field changes that contract: `body()` becomes the free-text notes only, and a new `attributes()` accessor carries the structured fields. Every current `body()` consumer must be re-examined:

- **Display (`show_password`)** — currently hands the user the whole blob; afterward it shows `body()` (notes) plus named fields from `attributes()`. This is the user-visible win.
- **Edit (`reassemble`)** — currently reassembles `password + body`; afterward it must round-trip `password + attributes + body` so an edit does not drop the attributes (the same class of bug the attachment work already blocks: an edit destroying the attachment).
- **TOTP** — `totp::has_totp(body)` moves to "is there a `totp` attribute," reading the model instead of scanning.
- **Attachment** — `attachment::extract/has_attachment/metadata` move to reading the `Content-Transfer-Encoding` / `Content-Disposition` attributes from the model; the base64 payload becomes the body (or a derived view), not a re-split region.

**Round-trip / gopass compatibility.** The correctness bar stays **gopass-readability**, not byte-identity with gopass's output (as for the attachment read/write RFCs). Because gopass's reader is position-agnostic over `": "`, gpm is free to store attributes as a separate ordered collection and reassemble them in a conventional order (password → attributes → body) on write; gopass decodes it identically. Order and duplicate keys must be preserved on the read side so a round-trip through gpm does not collapse a legacy multi-`Password:` secret.

**Threat model.** No new exposure. Attributes are already-decrypted plaintext; the same `Zeroizing`/wipe, sanitized-error, and bytes-never-reach-the-WebView discipline applies unchanged. An attachment's decoded bytes still come from the body/payload; modeling the `Content-Disposition`/`Content-Transfer-Encoding` lines as attributes changes where the _signal_ is read, not where the _bytes_ live or how they cross to the export path.

**Why this is its own RFC, not folded into the attachment work.** Legacy attachment read and R067 (attachment write) are scoped to the attachment mechanism and deliberately work around the missing attribute model. This RFC is the cross-cutting foundation that _removes_ that workaround; it also reshapes how every modern secret displays, so it is larger than either and must not be smuggled into attachment work. Scheduling it dissolves the legacy-rendered-body detection problem and simplifies R067's write reassembly.

## Alternatives considered

- **Status quo (no attribute model).** The three consequences persist; each new attribute consumer (TOTP, attachments, future named-field display) re-invents a body scanner. Rejected as the long-term direction, though acceptable in the short term — the ad-hoc scanners work today.
- **Minimal: special-case the attachment headers in `parse_legacy`.** `parse_legacy` keeps `Content-Disposition` / `Content-Transfer-Encoding` as structured markers instead of lowercasing them into the body, so the modern detector serves legacy secrets unchanged. Fixes the legacy-attachment case alone; does nothing for modern metadata display or for stopping the body-rescan duplication. A narrow patch, not the model.
- **Full attribute model (this RFC).** Models the AKV attribute region as a first-class `Secret` field, unifying TOTP, attachment, modern-metadata, and legacy-attribute handling on one read. Higher upfront cost — it reshapes `Secret`, both parse paths, and every `body()` consumer — but it is the only option that closes the root cause instead of the symptoms.
- **Defer until a second non-attribute feature needs it.** Wait until, e.g., YAML-secret support is in scope before building a general structured-field model. Premature: two real consumers (TOTP, attachments) already pay the re-scan cost today, and the legacy-attachment case hinged on exactly this gap.

## Effort

~L (human) / ~L (CC). The data-model change itself is small (an ordered, duplicate-tolerant attribute collection on `Secret` + an `attributes()` accessor + a reassembler). The bulk is the fan-out: migrating both parse paths, auditing and migrating every `body()` consumer (display, edit/reassemble, TOTP, attachment), keeping the gopass round-trip green against the live-binary interop tests, and deciding the phasing so a modern secret never display-regresses mid-migration.

## Depends on / Supersedes

Builds on the shipped legacy read (the former R054) and the shipped attachment read side — both already parse attributes ad hoc, so this RFC formalizes what they re-derive. Legacy attachment read (the case that first exposed this gap) is already resolved by the case-insensitive attribute lookup; this RFC removes the broader workaround. **Simplifies R067** (attachment write): the write reassembles `password + attributes + body` against the same model instead of hand-building the attribute lines. Naturally exercised by the live-binary interop tests, which can drive a real gopass to confirm gpm round-trips attribute-bearing secrets. Independent of, but unifying for, R067 (attachment write) and the preview RFC (R068).
