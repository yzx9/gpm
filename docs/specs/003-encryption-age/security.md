<!--
Feature-level threat model for age encryption. Complements docs/SECURITY.md. Living.
-->

# 003 — age encryption: threat model

## Assets & trust boundary

The age identity (x25519 private key, or an SSH private key) — whoever holds it
decrypts the store. The decrypted secret is handled per `001/security.md`. Trust
boundaries: the age-plugin subprocess (for plugin recipients) and process memory.

## Identity in memory

Decrypted content uses `Zeroizing<String>`, wiped after use (system-wide — see
`docs/SECURITY.md`). The identity cache lifetime follows AutoLock (`007/security.md`):
in Immediate mode the identity is decrypted per operation and wiped right after;
Idle/Never keep it cached for the session. A failed op also clears the cache.

## age-plugin subprocess trust boundary

Plugin recipients (e.g. `age1yubikey1...` from age-plugin-yubikey) are encrypted to by
spawning the user-installed `age-plugin-<name>` subprocess; only age file
keys/stanzas cross its stdio protocol — no secret reaches the WebView. This is the same
trust boundary the `age` CLI and gopass already assume: the user trusts the binary they
installed. A missing binary surfaces as `PluginUnavailable`, not a silent write failure.
Desktop only — Android can't run such a binary.

## Recognition vs decrypt (PQ / plugin identities)

Post-quantum (`age1pq1...`) and plugin (`AGE-PLUGIN-...`) keys are **recognized** and
surface a clear "not supported yet" status rather than a parse failure, so the user
knows the key type instead of seeing a generic error. Full PQ decrypt is blocked on
upstream rage; plugin-identity decrypt is future work. PQ stanza length is strictly
validated and PQ/non-PQ recipient mixing is forbidden.

## Error sanitization

Errors carry codes/generic descriptions only, never secrets (system-wide rule).

## Cross-references

- PQ route-A/B decision, IDENTITY|RECIPIENT convention: `prd.md` §5.
- Hardware-key identity abstraction (shared reshape with 004): `prd.md` §5.
