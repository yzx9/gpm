# Long-term roadmap: age post-quantum (X-Wing / mlkem768x25519) decryption

**Priority:** P3
**Status:** Blocked
**Phase:** Future

## What

Full, native support for decrypting age files encrypted to post-quantum recipients (`mlkem768x25519`, the X-Wing hybrid = ML-KEM-768 + X25519). This is a forward-looking roadmap, not an immediate implementation.

## Why

- **"Harvest now, decrypt later" threat.** Once a cryptographically relevant quantum computer exists, ciphertexts encrypted today with X25519 and captured in storage become recoverable. Long-lived secrets — exactly what a password vault holds — are the primary target.
- age's answer is to add PQ at the **recipient layer** (X-Wing hybrid), not to enlarge the symmetric key: per NIST category 1, age's existing 128-bit file key paired with a PQ KEM is already adequate (analysis by Filippo Valsorda).
- gopass already offers PQ at the recipient layer. As a read-only gopass GUI client, gpm **must** eventually read PQ-encrypted vaults, or it gets locked out the moment a user enables PQ — and is gradually marginalized from the gopass ecosystem.
- This is an investment in **long-term confidentiality**, not an immediate feature need.

## Significance / benefits

- **Long-term viability.** Keeps gpm able to open PQ-enabled vaults, so it stays inside the gopass ecosystem.
- **Aligned security posture.** Tracks the age / gopass PQ direction, signaling that long-term confidentiality is taken seriously.
- **Ecosystem consistency.** x25519, SSH, and PQ recipients all become first-class in gpm.
- **Institutional knowledge.** Mapping the age PQ spec, the X-Wing KEM construction, and the upstream dependency landscape is itself a valuable internal asset.

## Difficulties

- **PQ code is security-critical.** Cryptographic code that "looks right" and "is right" are far apart; correctness must be backed by test vectors, not eyeballed review. gpm is a password manager — zero tolerance for error.
- **KEM-construction alignment.** age uses `MLKEM768-X25519` (X-Wing) via HPKE; the Rust-side KEM must be confirmed to be the **same construction** (combiner, key derivation) as age's reference (`filippo.io/hpke-pq` / the X-Wing draft). Draft-version drift is a classic trap.
- **Protocol strictness.** The stanza's `enc` must be exactly 1120 bytes and the body exactly 32 bytes — strictly validated (spec requirement, mitigates partitioning-oracle attacks). The identity → X-Wing decapsulation-key derivation must follow `filippo.io/hpke-pq`.
- **Large-key UX.** An X-Wing recipient string is ~1900+ characters vs ~62 for X25519. Display / copy / paste flows need rethinking (a problem the whole age ecosystem shares).
- **Hybrid security semantics.** The spec forbids mixing PQ and non-PQ recipients on the same file; policy must avoid downgrade-style confusion.

## Blockers

- **Primary: upstream Rust age (rage) has not implemented native `mlkem768x25519`.** Published `age 0.11.3` has no PQ; `main` only has the hardware `tagpq` variant (`age1tagpq1` / `mlkem768p256tag`, aimed at YubiKey/TPM). The native X-Wing that gopass uses is not implemented at all — open issues [#621](https://github.com/str4d/rage/issues/621) (tracking) and [#598](https://github.com/str4d/rage/issues/598) (community draft). The cleanest path — consume rage's native PQ type directly — is therefore **not available yet**.
- **Secondary: test infrastructure.** The C2SP CCTV post-quantum test vectors must be in place first as the hard correctness gate for any approach.

## Two routes

- **Route A (recommended: wait for upstream).** Track rage #621. Once rage ships native `mlkem768x25519`, bump gpm's age dependency and plug the identity into the existing decryption flow — the current architecture already dispatches over `dyn age::Identity`, so the integration surface is small. **Cost:** timing is not under our control.
- **Route B (if upstream stalls).** Self-implement a gpm-local `mlkem768x25519` decryption type that implements `age::Identity`, using the RustCrypto `x-wing` + `hpke` crates, handling the corresponding stanza ourselves. **Benefit:** no upstream dependency, immediately usable, no rage fork needed (age's `Identity` trait hands all stanzas to the identity to pick from). **Cost:** new security-critical code, must pass the CCTV vectors, KEM alignment must be verified. Not recommended unless there's a concrete user need or upstream is clearly stalled.

## Out of scope

- Tagged recipients (`age1tag1` / `age1tagpq1`): hardware-plugin-oriented (YubiKey/TPM), not relevant to a mobile read-only client.

## Verification (when eventually implemented)

- End-to-end with the `age` v1.3.x CLI (`age-keygen -pq`): generate a PQ key → encrypt → decrypt with gpm; confirm interoperability.
- Pass the C2SP CCTV `mlkem768x25519` test-vector suite.

## References

- age spec (PQ recipient): https://c2sp.org/age
- Filippo Valsorda, "KEMs and Post-quantum age": https://words.filippo.io/post-quantum-age/
- age v1.3.0 release (PQ): https://github.com/FiloSottile/age/releases/tag/v1.3.0
- rage tracking: https://github.com/str4d/rage/issues/621 , https://github.com/str4d/rage/issues/598
- RustCrypto `x-wing`: https://crates.io/crates/x-wing ; `hpke`: https://lib.rs/crates/hpke
- CCTV test vectors: https://github.com/C2SP/CCTV/tree/main/age
- Prerequisite (recognition): done — PQ keys are now recognized and surface a clear "not yet supported" error instead of a confusing failure (shipped)

## Effort

Route A (wait for upstream): ~S (human) / ~S (CC) — small, once rage ships native `mlkem768x25519`; the existing decryption dispatch already accommodates a new identity type, so the integration surface is small.

Route B (self-implement, if upstream stalls): ~L (human) / ~L (CC) — new security-critical code (X-Wing KEM + stanza handling) that must pass the CCTV test-vector suite and confirm KEM-construction alignment with age's reference.

## Depends on / Supersedes

External blocker: rage native `mlkem768x25519` (tracking str4d/rage#621). Builds on the already-shipped PQ-key recognition.
