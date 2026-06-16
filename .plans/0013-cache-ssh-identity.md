# Cache the decrypted SSH identity (perf)

**Priority:** P2 (perf; no correctness bug)
**Scope:** `rustpass/` only — no app-layer or frontend change.
**Status:** Not started.

## Context

SSH identities are re-decrypted on **every entry read**; x25519 is not. The asymmetry lives in the
`Store::get` → `get_identity_bytes()` → `decrypt_bytes` path:

- **x25519 (passphrase-encrypted, unlocked):** `cached_identity` is populated (`store.rs:245-260`),
  so `get_identity_bytes()` (`store.rs:875-881`) returns the _decrypted_ `AGE-SECRET-KEY-...`
  plaintext. `decrypt_bytes` takes the cheap x25519 branch (`crypto.rs:69`). **Per-entry: cheap.**
- **SSH (passphrase-protected, unlocked):** `cached_identity` is **never** set for SSH (only the
  `AgeEncrypted` branch writes it). `get_identity_bytes()` returns the _raw_ SSH PEM from disk
  (`store.rs:884`), and `decrypt_bytes` re-parses + re-decrypts the SSH private key on every call
  (`crypto.rs:88-113`, the bcrypt KDF at `crypto.rs:108`). **Per-entry: full bcrypt KDF.**

x25519 pays scrypt once at unlock; SSH pays bcrypt on every copy/show. This is the acknowledged
"cache the decrypted SSH identity" TODO from `.plans/0002-keystore-biometric.md`.

## Design — reuse the bytes cache, not a trait-object cache

The naive approach is to cache `Vec<Box<dyn age::Identity>>`. **Rejected:** `age::Identity` has no
`Send`/`Sync` supertrait bound (`age/src/lib.rs:286` is bare `pub trait Identity {`), and `Store`
is `Arc`-shared across Tauri async tasks, so anything behind its `std::sync::RwLock` must be
`Send + Sync`. A trait object isn't — it would force swapping the cache to `tokio::sync::Mutex` and
threading async acquisition through `decrypt_bytes`. Not worth it.

**Chosen:** reuse the existing `cached_identity: RwLock<Option<Zeroizing<Vec<u8>>>>` field (decrypted
identity _bytes_). For SSH, `unlock()` will:

1. Parse the stored SSH PEM with `ssh_key::PrivateKey` (`ssh.rs` already round-trips SSH keys this
   way — `generate_keypair`, `export_private_key`).
2. If encrypted, `.decrypt(passphrase)` → an unencrypted `PrivateKey`.
3. `.to_openssh(LineEnding)` → an **unencrypted** OpenSSH PEM (`Zeroizing<String>`).
4. Store those bytes in `cached_identity`.

Then `get_identity_bytes()` returns the unencrypted PEM, and `decrypt_bytes`' SSH branch calls
`Identity::from_buffer(buf, None)` → `Identity::Unencrypted` → **no KDF** (`age/src/ssh.rs:552`
confirms unencrypted keys parse to the `Unencrypted` variant). The bytes cache is `Vec<u8>`
(`Send + Sync` trivially) — no lock-type change. This is the same mechanism x25519 already uses.

### Secret-surface note

Caching the decrypted SSH key increases its dwell time (today it's re-derived and dropped per
call). Capability is unchanged — the decrypted key and the passphrase are equivalent (both decrypt
every entry). The raw passphrase's dwell is removed in **0014**; net surface improves. State here
honestly if challenged.

## Implementation

### `rustpass/src/ssh.rs` — new helper

```rust
/// Decrypt an SSH private key (if encrypted) and return it as an UNENCRYPTED
/// OpenSSH PEM, for caching after unlock. `WrongPassphrase` on a bad passphrase.
pub fn to_unencrypted_pem(pem: &str, passphrase: &str) -> Result<Zeroizing<String>, Error>
```

`PrivateKey::from_openssh(pem)`; if encrypted `.decrypt(passphrase)` (resolve the exact
`is_encrypted`/`decrypt` API against ssh-key 0.6.7 — `.decrypt` exists, mirroring the existing
`.encrypt` at `ssh.rs:42`); then `.to_openssh(LineEnding::default())`. Reuses the serialization
pattern already in `export_private_key` (`ssh.rs:106-124`).

### `rustpass/src/store.rs` — `unlock()` decrypts SSH up front

- In `unlock()` (`store.rs:240-273`), add a `SshEd25519 | SshRsa` branch mirroring the existing
  `AgeEncrypted` `spawn_blocking` shape (the bcrypt decrypt is blocking work): decrypt the stored
  identity PEM via `ssh::to_unencrypted_pem(&raw, passphrase)`, store the bytes in
  `cached_identity`.
- **Keep** the existing `cached_passphrase` write (`store.rs:262-270`) for now — it becomes
  read-path-unused after this change but is removed cleanly in **0014**. Leaving it keeps `0013`
  green and `is_unlocked()`/tests undisturbed.

### `rustpass/src/crypto.rs` — no change

`decrypt_bytes` keeps its `(encrypted, identity_bytes, passphrase)` signature; the change is purely
what `identity_bytes` _contains_ for a cached SSH identity (unencrypted PEM) and that `passphrase`
is now ignored on the SSH read path.

## Phase-1 validation gate (do this first, ~10 min)

Throwaway/regression test in `ssh.rs`:

1. `generate_keypair(Some("pw"))` → encrypted PEM.
2. `to_unencrypted_pem(&encrypted, "pw")` → unencrypted PEM starting with
   `-----BEGIN OPENSSH PRIVATE KEY-----`, no `bcrypt`/encryption markers.
3. `age::ssh::Identity::from_buffer(unencrypted_pem.as_bytes(), None)` → parses to
   `Identity::Unencrypted` and unwraps a stanza the original key could.

If step 3 fails (age rejects the ssh-key-serialized unencrypted PEM), fall back to caching the
concrete `age::ssh::Identity` enum behind a `tokio::sync::Mutex` (the enum is `Clone + Send + Sync`,
so viable — just more invasive). The bytes path is strongly expected to work since both age and
`ssh.rs` use ssh-key's OpenSSH format.

## Tests

- **NEW — SSH decrypt happens once:** after `unlock()` on an encrypted SSH identity, `cached_identity`
  holds an unencrypted PEM (assert no encryption markers), and `get()` succeeds.
- **`unlock_marks_ssh_identity_unlocked`** (~1612) still passes. Optionally tighten to also assert
  `cached_identity` is populated for SSH.
- Keep existing conflict-probe and age-decrypt tests green. `just test`.

## Verification

- **`just test`** — rustpass unit + integration, incl. the new test.
- **`just lint`** — `clippy -D warnings` + `cargo fmt`.
- **Perf sanity (optional, manual):** with an SSH+passphrase identity, time several `get()` calls
  before/after. Expect per-entry bcrypt to collapse to a one-time unlock cost.

## Risks

- **Round-trip of ssh-key's unencrypted PEM through age's parser** — the one genuine unknown,
  mitigated by the Phase-1 gate. Documented fallback exists.
- **Decrypted-key dwell time increases** — net surface improves once 0014 lands; acceptable.

## NOT in scope

- **Dropping `cached_passphrase`** (secret lifetime) — that's **0014**, which depends on this.
- **`has_stored` liveness check** (F5) — **0015**.
