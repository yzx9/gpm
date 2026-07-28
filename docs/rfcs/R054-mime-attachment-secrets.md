# MIME / Attachment Secret Gap

**Priority:** P2
**Status:** Draft
**Phase:** Future

## What

gpm models a decrypted secret as opaque text — the first line is the password, everything after it is the body — with no MIME awareness. gopass stores rich secrets as a MIME multipart envelope (primary content plus typed attributes and binary attachments). Record this gap and the design direction for first-class MIME handling, starting with read-only support so gpm displays a gopass MIME secret correctly instead of dumping raw MIME at the user.

## Why

gopass compatibility is a hard constraint, and MIME secrets are a documented gopass feature a user can create today. A gpm user pointed at a gopass store that uses them gets a silently degraded experience: the body shown is raw `Content-Type` headers and boundaries, the field the app labels "password" may actually be a MIME boundary line, and attachments are invisible. There is no warning that the secret is a format gpm does not understand. The gap is invisible to gpm's own test suite because gpm never produces MIME, so it only surfaces against a real gopass store.

## Context

**How gopass encodes rich secrets.** gopass wraps a multi-part secret in a MIME `multipart/mixed` envelope: one part is the primary secret (the line gopass treats as the password), and subsequent parts are named attachments, each with its own headers (content type, disposition, filename). The on-disk object is the age-decrypted plaintext in this shape.

**Where gpm stands.** gpm's secret parse splits the decrypted plaintext on the first newline and treats the remainder as an opaque body. There is no MIME or multipart handling anywhere in the crate. A MIME envelope therefore round-trips through gpm as literal text, and a gpm _write_ would destroy a MIME secret by rewriting it as flat text.

**Design direction (read-only first).** A parse step that detects the MIME envelope and exposes three things: the primary secret (so the existing password copy/show path keeps working unchanged), the named attributes, and the downloadable attachments. The detection boundary matters — a plain `password\nbody` secret must not be misread as MIME. Read-only support is the high-value, smaller step: it fixes display and copy without touching the write path.

**Read/write parity is the harder second step.** Round-tripping a MIME secret on write must regenerate a byte-compatible envelope so gopass still reads what gpm wrote, including attachment headers and ordering. That is a real serialization surface and is deferred.

**Threat-model notes.** Attachments are decrypted content — they fall under the same `Zeroizing`/wipe discipline and the never-reaches-the-WebView copy discipline as passwords today. Binary attachments also stress the "password never reaches the WebView" guarantee, since an attachment viewer would by definition render decrypted bytes; a read-only path that copies attachment bytes through Rust (like `copy_password`) avoids that. Large attachments make the per-operation identity decrypt and the configurable auto-clear lifecycle more expensive, which interacts with the Immediate-no-cache default.

## Alternatives considered

- **Leave as-is and document as a known limitation.** Cheapest, but it leaves the compat constraint unmet for any attachment-using store and the failure is silent rather than signposted.
- **Detect-and-warn only.** Surface that a secret is MIME without parsing it — a cheap middle ground that at least stops the silent degradation, but still hides attachments and attributes.
- **Full read/write MIME support in one step.** Rejected for now: the write-side byte-compatibility surface is large and read-only delivers most of the user value first.

## Effort

~M (human) / ~M (CC) for read-only MIME support. ~XL for full read/write with byte-compatible envelope regeneration.

## Depends on / Supersedes

None. Naturally exercised by — and would be surfaced concretely by — the live-binary interop tests.
