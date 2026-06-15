# GPG / OpenPGP commit signature verification

**Priority:** P4
**Status:** Proposal (not yet scheduled)
**Phase:** Future (extension to the SSH-only authenticity feature — see [repo-authenticity](../docs/SECURITY.md#repository-authenticity))

## Goal

Add **GPG / OpenPGP** commit signature verification as a parallel verifier
alongside the existing SSH-sig verifier, so that gopass repositories whose
commits are GPG-signed (still common in traditional setups and on hosts where
`gpg.signingkey` is the norm) get the same authenticity guarantee gpm already
provides for SSH-signed commits.

Today a GPG-signed commit is recognized but **not** verified — it surfaces as
`CommitSigStatus::UnsupportedFormat { format: "gpg" }`, a soft warning. In
**Audit** that means a noisy "signed, but not with an SSH key gpm can check"
nag on every GPG-signed commit; in **Enforce** it blocks the pull unless the
user ignores each one. This plan would turn those into real `Verified` /
`UntrustedKey` / `BadSignature` verdicts.

The non-goal is unchanged from the authenticity feature: gpm only
**verifies** — it never signs. Signing remains a desktop-side concern.

## Why (优点)

- **Closes the ecosystem gap.** git supports two signing formats; gpm currently
  honours only one. Repositories that mandate GPG signing (organizational
  policy, CI bots with GPG keys, long-established gopass stores) cannot use
  Enforce meaningfully today — every GPG commit is a blocking issue.
- **Reuses the existing scaffolding.** The whole authenticity stack —
  `CommitSigStatus` enum, `signing.json` trust set + ignore list, the Off /
  Audit / Enforce modes, verify-before-checkout pull, the `/history` screen,
  the indicator badge, the modals — is format-agnostic. A GPG verifier plugs
  into one branch of `status_of_commit` (`classify_signature` already
  distinguishes GPG armor) and emits the same status variants. No new UI
  paradigm, no new persistence model.
- **Defence in depth for mixed histories.** Some repos have a mix of
  SSH-signed recent commits and GPG-signed older ones (a migration in
  progress). Verifying both lets Enforce cover the full history instead of
  forcing the user to ignore the entire GPG-era tail.
- **Consistency with the product's pitch.** "Detect a compromised remote" is a
  weaker claim when it silently skips an entire signing format the user
  actually relies on.

## Cons (缺点)

- **A real, heavy crypto dependency.** Unlike SSH-sig verification (which
  reused the already-present `ssh-key` crate at zero cost), GPG needs an
  OpenPGP implementation — `sequoia-openpgp` (large, its own net dependency
  tree) or the `pgp` crate. This measurably increases binary/APK size and
  build time, and widens the trusted-crypto surface for a feature many users
  will never turn on.
- **A key-distribution story gpm otherwise doesn't have.** SSH public keys are
  self-contained one-liners (`ssh-ed25519 AAAA…`) that paste cleanly into a
  trust set. GPG public keys are keyrings with subkeys, user IDs, expiry,
  revocation signatures, and a web-of-trust / keyserver model. gpm has no GPG
  keychain today and would have to invent a minimal one.
- **UX complexity leaks into the trust UI.** The current "add a trusted signing
  key" paste box is tuned for a single SSH pubkey. GPG keys invite questions
  the SSH path never raises: which subkey signed it, is the key expired /
  revoked, do we trust by primary fingerprint or per-subkey, do we honor
  keyserver lookups. Each is a small decision that compounds.
- **Marginal benefit for a likely-small audience.** New gopass setups and the
  GitHub-signing ecosystem have converged on SSH signatures; GPG is the
  legacy/enterprise path. The effort may serve a minority of users.

## Blocks (前置依赖)

- **Pick the OpenPGP crate.** `sequoia-openpgp` (more complete, actively
  maintained, larger) vs `pgp` (lighter, pure-Rust, less mature). This is a
  workspace-`Cargo.toml` decision and the single biggest cost lever — must be
  settled before anything else.
- **Decide the trust model for GPG keys.** How does a user record a trusted
  GPG signer? Options: paste an ASCII-armored public key block; import from a
  keyserver by fingerprint; reuse `pass`-style `.gpg-id`. The choice shapes
  both the data model (a `TrustedKey` today stores an SSH pubkey string — GPG
  keys don't fit that shape cleanly) and the Settings UI.
- **Generalize `TrustedKey` / the trust UI.** The current `TrustedKey` is
  SSH-shaped (an OpenSSH pubkey string + its `SHA256:` fingerprint). GPG keys
  need their own identity (long key ID / fingerprint, optional subkeys, expiry
  handling) and their own "add trusted key" flow. The Settings card and the
  `/history` "Trust this signer" TOFU action need to branch on key format.
- **Confirm the git GPG verify contract.** git signs commits with the `git`
  tag for GPG (detached signature over the commit object, same `gpgsig`
  header as SSH). The verifier must reproduce libgit2's notion of the signed
  payload exactly, or it will false-`BadSignature` on every commit. (The SSH
  path already proved this works against `git2::extract_signature`'s
  `signed_data`; GPG should reuse the same bytes.)

## Challenges (可能的挑战)

- **Key / algorithm diversity.** GPG keys may be RSA, DSA, EdDSA, or ECC
  (NIST/Brainpool/Curve25519). The chosen crate must verify all of them —
  dropping support for an algorithm a real signer uses would silently regress
  that repo to `UnsupportedFormat`.
- **Subkeys, rotation, expiry, revocation.** A commit is often signed by a
  _subkey_, while the user trusts the _primary_ key. The verifier must follow
  the binding signatures, honor expiry at signing time, and treat a revoked
  key as a warning (revoked ≠ automatically untrusted, but it must surface).
  SSH has none of this machinery.
- **"Trust" means something subtler in GPG.** SSH TOFU is binary (the
  fingerprint matches or it doesn't). GPG carries owner-trust and
  certification levels; gpm would likely ignore all of that and do its own
  simple "is this key in the trusted set?" check — but should say so
  explicitly in `SECURITY.md` so users don't assume web-of-trust semantics.
- **Mixed-signing repos under Enforce.** A history that transitions
  GPG → SSH (or vice versa) must verify cleanly across the boundary, or
  Enforce becomes unusable for that repo. Needs a deliberate policy: does a
  trusted GPG key + a trusted SSH key both satisfy Enforce? (Intuitively yes,
  but the data model must support multiple trusted keys of mixed formats
  without ambiguity.)
- **Performance.** OpenPGP verification (especially RSA) is heavier than
  ed25519. The authenticity feature's design justifies
  per-commit range verification by noting ed25519 is "microseconds"; that
  assumption weakens for large RSA GPG keys across a long range. May need a
  note about typical gopass-store sizes.
- **Key import UX.** GPG public-key blocks are multi-line armored blobs with
  metadata; pasting one on a phone keyboard is painful. A keyserver lookup by
  fingerprint is friendlier but pulls in network + a trusted keyserver
  assumption — a new trust vector for a feature whose whole point is trust.
- **Feature-flag / build-cost isolation.** Because the dependency is heavy and
  the audience small, there's a strong argument to ship GPG verification
  behind a Cargo feature flag (off by default) so the default build doesn't
  pay the size/cost. That interacts with how `status_of_commit` is compiled
  and with the frontend's mode selector (can you pick Enforce if the build
  lacks GPG support and the repo is GPG-signed?).

## Relationship to existing work

Builds directly on the repo-authenticity feature. The `CommitSigStatus`
enum already reserves `UnsupportedFormat` for exactly this case, so adding GPG
verification does **not** change any public type, any persisted file
(`signing.json`), or any IPC shape — it only changes what one match arm of
`status_of_commit` returns. That makes this a contained extension rather than
a redesign, which is the main reason it remains a plausible future plan
despite the cons.
