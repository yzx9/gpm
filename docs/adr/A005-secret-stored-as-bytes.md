# A005: Secret Stored as Bytes (UTF-8 as a Layered View)

**Status:** Accepted

**Date:** 2026-08-04

## Context

`Secret` held its password and body as `Zeroizing<String>`, and `Secret::parse`
ran `String::from_utf8_lossy(content)` on the decrypted bytes. That lossy
conversion is a latent **data-loss bug**: any secret whose plaintext is not
valid UTF-8 (a body holding raw bytes, a legacy-encoded note, a non-ASCII
password) is silently mangled on read, and the mangled text is re-encrypted on
the next edit-save — the original bytes are gone, recoverable only from git
history.

gopass does not have this problem because a Go `string` is a byte slice — it
carries arbitrary bytes and never coerces UTF-8. gpm's choice of `String`
storage was therefore a divergence from gopass's byte semantics, not parity.
The on-disk format itself is byte-oriented: lines split on `0x0A`, attributes on
the `": "` separator (`[0x3A, 0x20]`), CRLF normalization, trailing-whitespace
trim — all ASCII-safe operations that need no UTF-8 validity.

Two `String`-based alternatives were unworkable: keeping `from_utf8_lossy`
(the status quo — corrupts), or erroring on non-UTF-8 (bricks the read of one
weird secret, and a read failure must never block access to the store).

## Decision

**Store `Secret` as `Zeroizing<Vec<u8>>`** (password and body), with UTF-8 as a
**layered view**, never the storage type.

- Parsing is **byte-oriented** (`normalize_bytes`, `modern_split_bytes`): no
  `from_utf8_lossy`. Non-UTF-8 modern secrets round-trip byte-exact through
  `Secret::to_bytes`.
- `password()` / `body()` return `&str` via `std::str::from_utf8(...).unwrap_or("")`
  — identity for the valid-UTF-8 majority (so the ~46 existing assertions and
  all `&str` consumers are unchanged), empty when the bytes aren't valid UTF-8.
  Byte-exact accessors `password_bytes()` / `body_bytes()` serve the cases that
  must never be lossy.
- **Non-UTF-8 secrets are edit-blocked** (`Secret::is_utf8()` →
  `SensitiveContent.edit_blocked` / `EntryProbe.edit_blocked`): the lossy view
  is display-only and never written back, which is the actual corruption fix.
- The **IPC boundary stays `String`** (the WebView speaks UTF-8); bytes live in
  Rust, UTF-8 is the view that crosses to the frontend.

## Consequences

- **The corruption bug is fixed.** A non-UTF-8 secret reads lossy (display only,
  edit-blocked) and round-trips byte-exact through sync; editing it is blocked
  rather than silently destroying the original bytes.
- **UTF-8 becomes explicit, not assumed.** Text consumers (`attachment`,
  `totp`, display, IPC) call the `&str` views; anything byte-sensitive
  (`to_bytes`, future binary handling) uses `*_bytes()`.
- **Known limitation:** the legacy `GOPASS-SECRET-1.0` parser remains
  text-based (`parse_legacy` works on a `&str`), so a non-UTF-8 _legacy_ secret
  is still lossy on read and reports `is_utf8() == true`. Legacy MIME secrets
  are text (mid-2020–v1.13) and non-UTF-8 among them is an edge of an edge;
  modern secrets — the realistic non-UTF-8 case — are byte-faithful. A
  byte-oriented `parse_legacy` would close this; deferred.
- **Phase 2 builds on this.** The attribute-region work (R069, now shipped) adds
  `Attribute { key, value: Zeroizing<Vec<u8>> }`, the `SecretBody` attachment
  variant, and `get` / `get_ci` — all bytes-native from the start, because the
  storage type is already bytes.

## Alternatives considered

- **Keep `String` + `from_utf8_lossy` (status quo).** Rejected: silent
  corruption of non-UTF-8 secrets is a data-loss bug for a password manager.
- **`String` + error on non-UTF-8.** Rejected: a read must never fail on one
  weird secret — it blocks access to the store and diverges from gopass, which
  reads the same bytes fine.
- **Bytes for body only, `String` for password/attributes.** Rejected as
  inconsistent: a password or attribute value can be non-UTF-8 too, and a
  split storage model is more code, not less. One bytes-native model with
  UTF-8 views is simpler and uniform.

## Related

- [A004](A004-yaml-secret-format-out-of-scope.md) — YAML format out of scope
  (the other Secret-model scoping decision).
- `crates/rustpass/src/secret.rs` — the bytes-native `Secret`, `to_bytes`,
  `is_utf8`, and the `non_utf8_*` tests pinning the round-trip.
- The attribute region (phase 2, shipped) is built on this bytes-native
  foundation.
