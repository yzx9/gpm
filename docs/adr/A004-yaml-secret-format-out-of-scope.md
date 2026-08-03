# A004: YAML Secret Format Out of Scope — Opaque-Body Compatibility

**Status:** Accepted

**Date:** 2026-08-03

## Context

gopass secrets come in three on-disk formats (verified against the gopass
source under `pkg/gopass/secrets/`): **AKV** (the modern default), **MIME**
(`GOPASS-SECRET-1.0`, legacy), and **YAML** (legacy, discouraged). For each,
gpm must decide whether to parse it as structured data or carry it as opaque
bytes. gpm already does structured handling for two:

- **AKV** — `Secret::parse` → `modern_split`: first line is the password, the
  rest is the body.
- **MIME** — `parse_legacy`: read-only normalization that lifts the `Password:`
  header into the password slot and renders the remaining headers as body text
  (mirroring gopass's own MIME→AKV normalization).

The open question is **YAML** — must gpm recognize its `---` document marker
and YAML data block, or can YAML secrets pass through as opaque body?

## Decision

**Do not implement YAML parsing.** YAML secrets fall through to `modern_split`:
password = first line, the `---` separator and YAML data block are carried as
opaque body text. gpm's gopass-format compatibility surface is therefore
**AKV (read + write) and MIME (read-only normalization); YAML is not parsed.**

## Why YAML is safe as opaque body

The decisive fact: **in both AKV and YAML the password is the first line.**
`modern_split` extracts it correctly. MIME is the different case — its password
lives in a `Password:` header, not the first line, so `modern_split` alone would
set the password to the magic string `GOPASS-SECRET-1.0` (and `copy_password`
would copy that magic to the clipboard). That correctness bug is exactly why
MIME required dedicated `parse_legacy` handling. YAML has no such hazard: the
first line is the password, identical to AKV, so there is no bug to fix.

The opaque-body path is also **lossless on round-trip.** A YAML secret on disk
is `<password>\n<body text>\n---\n<yaml>`. gpm parses password = first line,
body = everything else, including the `---` marker and the YAML block. The
editor's `reassemble(pw, body)` writes `${pw}\n${body}` back verbatim, so an
edit preserves the marker and YAML block and gopass re-reads the secret with
its full structured YAML intact. gpm does not corrupt YAML secrets — it merely
does not expose their fields as structured data. (`Secret::parse` trims trailing
whitespace and normalizes CRLF, so the round-trip is gopass-readable, not
byte-identical — consistent with gpm's stated bar of gopass-readability over
byte-identity.)

**gopass's own posture supports leaving YAML unparsed.** YAML is a discouraged
legacy format. gopass never writes new YAML secrets — `New()` and the create
wizard both produce AKV (`pkg/gopass/secrets/new.go`,
`internal/create/wizard.go`); there is no `NewYAML` write constructor, only a
read-side `ParseYAML`. Existing YAML secrets are preserved through edits (read
via `ParseYAML`, re-serialized via `Bytes()`), but, unlike MIME, are given **no
migration path**: MIME carries a `fromMime` provenance flag and fsck-driven
conversion to AKV; YAML has neither. The format is frozen in place, not winding
down via conversion. Aligning with gopass therefore means treating YAML as a
read-source gpm can safely pass through, not one it must model.

This also fits the attribute-region direction (consolidating secret handling on
AKV). Adding a YAML parser would move against both gopass's deprecation
trajectory and gpm's AKV-first model, and would import a YAML dependency with
its own manual-editing footguns (e.g. unquoted numeric strings parsed as octal)
for a dying format.

## Consequences

- **YAML secrets are safe in gpm.** The password extracts correctly and the data
  round-trips without corruption; a YAML secret remains gopass-readable after a
  gpm edit.
- **No structured field access for YAML.** A YAML secret displays its `---`
  marker and indented YAML as body text, not as named fields. The TOTP and
  attachment body scanners incidentally scan those lines too, so a `totp:` or
  `Content-Transfer-Encoding:` inside a YAML block may be detected by accident —
  tolerated, not guaranteed.
- **Known cosmetic edge case.** A YAML secret whose first line is the `---`
  marker itself (a password-less YAML document) shows `---` as the password.
  Rare, immediately visible, and still non-destructive: it round-trips as valid
  YAML.
- **The format-compat surface is now stated.** AKV (read + write) and MIME
  (read-only); future format questions resolve against this boundary. If
  structured YAML display ever becomes a real requirement, the preferred
  mitigation is a one-way "flatten to AKV on edit" normalize (drop the `---`
  block into the attribute region / body), not a full YAML parser. Revisit this
  ADR only then.

## Alternatives considered

- **Full YAML parser (read structured fields).** Rejected: gopass deprecated the
  format, never writes it, and offers no migration path; the structured benefit
  accrues only to users who hand-authored YAML years ago. It costs a YAML
  dependency plus its editing footguns, and cuts against the AKV-consolidation
  direction.
- **Read-only YAML recognition (mirroring MIME).** Rejected as unnecessary:
  MIME needed special handling because its password is not on the first line (a
  correctness bug). YAML's password is on the first line, so `modern_split`
  already handles it correctly — there is no bug to fix.
- **One-way flatten to AKV on edit (normalize YAML away).** Deferred, not
  rejected: it is the cheap mitigation if structured display is ever required,
  but it is not needed while opaque-body handling is both safe and lossless.

## Related

- [A002](A002-rust-first-without-gopass.md) — Rust-first, narrow-scope stance;
  age-only, no GPG. Declining a legacy format gopass itself discourages is the
  same direction.
- `crates/rustpass/src/secret.rs` — `modern_split` (AKV/YAML path) and
  `parse_legacy` (MIME path).
