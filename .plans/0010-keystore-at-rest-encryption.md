# Encrypt local private data at rest (Android Keystore)

**Priority:** P3
**Status:** Proposal (not yet scheduled)
**Phase:** Future (hardening; mitigates the local-storage assumption documented in `docs/SECURITY.md`)

## Goal

Encrypt (and integrity-protect) gpm's local private files **at rest** using a
key sealed in the Android Keystore, so that an attacker who can _read_ the
app's private storage — via file theft, a backup/forensic dump, or another app
abusing storage access — cannot recover the secrets or silently tamper with
the trust configuration. This directly closes the gap the SECURITY.md
"Threat Model" section now states as an assumption: _no local attacker has
write access to the app's private storage_.

The target files are exactly those the SECURITY note calls out:

| File            | Today                                                                                                  | Real concern                                          | Right tool                                                 |
| --------------- | ------------------------------------------------------------------------------------------------------ | ----------------------------------------------------- | ---------------------------------------------------------- |
| `identity`      | Optional age-scrypt (passphrase)                                                                       | Confidentiality (only when no passphrase set)         | Encrypt                                                    |
| `repo.json`     | **Plaintext JSON** — holds PAT / SSH private key / SSH passphrase **and** the `authenticity` trust set | **Confidentiality (secrets) + integrity (trust set)** | AEAD encrypt (AES-GCM: confidentiality + integrity in one) |
| `repo/` (clone) | age blobs                                                                                              | Already encrypted by age                              | Out of scope                                               |
| Keystore blob   | Sealed by biometric plugin                                                                             | Already in Keystore                                   | Out of scope                                               |

The headline realization: **`repo.json` is the most sensitive unencrypted file
today** — it contains the git PAT and/or the SSH private key in cleartext, and
(now) the authenticity trust set. Because the trust set lives in the same file
as the secrets, a single AEAD over `repo.json` covers both jobs at once.

## Why

- **Closes the documented gap honestly.** Right now SECURITY.md can only
  _assume_ no local attacker; this feature turns that assumption into an
  enforced property (within the threat model below).
- **Hardware-backed on Android 11+.** A Keystore AES-GCM key (StrongBox / TEE)
  never leaves the secure element, so stolen files are ciphertext without the
  key — even a full copy of app-private storage is useless offline.
- **Reuses the Keystore plugin already built** for [0002](./0002-keystore-biometric.md).
  No new crypto stack; the `tauri-plugin-biometric-keystore` machinery
  (AES/GCM, hardware-backed) extends to an at-rest key.
- **Defends the authenticity feature too.** AEAD-authenticating `repo.json`
  means a tamper of the `authenticity` trust set (add attacker key, flip `mode`
  to `off`, add an `IgnoredIssue`) is _detected and rejected_ instead of
  silently neutering verification. This is the clean fix for the
  the authenticity feature's integrity blind spot we discussed.
- **Defense in depth.** Even if the OS sandbox is bypassed for _file read_
  (e.g., a malicious app with a storage-permission escalation), the secrets
  stay ciphertext.

## Cons

- **Android-only by nature.** The Keystore has no desktop equivalent, so this
  creates platform asymmetry: Android gets at-rest encryption, desktop stays
  plaintext (unless a separate desktop key source is adopted — see Blocks).
  This mirrors the existing biometric asymmetry but extends it to _all_ config.
- **Doesn't help against the strongest stated threat.** gpm already declares it
  won't defend against root or a fully compromised OS. A Keystore key only
  defeats _file theft_ and _non-root malicious apps_ — a process running **as
  the app** can still ask the Keystore to decrypt. So the marginal coverage is
  real but narrower than "encrypt everything" sounds.
- **Data-loss / recovery risk.** If the Keystore key is invalidated (app data
  cleared, Keystore wiped, factory reset, biometric-change policy on an
  auth-tied key) and there is no escrow, the encrypted files become
  **permanently unreadable** → forced re-setup. This is a serious UX cliff and
  must be designed around explicitly.
- **More crypto surface in a security-critical path.** Every config read/write
  now passes through an encrypt/MAC layer; a bug there (corrupt ciphertext,
  wrong nonce handling) can brick the app on boot. Must reuse vetted AES-GCM,
  not hand-roll.
- **Migration cost.** Existing users have plaintext files; a one-time
  encrypt-on-next-launch migration is required, and a half-finished migration
  is a corruption hazard.
- **Breaks naive backup/export.** Backed-up files are ciphertext; any future
  "export config" must decrypt first.

## Blocks

- **Pick the key's authentication policy.** Two shapes, with very different
  trade-offs:
  - **(A) Auth-free master key** (`setUserAuthenticationRequired = false`): the
    app can encrypt/decrypt on launch without a prompt. Simple, doesn't brick
    on biometric changes, but any app-context code can decrypt. Defeats _file
    theft_ only.
  - **(B) Passphrase / biometric-unwrapped key**: the at-rest key is derived
    from or sealed by the user's unlock. Far stronger (even app-context code
    needs the passphrase), but requires unlock before _any_ file access — and
    `repo.json` is needed at clone time during setup, so the setup flow must
    reorder. Risks bricking on key loss.
    This decision drives UX, threat coverage, and recovery — must be settled
    first. **(A) is the pragmatic fit for gpm's threat model; (B) is the
    maximalist option.**
