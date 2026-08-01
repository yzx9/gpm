# Attachment Secret Support

**Priority:** P2
**Status:** Draft
**Phase:** Future

## What

gpm should round-trip gopass **binary attachments**: secrets whose decrypted body is a base64-encoded file, flagged by two `Content-Disposition` / `Content-Transfer-Encoding` attribute lines. Ship in phases — first an **export-to-file** path (decrypt + decode + write to a user-chosen location, never touching the WebView), then display of attachment metadata, then byte-compatible attachment **creation** on write. This replaces the RFC's earlier framing: gopass attachments were described in the original R054 draft as a `multipart/mixed` MIME envelope, which is inaccurate (see Context), and the first step is now the export path rather than parse-and-render.

Name the feature this RFC serves: there is no `docs/specs/NNN-*` for attachments yet; this RFC is the seed, and a spec should be opened when the display phase is scheduled.

## Why

gopass compatibility is a hard constraint, and binary attachments (`gopass fscopy` / `gopass cat`) are a documented, fully read/write feature a user can create today. A gpm user pointed at a store that uses them gets a silently degraded experience: the body shown is a wall of base64, the field gpm labels "password" is an empty line, and the attachment is invisible. Worse, any gpm **write** rewrites the secret as flat text and destroys the attachment — recoverable only from git history. The gap is invisible to gpm's own suite because gpm never produces attachments, so it only surfaces against a real gopass store (the live-binary interop tests, R053, would catch it concretely).

The export path is the highest-value first step because it is exactly what gopass itself treats as the primary attachment operation (`gopass fscopy <secret> <file>`): get the decrypted bytes out to a real file. It fixes the most common need (recover the attachment) without yet solving attachment display in the WebView or byte-compatible write, both of which are larger and carry more threat-model weight.

## Context

**How gopass actually encodes attachments (this corrects the prior multipart premise).** gopass never used `multipart/mixed`. A short-lived "MIME secret" format existed for roughly six months in 2020–2021 and was removed in gopass v1.13; even that format was a single RFC-822-style header block, not multipart. Searching gopass's history for `multipart` returns nothing — there is no boundary-based envelope at any point. The current and only attachment mechanism is layered on gopass's everyday text format (AKV): the decrypted plaintext of an attachment is an empty password line, two attribute lines (`Content-Disposition: attachment; filename="..."` and `Content-Transfer-Encoding: Base64`), and a body that is the standard base64 encoding of the original file. Detection on read keys off the `Content-Transfer-Encoding: Base64` attribute (matched case-insensitively); the reader strips the attribute lines, takes the body, and base64-decodes it back to the original bytes.

**Encryption is orthogonal and unchanged.** The AKV+base64 layout describes the _plaintext_ — what age decryption yields. age still encrypts the entire plaintext blob exactly as for any other secret; attachments are never stored unencrypted. base64 is _encoding_ (so binary can live inside a line-oriented text format), not encryption, and provides no secrecy. All of gpm's existing decrypted-content discipline — `Zeroizing`/wipe, sanitized errors, password-never-reaches-the-WebView — applies to attachment bytes identically.

**Where gpm stands.** gpm's decrypted-secret model is a first-line password plus an opaque body, with no attribute/structured-field awareness at all. For an attachment that means gpm sees an empty password and a body containing the two attribute lines plus the base64 — it has no way to tell the base64 is an attachment, and copy/show operate on that garbage. The export path needs only a narrow addition: detect the `Content-Transfer-Encoding: Base64` attribute, strip the attribute lines, decode the body, and hand the bytes to a file destination — without introducing a general attribute model.

**Threat-model notes.** The export path mirrors `copy_password`: bytes are decrypted, decoded, and written in the backend; only a non-secret success/failure result crosses to the WebView. This preserves the "decrypted bytes never reach the WebView" guarantee even for binary attachments, which an in-app attachment viewer would by definition violate (a viewer is therefore deferred to the display phase and out of scope here). Large attachments make the per-operation identity decrypt and the auto-clear lifecycle more expensive, which interacts with the Immediate-no-cache default; the export path inherits the same identity discipline as copy/show. Destination selection reuses the existing file-save capability (Android Storage Access Framework on mobile, a native save dialog on desktop) that already stages through a temp file and wipes the stage — the same pattern used for diagnostics export today.

**Detection must not misfire.** A plain secret whose body happens to contain the string `Content-Transfer-Encoding` must not be treated as an attachment. The detection signal is the structured attribute (a leading `Content-Transfer-Encoding: Base64` line in the attribute region), matching gopass's own check, not a free-text substring match.

**Phasing.** Phase 1 (this RFC's near-term scope): export-to-file, read-only, no WebView exposure of bytes, reusing the file-save capability. Phase 2: display attachment metadata (filename, size) and offer export from the detail view, still without an in-app binary renderer. Phase 3: byte-compatible attachment creation on write (regenerating the exact attribute lines and base64 body so gopass reads what gpm wrote) — a real serialization surface, deferred.

## Alternatives considered

- **Parse-and-render first (the original RFC direction).** Rejected as the first step: rendering binary in the WebView is the most expensive and most threat-model-sensitive piece, and it is not what gopass itself prioritizes — `gopass fscopy` is the primary attachment operation. Export delivers more value sooner and more safely.
- **Detect-and-warn only.** A cheap middle ground that stops the silent degradation, but still hides the attachment bytes the user actually wants. Acceptable as a fallback if export proves blocked on a platform, but not the target.
- **Full read + display + write in one step.** Rejected for now: the write-side byte-compatibility surface is large and the in-app viewer is threat-model-heavy. Phasing de-risks it.
- **Leave as-is and document as a known limitation.** Cheapest, but leaves the compat constraint unmet for any attachment-using store and fails silently.

## Effort

~M (human) / ~M (CC) for Phase 1 (export-to-file). ~M added for Phase 2 (metadata display). ~L for Phase 3 (byte-compatible write).

## Depends on / Supersedes

Naturally exercised by — and surfaced concretely by — the live-binary interop tests (R053). The original R054 draft conflated this with the legacy-format read gap under an inaccurate multipart premise; that gap is now its own RFC (R054), and this one covers modern attachments only.
