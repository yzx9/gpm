# Encrypt an export to a recipient (age/GPG)

**Priority:** P2
**Status:** Draft
**Phase:** Future
**Revision:** 1

## What

A cross-cutting capability — independent of any one export feature — that **encrypts an export
artifact to one or more recipients** using asymmetric crypto, so only a holder of a matching
private key can read it. It applies to every export: the diagnostics-log archive, a repository
export (R078), and a whole-application export (R088). **age is the primary mechanism** (its
recipient model, including SSH recipients); **GPG is secondary** (for GPG recipients). **No
signing.**

This RFC also owns the **safety rules for including a repository's secrets** in an export (the
`secrets` slot R088 reserves): a key that already carries a passphrase may be exported as-is with
a warning; a key with no passphrase (the default native age identity) may be exported only when
the export is encrypted; PATs are never exported.

## Why

Exports carry sensitive material — logs that may name entries or settings, repository metadata
(entry paths, commit messages), and optionally identities/credentials. Encrypting an export to a
recipient hides all of that from anyone lacking the key, which is what makes it safe to hand an
export to a developer (logs), transfer a vault to a partner, or keep a backup in a shared
location. age's recipient model makes this natural, and age's native SSH-recipient support lets
gpm encrypt to a _public_ SSH key — the mechanism behind a one-tap "encrypt to the developer"
option for logs. The capability also closes the safety gap on the whole-app export's optional
`secrets` slot: a raw (no-passphrase) identity is only ever exported behind encryption.

## Context

**Outer envelope.** Encryption wraps the _entire_ export artifact as an outer layer. It hides
repository metadata (which a bare bundle exposes) and gives logs their only confidentiality. The
encrypted file is self-identifying only at the minimum needed — its nature plus how to decrypt —
and the payload is fully opaque until decrypted (an age-encrypted file's native header plus a
recognizable extension signal "this is an encrypted gpm export"; a tiny unencrypted type/version
stub is optional). No content leaks.

**age primary, GPG secondary; no signing.** age encryption is the default path. age accepts an
age recipient (`age1…`) or an SSH recipient (`ssh-ed25519` / `ssh-rsa`), and gpm's age dependency
already enables SSH recipients — so "encrypt to a public SSH key" works out of the box. GPG
encryption is offered where a GPG recipient is the natural choice (a GPG store's own key). age
gives confidentiality and integrity (AEAD) but not sender-authenticity; **signing is out of
scope** — the use cases (backup-to-self, logs-to-developer, transfer-to-partner) need
confidentiality, not proof of author.

**Recipient selection, with sensible defaults and manual paste.**

- **Logs** default to "the developer" (below) and accept a manually-pasted recipient.
- **Repository exports** default to the repository's _own_ recipient (encrypt-to-self — the owner
  can decrypt their own backup); a GPG store defaults to its GPG key, an age store to its age
  recipient. They accept a manually-pasted recipient.
- **Multiple recipients** are supported (e.g. self + partner) — age and GPG both encrypt to N
  recipients, any one of which can decrypt.
- **Manual paste** of an age/SSH/GPG public key is always available, so a user can target anyone.

**"Encrypt to developer" — bundled, offline, trusted.** The developer's public key(s) ship _in
the app_, not fetched at export time, so the option is offline, immune to network/MITM tampering,
and rotated on app release (like a pinned CA key). This makes "send my logs to the developer" a
safe one-tap action with no trust-on-first-use footgun. (Fetching from a host for freshness is a
possible later addition, but only behind pinned-fingerprint verification.)

**Secrets-safety rules (govern R088's per-repository `secrets` slot).** When an export includes a
repository's identity or credentials:

- A key that **already has a passphrase** (GPG S2K secret key, an age-encrypted identity) may be
  included **as-is** — the passphrase is its protection — with a clear risk warning.
- A key with **no passphrase** (the default native age `X25519` identity; post-quantum age keys)
  may be included **only when the export is encrypted** to a recipient; a raw key never leaves
  unencrypted.
- A **PAT** (a bearer token, not a key; no passphrase) is **never** included — the user re-creates
  it on restore.
- The **master/vault key** is device-bound and never exported.

These rules live here because they depend on the encryption capability: the no-passphrase case is
only safe behind this RFC's encryption.

**Leak surface with encryption.** Encrypted, an export exposes nothing but its existence and size
until decrypted. Without encryption, the leak surface is whatever the inner artifact already has
(R078: repository metadata = the remote's; logs: already-redacted content). Encryption is
additive confidentiality, not a change to the inner artifact's own threat model.

## Alternatives considered

- **Fetch the developer's key from a host at export time.** Rejected as the default: it adds a
  network dependency and a trust/TOCTOU window (a compromised host or network serves an
  attacker's key). Bundling the key removes both; freshness via fetch can come later behind
  pinned-fingerprint verification.
- **Always sign exports.** Rejected: age cannot sign, and the use cases need confidentiality, not
  authorship proof. Signing is unnecessary complexity for the intended recipients (self, one's own
  developer, a known partner).
- **Always re-encrypt keys under a fresh passphrase for export.** Considered for the `secrets`
  slot — redundant for keys that already carry a passphrase. The rule above (as-is if passphrase,
  encrypt-required if not) is the minimal sufficient contract.
- **GPG-primary.** Rejected: age is gpm's default crypto, its recipient model is simpler, and SSH
  recipients enable the offline developer option. GPG stays secondary for GPG-native recipients.

## Effort

Medium. The age encryption path is largely already available (the age dependency ships with SSH
recipients enabled); the work is the recipient-selection UX, the bundled developer key and its
release rotation, wiring outer-envelope encryption into each export feature, and enforcing the
secrets-safety rules. (human: ~3–5 days / CC: ~2–3 sessions)

## Depends on / Supersedes

- Applies to every export: the diagnostics-log export (existing), repository export (R078), and
  whole-application export (R088).
- Owns the safety rules for R088's per-repository `secrets` slot.
- age's SSH-recipient support (already enabled in gpm's age dependency) underpins the
  "encrypt to developer" option.