- **Recovery / key-invalidation strategy.** Define what happens when the
  Keystore key is gone: forced re-setup? escrow? read-only fallback? Must be
  designed and documented before shipping.
- **Desktop story.** Explicitly decide: leave desktop plaintext and document
  the asymmetry, or adopt a cross-platform secret store (the `keyring` crate →
  macOS Keychain / Windows DPAPI / Linux Secret Service). The latter removes
  asymmetry but adds a dependency and per-OS quirks.
- **Per-file policy (post-merge: simpler).** `repo.json` → AEAD encrypt
  (AES-GCM covers the git credentials' confidentiality **and** the
  `authenticity` trust set's integrity in one shot); `identity` → encrypt
  _only if_ no passphrase (don't double-wrap an age-scrypt blob); cloned
  `repo/` → out (age handles it).
- **Migration path.** Encrypt existing plaintext files on first launch after
  upgrade, atomically, with a verified-readable gate before deleting
  plaintext.

## Challenges

- **Confidentiality vs. integrity are different jobs — but the merge unifies
  them for `repo.json`.** Since the (public) trust set now lives inside the
  (secret-bearing) `repo.json`, a single AES-GCM over that file delivers both
  at once. The remaining per-file nuance is only `identity` (skip if already
  age-scrypt encrypted).
- **Key lifecycle vs. biometric invalidation.** The existing biometric key is
  invalidated on new fingerprint enrollment. An at-rest master key should
  likely be auth-free (Option A) precisely to avoid bricking the store on a
  fingerprint change — but that trades away the "even app code can't decrypt"
  property.
- **`repo.json` is needed before unlock.** Clone happens during setup, before
  identity unlock. Option B (passphrase-unwrapped) forces a reordering; Option
  A sidesteps it. This is a concrete reason to prefer A unless B's extra
  strength is judged worth the setup-flow surgery.
- **Atomicity and fail-safety.** Write ciphertext → verify readable → only
  then discard plaintext. A crash mid-migration must leave the store
  recoverable, not half-encrypted.
- **Testability.** The Keystore isn't available in desktop CI unit/integration
  tests. The encrypt/MAC layer needs a **swappable cipher seam** (or a software
  Keystore mock) so the logic is testable without a device.
- **Nonce / header format.** Need a stable on-disk envelope
  (`{scheme, key_id, nonce, ciphertext, mac}`) so future scheme changes are
  forward-compatible, not a brittle ad-hoc blob.
- **Don't double-encrypt.** A passphrase-protected `identity` is already
  age-scrypt encrypted; wrapping it again wastes work and adds failure modes.
  The policy layer must detect "already encrypted" and skip.
- **`repo.json` AEAD-auth failure semantics.** On auth-tag mismatch: refuse to
  load and surface a loud error (do **not** silently fall back to `Off`, which
  is what an attacker wants). But also distinguish "tampered" from
  "legitimately migrated from an older plaintext version" — the migration must
  install the ciphertext + tag atomically.

## Sketch (approach, not commitment)

1. **Extend the Keystore plugin** (or add a sibling `tauri-plugin-at-rest`)
   with a non-auth AES-GCM key + an HMAC key, exposing `encrypt(blob)`,
   `decrypt(ct)`, `mac(blob)`, `verify(mac, blob)`.
2. **Per-file envelope** in `config.rs` / a new `rustpass::atrest` module: a
   versioned header `{scheme, nonce, ct, mac}` with policy dispatch
   (encrypt / mac / passthrough).
3. **Transparent wrappers** so `load_repo_config` / `save_repo_config` /
   `load_signing_config` / `save_signing_config` / identity load-save go
   through the envelope, with a `KeystoreAvailable` capability flag that
   falls back to plaintext on desktop (documented).
4. **One-time migration** on app start: for each plaintext file, encrypt/MAC
   in place, verify readable, commit.
5. **Failure mode**: if the Keystore key is absent at read time and a
   ciphertext file exists, surface "key unavailable — re-setup required"
   rather than silently downgrading.

## Relationship to existing work

- Builds on the Keystore plugin + discipline from
  [0002-keystore-biometric.md](./0002-keystore-biometric.md).
- Directly enforces the assumption just written into `docs/SECURITY.md`
  ("no local attacker has write access to private storage") — turning an
  assumption into a mitigation.
- Complements the repo-authenticity feature: AEAD
  authentication of `repo.json` closes the authenticity-bypass-via-tamper path
  that the plaintext `authenticity` field currently leaves open.
- The non-goal is unchanged: gpm does **not** defend against root or a fully
  compromised OS — this feature narrows the gap (file theft / non-root
  malicious app), it does not eliminate it.
