<!--
Feature-level threat model for git storage & sync: authenticity, working-tree integrity,
identity cache during sync. Complements docs/SECURITY.md. Living.
-->

# 005 — Git storage & sync: threat model

## Repository authenticity

`age` guarantees **confidentiality** but not **authenticity** of the store history. A
successful `git pull` only proves you received a valid git object graph — not that it
came from someone you trust. An attacker who controls the remote can feed age blobs that
decrypt fine but contain data they also know (e.g. a new `aws/root.age` with a password
they chose).

gpm offers optional **commit signature verification** for both formats git supports —
**SSH-signed** commits (git ≥ 2.34 `gpg.format = ssh`, verified with the `ssh-key`
crate) and **GPG/OpenPGP-signed** commits (verified with the pure-Rust `rpgp` crate —
no C, `ring`, or `openssl`). Both are checked against a user-managed trusted-signing-key
set. Tri-state per-repo:

- **Off** — no verification (default).
- **Audit** — verify every pulled commit; warn on mismatch, always pull.
- **Enforce** — verify every pulled commit; a non-ignored blocking issue aborts the
  pull, leaving HEAD and the working tree on the last verified state.

On each pull every commit in `(old HEAD, new HEAD]` is verified (not just the tip — a
buried malicious commit behind a signed tip is still caught). The trusted-signing-key
set is public, non-secret; it lives as the `authenticity` field of `repo.json`.

**Trust is set membership, not web-of-trust.** A simple "is this key in the trusted
set?" check — ignores GPG owner-trust, certification levels, keyserver lookups, so no
new network trust vector. Add a trusted signer by pasting its public key (or importing
`.asc`). For GPG the trusted identity is the primary-key fingerprint; a subkey signature
verifies against the trusted primary via its binding signature.

**Expiry and revocation are NOT enforced.** An expired or revoked GPG key still verifies
here (revocation isn't even parsed for policy). Treat the trust set as "keys I have
chosen to trust", not "keys currently valid by GPG's rules."

**SSH-sig vs GPG make different guarantees about an untrusted signer.** An SSH signature
embeds the signer's public key, so gpm always verifies the cryptography and only the
trust decision remains — an untrusted SSH signer surfaces as `UntrustedKey`
(crypto-verified, just not in your set). A GPG signature carries only the issuer
fingerprint, never the key, so when the signer is untrusted gpm has no key to check and
performs **no cryptographic verification** — surfaces as a distinct `UnverifiedSignature`
status, a weaker statement. The difference is visible to the user, not hidden.

**Defeats** (Enforce; detects in Audit): a compromised remote feeding unsigned or
attacker-signed commits, or tampering with a signed commit's contents (any edit
invalidates the signature → `BadSignature`).

**Does not defeat**: the signing key itself being compromised (rotation/revocation is
the countermeasure — and gpm does not yet honor revocation); a malicious commit made
before the feature was enabled (verification is forward-looking — use History to audit
the past); transport-level spoofing (HTTPS/SSH transport trust).

**Irreducible first-use assumption:** trusting the current HEAD's signer at enable time
assumes HEAD isn't already an attacker commit. The explicit confirm step is the
mitigation; History is the escape hatch.

## Working-tree tampering (accepted limitation)

Authenticity verifies commit signatures on `pull` (remote→local), **not** local
working-tree tampering. gpm assumes no local write attacker (system-wide — see
`docs/SECURITY.md`); a write attacker can tamper with the cloned `repo/`, `.git`
objects, or the recipients file between operations. Defending that would require a
sealed snapshot over the working tree, not implemented. Authenticated at-rest encryption
prevents _forging_ a file but not a rollback to an older plaintext.

## Identity cache during divergence resolve

On a `NeedsDivergenceResolve` outcome, the Immediate-mode identity-cache wipe is
**deferred** so a keep-mine resolve can reuse the cached identity without a second
unlock. The deferred wipe runs both in the resolve step and on resolve-cancel, so
abandoning the modal never strands the key. (Cache lifecycle details in `007/security.md`.)

## Secret revision history

Viewing a past version of a secret (the shipped secret-revisions feature, formerly R027) decrypts a blob from history with the current identity and reveals it under the same short-lived reveal / auto-clear / wipe-on-drop contract as the current password — no new auth surface. A past revision the current identity can't decrypt (recipient rotation, a teammate's revision in a shared store) is reported as an "undecryptable" state and **never** surfaced as ciphertext, mirroring how an unreadable remote secret is treated — so the threat model holds unchanged: ciphertext never crosses into the untrusted layer, only decrypted plaintext, and only on the decryptable outcome. The listing itself is pure metadata (commit hash, author, date, message, signature status) and is key-free.

## Cross-references

- Recipients file-level trust (a separate, complementary defense): `006/security.md`.
- System-wide "no local write attacker" assumption: `docs/SECURITY.md`.
