# A004: YAML Secrets — Read-Only Display + Lossless-Only Migration to AKV

**Status:** Accepted

**Date:** 2026-08-03 · revised 2026-08-14

## Context

gopass secrets come in three on-disk formats (verified against the gopass source
under `pkg/gopass/secrets/`): **AKV** (the modern default), **MIME**
(`GOPASS-SECRET-1.0`, legacy), and **YAML** (legacy, discouraged). gpm does
structured handling for AKV (read + write) and MIME (read-only normalization
into AKV).

**The original 2026-08-03 decision** was to not implement YAML parsing at all: a
YAML secret fell through `modern_split` — password = first line, the `---`
marker and YAML block carried as opaque body text. That was safe while the
editor reassembled `password + body` verbatim, and aligned with gopass's own
posture: gopass never writes new YAML secrets and gives the format no migration
path (unlike MIME, which gets a one-way `fromMime` conversion via fsck).

**It stopped being safe with R069's attribute region.** `split_attrs` extracts
every `": "` line as an attribute, position-agnostic — including `k: v` lines
*inside a YAML block*. Combined with `to_bytes`'s canonical reorder (password →
attributes → body), an edit through gpm rewrites a YAML secret into a shape
gopass re-reads with a different type and with YAML fields demoted to body text
— silent corruption on edit.

Verified against the gopass source (`pkg/gopass/secrets/{akv,yaml}.go`,
`secparse/parse.go`):

- gopass's cascade parses a `---`-bearing secret as **YAML, not AKV** — the
  `---` marker routes it through `ParseYAML` before the AKV fallback. gpm
  treating it as AKV was divergence, not parity.
- YAML→AKV is **lossy in general**: nesting, typed scalars, arrays, and the
  `---` marker itself are all lost. Only a flat map of string→string scalars
  converts without loss.

Two facts shape the fix:

1. The corruption **cannot occur unless gpm writes the secret back** — blocking
   the edit path eliminates the bug without touching the write path.
2. The original concern was a YAML parser **in the read path** (decoding every
   secret, octal footguns on manual edit). That concern can be honored while
   still adding a parser, by **quarantining** the parser to a one-way migration.

## Decision

YAML stays out of scope for structured access — no parsed YAML display, no YAML
fields, no YAML write path — and is handled as **read-only display + opt-in,
lossless-only migration**:

1. **Detection (read path, parser-free).** A secret containing a line beginning
   with `---` is treated as YAML. A strict byte check — no YAML decode on the
   read path.
2. **Read-only display.** A detected YAML secret is shown read-only via a new
   `EditBlockReason` mirroring the existing `NonUtf8` path. The password — the
   first line, unless the first line is itself `---` (a bare YAML document) —
   remains copyable through `copy_password`; the rest is an opaque text view.
   gpm never writes a YAML secret back, so the edit-time corruption is
   impossible.
3. **Migration (user-initiated; the only place a YAML parser runs).** A
   migrate-to-AKV action parses the `---` block. If the **entire** YAML is a
   flat map of string→string scalars — no nesting, no arrays, no non-string
   scalars (numbers/bools/null), the only subset that converts without loss —
   it is rewritten in place as AKV (attributes from the flat pairs, the password
   preserved). Any nesting, array, or non-string scalar **aborts** the migration
   (all-or-nothing per secret) with a prompt to edit the secret in gopass
   instead. The parser must be YAML **1.2** (bare `0123` stays a string, not
   octal) and must be matched on node variants — never coerced — so non-strings
   are rejected rather than silently mangled.

The YAML parser is a **migration-only dependency** (a maintained MIT/Apache
YAML 1.2 crate), never on the read path.

## Consequences

- **The edit-time YAML corruption is eliminated** by removing the edit path for
  YAML, not by repairing the write path.
- **A YAML crate becomes a gpm dependency**, confined to the migration command.
- **Rich YAML — the common real-world case — cannot migrate.** Such secrets stay
  read-only with a prompt pointing the user to gopass. Intentional: lossy
  migration would silently mangle data, and gpm declines to model YAML
  structure.
- **Known cosmetic divergence.** Strict `---` detection treats a small set of
  `---`-bearing non-YAML secrets as read-only where gopass treats them as
  editable AKV. Recoverable (edit in gopass), and self-diagnoses at migration
  time when the parser finds no valid YAML mapping.
- If structured YAML display ever becomes a real requirement, the answer remains
  "not a full YAML parser on the read path" — revisit this ADR only then.

## Alternatives considered

- **Opaque-body passthrough (the original stance).** Correct only while writes
  preserved the body verbatim; broken by the attribute region. Preserved here in
  spirit (read-only, no parsing) with the edit path closed.
- **Round-trip fidelity via raw-retention (a `Secret` that stores the original
  bytes and writes them back verbatim).** Deferred: a larger `Secret`-model
  refactor that also changes AKV line-order behavior; read-only + migrate fixes
  the YAML corruption at lower cost. (It remains the candidate fix for the
  separate AKV interleaved-line reorder issue, out of scope here.)
- **Structured YAML read/edit (full gopass parity: a YAML type with a parsed
  data map).** Rejected: reopens the structured-access question this ADR closes,
  needs a read-path parser, and the edit UI has no model for nested YAML — for a
  format gopass itself no longer writes.
- **Lossy migration (flatten nesting with dotted keys).** Rejected: irreversible
  and silently loses structure/types.

## Related

- [A002](A002-rust-first-without-gopass.md) — narrow-scope stance; declining to
  model a deprecated format is the same direction.
- [A005](A005-secret-stored-as-bytes.md) — the bytes-native `Secret` this
  builds on.
- gopass `pkg/gopass/secrets/{akv,yaml}.go`, `secparse/parse.go` — the cascade
  and format models gpm stays compatible with.
