# Attachment Preview (in-app render of common image types)

**Priority:** P2
**Status:** Draft
**Phase:** Future
**Revision:** 1

## What

gpm should **preview common attachment types in-app** — initially raster images (JPEG, PNG, GIF, WEBP) — so a user can look at an attached image without first exporting it to a file. Today the attachment read side shows the filename, the decoded size, and an Export action, but no inline render. This RFC adds a controlled, user-initiated inline preview for a safe-image allowlist; non-image or unsupported types keep the Export-only affordance. This is the in-app preview the read side deferred for its trust-boundary weight.

Attachment access is scoped under [001 Entry Access](../specs/001-entry-access/prd.md); this RFC is the preview-side design, alongside the write RFC (R067).

## Why

For an image attachment, "export to a file, then open that file in another app" is a heavy detour — especially on mobile, where the exported file lands somewhere the user then has to find. An inline preview is the natural way to glance at an attached image, and it is the single biggest remaining UX gap for image attachments after the read/export side shipped. The cost is a trust-boundary decision the read side deliberately deferred (below); this RFC re-opens and resolves it.

## Context

**The trust-boundary crux.** The attachment feature rests on "decrypted bytes never reach the WebView," and the read side explicitly deferred an in-app viewer as threat-model-heavy. A preview necessarily renders bytes in the WebView — so it re-opens that question. The reframing: a preview is a **user-initiated, scoped reveal of one specific attachment**, exactly analogous to the existing reveal operation for text secrets, which already sends the decrypted password and notes to the WebView under a strict auto-clear + lifecycle-wipe + screen-capture-protected discipline. The real invariant is not "bytes never reach the WebView" absolutely; it is "bytes reach the WebView only as a deliberate, scoped, user-requested reveal with lifecycle cleanup." An image preview crosses the same line that text reveal already crosses, for image bytes instead of text bytes.

**Type safety.** Modern gopass attachments carry no `Content-Type`, so the type is inferred — from the `Content-Disposition` filename extension first, then magic-byte sniffing as a fallback (mirroring how the encode side sniffs). Preview is restricted to a **safe raster allowlist** (JPEG/PNG/GIF/WEBP): no SVG (script execution), no HTML, no format the WebView would interpret as code. Decoded dimensions and byte size are capped to bound decompression-bomb and memory risk. Anything unrecognized or unsupported falls back to the current Export-only affordance rather than attempting a render.

**Render approach.** Render the decoded bytes via a scoped, short-lived object/data URL in an image element, revoked and wiped on leave. The bytes cross to the WebView for the duration of the preview only; they are not logged, not persisted, and not left in the DOM after the user leaves or the session locks. This is the same scoped-reveal posture the text-reveal path already establishes.

**Lifecycle.** The preview inherits the text-reveal lifecycle verbatim: per-operation identity decrypt (no session-cached vault), the existing FLAG_SECURE screen-capture protection raised while the image is on screen, and wipe on leave / hard lock. Large-image memory is the same class of concern as the documented large-attachment memory limit on the read side (a streaming decode is the eventual fix, tracked separately) — a preview of a very large image is the most memory-expensive reveal and may need its own practical size guard.

## Alternatives considered

- **No preview (Export-only, as today).** Safest, but the export-to-view detour is the main complaint for image attachments and leaves the feature feeling half-finished. Acceptable as a fallback if the trust-boundary decision goes the other way, but not the target.
- **Native renderer (Android `ImageView`, desktop native image view).** Keeps bytes out of the WebView entirely — the strongest trust boundary — at the cost of a per-platform native rendering surface and a much larger build than a WebView image element. This is the right escalation if the threat model hardens; the scoped-WebView approach is the pragmatic first step precisely because text reveal already crosses the same line.
- **Backend-generated, downsampled thumbnail.** Decode + re-encode a smaller, sanitized image in the backend and send only that to the WebView — reduces what crosses (smaller, known-safe, re-encoded) at the cost of an image-decode dependency in the backend. A genuine middle ground if raw-bytes-in-WebView is unacceptable; more complex than the scoped-WebView render.
- **Tiny allowlist first (PNG/JPEG only), widen later.** Smallest initial attack surface; GIF/WEBP added once the lifecycle and guards are proven. Reasonable phasing regardless of which render approach is chosen.

## Effort

~M (human) / ~M (CC) for the scoped-WebView-image approach (allowlist, sniff, scoped URL + lifecycle, size guards). ~L if a native renderer or a backend-thumbnail path is chosen instead.

## Depends on / Supersedes

Builds on the shipped attachment read/export — needs attachment detection plus the existing screen-capture-claim and scoped-reveal lifecycle that the text-reveal path established. Independent of the write RFC (R067). The bytes-in-WebView reframing is the load-bearing decision and should be confirmed in the feature's security review before this is scheduled.
