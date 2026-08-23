# Attachment Write (byte-compatible create/replace)

**Priority:** P2
**Status:** Draft
**Phase:** Future
**Revision:** 1

## What

gpm should let a user **attach a file to a secret** — create a new attachment entry or replace an existing attachment's bytes — regenerating the exact gopass attachment plaintext so gopass reads what gpm wrote. This closes the round-trip: today gpm detects, displays metadata for, and exports attachments, but cannot create or change them. The reverse of the export path — pick a file, encode, write.

Attachment access (reading/exporting a binary attachment) is scoped under [001 Entry Access](../specs/001-entry-access/prd.md); this RFC is the write-side design (create/replace), alongside the preview RFC (R068).

## Why

gopass compatibility is a hard constraint, and `gopass binary attach` / `fscopy` is a fully read/write feature. Today gpm is **read-only for attachments**: a gpm-only user cannot create one, and a mixed gpm/gopass user who attaches a file on desktop gets no round-trip on mobile. The gap is invisible to gpm's own suite (gpm never produces attachments), so it only shows against a real gopass store (the live-binary interop tests).

A second motivation: the read side already blocks text-editing an attachment (because a text save destroys it). That block exists precisely because there is no correct replacement path yet — this RFC is that path. "Replace attachment" is a byte-compatible re-encode, not a text edit.

## Context

**The encode mirror.** A modern gopass attachment's plaintext is an empty password line, a `Content-Disposition: attachment; filename="…"` line, a `Content-Transfer-Encoding: Base64` line, and a single-line base64 body. gopass's encode path (`secFromBytes`) always base64-encodes — there is no raw/text branch and no `Content-Type` in the modern format (a brief text-vs-base64 branch existed in the legacy MIME era and was removed). gpm's write mirrors this exactly: empty password line, the two attribute lines, base64 of the picked bytes. The correctness target is **gopass-readability**, not byte-identity with gopass's output — gopass's reader is tolerant, so as long as gpm emits the CTE attribute and a decodable base64 body, gopass decodes it.

**Reuses existing plumbing; bytes stay backend-side.** The write is the export path in reverse and inherits the same invariants. The picked file's bytes are read into the backend by the existing pick-and-read file plugin (the read-only sibling of the export's file-save plugin), base64-encoded in the backend, and handed to the existing store-write + autosync (pull → write → push) pipeline that create/edit/delete already use. Decrypted-or-\_about-to-be-encrypted attachment bytes never reach the WebView; only a non-secret write outcome crosses to the UI. The per-operation identity lifecycle, the at-rest encryption, the divergence-resolve prompt, and the documented "save built on an out-of-date read can fast-forward over a newer remote" caveat all apply unchanged.

**Create vs replace.** Create makes a new attachment entry from a picked file (the inverse of export). Replace re-encodes an existing attachment entry's bytes (the correct alternative to the text editor, which the read side already refuses for attachments). Both produce the same on-disk plaintext shape; the only difference is whether the entry path already exists. The UI affordance lives alongside Export on the attachment entry (and as an "attach a file" option in create), gated on the attachment-awareness the read side already provides.

**Threat model.** No new exposure beyond the existing write path: the picked bytes are encoded and written in the backend under the same identity discipline as any save. The picked file is the user's own; there is no untrusted-input injection surface (the encoded body is base64, which carries no executable content; the filename is sanitized as the export path already does). A replace is recoverable in git history like any write.

## Alternatives considered

- **Text-edit only, no attachment write.** Leaves gpm unable to create or replace attachments — a permanent gopass-compat gap and leaves the "replace" use case with no correct path. Rejected.
- **Round-trip via an external tool (shell out to gopass).** Rejected: Android can't run gopass, and gpm owns its own write pipeline.
- **Add a `Content-Type` / use the legacy text-vs-base64 branch.** Rejected — diverges from modern gopass, which always base64 and carries no `Content-Type`; emitting extra structure gopass doesn't produce would be a one-sided extension, not compatibility.

## Effort

~M (human) / ~M (CC). The encode is small; the bulk is the create/replace affordance, distinguishing replace from text-edit, wiring the write pipeline + autosync, and the byte-compat tests.

## Depends on / Supersedes

Builds on the shipped attachment read/export (detection, metadata, export). Naturally exercised by the live-binary interop tests, which can drive a real gopass to verify gpm-written attachments round-trip. Independent of, but complementary to, the preview RFC (R068).
