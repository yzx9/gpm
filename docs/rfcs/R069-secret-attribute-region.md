# Secret Attribute Region (model gopass AKV headers as a first-class field)

**Priority:** P2
**Status:** Accepted
**Phase:** Next

## Progress

Phased in the code as **Phase 2a** (shipped) and **Phase 2b** (pending — the remaining work this RFC now tracks).

**Phase 2a — shipped** (`9b4cae2 refactor(secrets): model the AKV attribute region on Secret`). `Secret` carries an `attributes: Vec<Attribute>` field (byte-exact `Zeroizing<Vec<u8>>`, `Attribute { key, value }`), populated by `parse_attributes(body)` on **both** parse paths, with accessors `attributes()` / `get(key)` (exact, gopass `Get` parity) / `get_ci(key)` (case-insensitive) / `attribute_str(key)` / `is_attachment()`. The two read-side detectors migrated off body-scanning: TOTP reads `attribute_str("otpauth" | "totp")` (`totp.rs`), and attachment detection reads `Secret::is_attachment()` + `get_ci("Content-Disposition")` (`attachment.rs`). This closes consequence 2 (attachment detection) and the detection edge of consequence 3 at the read layer.

In Phase 2a `attributes` is a **derived view** over `body` — `body()` still returns the whole blob with attribute lines inline, and the detectors work only because the attributes stay recoverable from that blob.

**Phase 2b — pending.** Make `attributes` the source of truth instead of a derived view:

1. **Split `body()`** to return free-text notes only (every `Key: Value` line leaves the body and lives in `attributes`); gopass `Body()` parity.
2. **Display (`show_password`)**: surface `attributes` as named fields instead of dumping the raw blob into `notes`. The user-visible win — consequence 1.
3. **Edit/reassemble**: round-trip `password + attributes + body` explicitly (frontend reassembly + `Secret::to_bytes`), so an edit can no longer silently drop attribute lines. Today it survives only because attributes are inline in `body`.
4. **`parse_legacy` root cause**: stop lowercasing headers into `body`; preserve them as structured `Attribute`s, dissolving consequence 3 at the source (currently worked around via `get_ci`).

## What

gpm should model gopass's **attribute region** — the `Key: Value` header lines that AKV secrets carry between the password and the free-text body — as a first-class part of `Secret`, instead of a `password + opaque body` collapse. **Phase 2a** shipped the structured `attributes` field and migrated the TOTP/attachment detectors off body-scanning (see Progress); **Phase 2b** — the remaining work — makes `attributes` the source of truth: `body()` returns free-text notes only, display shows named fields, edit round-trips attributes explicitly, and the legacy `GOPASS-SECRET-1.0` parse stops flattening headers into body text.

This is a model-level refinement of the internal `Secret` representation that [001 Entry Access](../specs/001-entry-access/prd.md) relies on, not a standalone product feature; this RFC is its design home.

## Why

The attachment work exposed a single shared root cause: **gpm had no attribute / structured-field awareness.** The decrypted-secret model assumed exactly one layout — first line is the password, everything after is an opaque body — so it could not tell an attribute line from a free-text note. That gap had three visible consequences, each worked around separately (2 and the detection edge of 3 are now resolved at the read layer by Phase 2a; 1 and 3's root cause remain for Phase 2b — see Progress):

1. **Modern secrets' `key: value` metadata** (gopass conventions like `user:`, `url:`, `note:`) is folded into `body` and shown to the user as one text blob, not as named fields.
2. **Attachments** were detected by re-scanning the body for `Content-Transfer-Encoding` / `Content-Disposition` on every call (`extract`, `has_attachment`, `metadata` each split the body again) — because the model had no attributes to read.
3. **Legacy `GOPASS-SECRET-1.0` attributes** are rendered as lowercased `key: value` body text by `parse_legacy`. The legacy-attachment case is the sharpest edge: once rendered, the attachment headers ARE body text, so the "must not misfire on free text" guarantee weakens and an edit bakes the lowercased headers in place.

A first-class attribute region dissolves all three: TOTP and attachment detection read `attributes`, metadata displays as named fields, and legacy attributes stay structured (the legacy-rendered-body detection problem never arises). gopass itself treats attributes as the load-bearing structure of a secret (AKV = Attribute-Key-Value); gpm aligning with that is the compatibility-conservative move, not a divergence.

## Context

**gopass's AKV layout.** A modern gopass secret is: line 1 = password; then zero or more `Key: Value` attribute lines; then the remaining free-text body. Verified against `~/git/gopass` `pkg/gopass/secrets/akv.go`: the parser keys off the `": "` separator — any line containing it is an attribute, every other line is body. It is **position-agnostic** (attributes are recognized anywhere, not only in a leading block), preserves insertion order, and allows duplicate keys (the legacy format leans on this — multiple `Password:` headers). `Body()` returns every non-`": "` line; `Get(key)`/`Set(key,val)`/`Keys()` are the attribute surface. Two gopass features gpm already cares about are _just attributes_: TOTP (`totp: otpauth://…`) and binary attachments (`Content-Disposition:` + `Content-Transfer-Encoding: Base64` + a base64 body).

**Where gpm stands.** `Secret { password: Zeroizing<Vec<u8>>, body: Zeroizing<Vec<u8>>, attributes: Vec<Attribute> }` (`crates/rustpass/src/secret.rs`). Phase 2a added `attributes` as a **derived view** over `body` — both parse paths still produce `password + body` bytes, then `parse_attributes` derives the attribute list from `body`; Phase 2b flips it to the source of truth.

- Modern `Secret::parse`: first line → `password`, the rest → `body` verbatim (attribute lines and notes together).
- Legacy `parse_legacy` (the former R054): parses the `GOPASS-SECRET-1.0` header block, lifts `Password:` → `password`, and renders every remaining header as a lowercased `key: value` line **into `body`** — the flattening Phase 2b removes.

So attributes are now modeled, but only as a recoverable view: `body()` still carries them inline, and `parse_legacy` still flattens headers into `body`. Phase 2a already moved TOTP and attachment detection off the old body scanners (`attachment::kv_value` is gone — detection is `Secret::is_attachment()` / `get_ci`); Phase 2b is the body split + display + edit + `parse_legacy` work.

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
