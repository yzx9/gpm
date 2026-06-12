# Multi-identity support + .age-recipients

**Priority:** P3
**Status:** TODO
**Phase:** Future

## What

Support multiple age and/or SSH identities for decryption. Parse `.age-recipients` files to determine which identity to use for each `.age` file, instead of trying all identities against every file.

## Why

MVP supports a single identity. In multi-user gopass setups, different `.age` files may be encrypted to different recipients (public keys). A user who has identities for multiple recipients needs all of them available. Without `.age-recipients` parsing, the app would need to try every identity against every file — slow and inelegant.

## Context

### Current behavior

Single identity tried against every `.age` file. If decryption fails, return error. No recipient awareness.

### gopass / age recipient model

- `.age-recipients` file in each directory lists public keys (age recipients or SSH public keys) that the files in that directory are encrypted to.
- To decrypt, the user needs at least one matching private key (identity).
- A gopass repo may have multiple `.age-recipients` files in different directories.

### Implementation

1. **Store multiple identities:** Replace single identity file with a collection. UI allows adding/removing identities. Each has a label (e.g., "Work key", "Personal key").

2. **Parse `.age-recipients`:** When listing entries, also parse `.age-recipients` files. For each entry, note which recipients it's encrypted to. This allows showing "needs: Work key" for entries the user can't yet decrypt.

3. **Targeted decryption:** When decrypting an entry, check its recipients → find matching identity → decrypt with that specific identity. Falls back to trying all identities if no `.age-recipients` exists (backward compatible).

4. **UI changes:**
   - SetupPage: allow pasting multiple identities
   - EntryListPage: show entries the user can't decrypt (grayed out, with "add identity" hint)
   - New: identity management section (add, remove, label identities)

### Key files

- `rustpass/src/config.rs` — Support multiple identities (Vec instead of single)
- `rustpass/src/crypto.rs` — Try multiple identities, return on first success
- `rustpass/src/store.rs` — Parse `.age-recipients`, map entries to required recipients
- `src/views/SetupPage.vue` — Multi-identity input UI
- `src/views/EntryListPage.vue` — Show decryptability status per entry
- `src/types.ts` — Update Entry type with recipient info

### Relationship to 0003 (encrypted SSH key)

Encrypted SSH keys (0003) are already supported as identities. Multi-identity must handle both x25519 and SSH key types with their optional passphrases.

## Effort

~1-2 days (human) / ~45 min (CC)

## Depends on

0007-reconfiguration-flow.md (reconfiguration should land first so the identity type system handles both formats from the start)
