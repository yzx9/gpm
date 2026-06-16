# Drop the raw-passphrase cache (secret lifetime)

**Priority:** P2 (secret-surface hygiene; no correctness bug)
**Scope:** `rustpass/` only — no app-layer or frontend change.
**Depends on:** [0013 — cache the decrypted SSH identity](./0013-cache-ssh-identity.md).
**Status:** Not started.

## Context

`unlock()` writes `cached_passphrase` **unconditionally** for every identity type
(`store.rs:262-270`):

- **x25519:** the passphrase is unused after unlock — decryption uses only `cached_identity`. It
  nonetheless sits in memory for the whole session.
- **SSH:** the cached passphrase was the unlock state (re-decrypted per entry). Once **0013** caches
  the decrypted SSH key in `cached_identity`, the passphrase is no longer needed on the read path
  for SSH either.

So after 0013, **neither identity type needs `cached_passphrase` in steady state**. The raw
passphrase (the user's actual secret, reusable elsewhere) can then live only transiently during
`unlock()` / `validate_passphrase()`, not for the whole session. This matters most on the biometric
feature, whose whole job is to retrieve that passphrase. (codex F2 / D8 from
`.plans/0002-keystore-biometric.md`.)

## Why it depends on 0013

`is_unlocked()` (`store.rs:220-228`) recognizes SSH unlock via `cached_passphrase` today, and SSH
entry decryption re-derives the key from the cached passphrase. Dropping the field before 0013
caches the decrypted SSH key would break **both** SSH unlock recognition and SSH entry decryption.
0013 makes `cached_identity` populated for SSH, so removing `cached_passphrase` becomes safe.

## Implementation

All in `rustpass/src/store.rs`:

- **Remove the `cached_passphrase` field** (`store.rs:129`), its init (`store.rs:151`), the
  unconditional write in `unlock()` (`store.rs:262-270`), the `lock()` clear (`store.rs:311-313`),
  and `get_cached_passphrase()` (`store.rs:899-904`).
- **Update the three call sites** that fed the cached passphrase into decrypt — `get`
  (`store.rs:524`) and the two conflict-detection probes (`store.rs:816, 839`) — to pass `None`:
  `decrypt_…(…, &identity_bytes, None)`. (The cached `identity_bytes` already contain the decrypted
  key for both age and SSH after 0013.)
- **`is_unlocked()` (`store.rs:220-228`):** simplify to check `cached_identity` only. After 0013,
  both age and SSH populate it; plaintext never calls `unlock()` (and has `encrypted=false`, so the
  lock gate never triggers). Confirm no type relies on `cached_passphrase` for its unlocked signal
  (none does).
- **`validate_passphrase()` (`store.rs:286-302`):** unchanged — it takes the passphrase as an
  argument and is a no-cache one-shot. Confirm it does not touch the field.

## Tests

- **Update `unlock_caches_passphrase_for_plaintext_identity`** (`store.rs` ~1584): its premise
  (`unlock()` on a plaintext identity + a `cached_passphrase` assertion) no longer holds. Delete or
  repurpose — assert `cached_identity`/`is_unlocked()` behave sanely for plaintext instead.
- **NEW — no steady-state passphrase:** after unlocking an age-encrypted identity, the field is gone
  (structurally enforced by removal + `clippy` surfacing any stray reader).
- Keep `unlock_marks_ssh_identity_unlocked` (~1612) green (now via `cached_identity`).
- `just test`.

## Verification

- **`just test`** + **`just lint`** (`clippy -D warnings` will surface any lingering
  `cached_passphrase` reference; `cargo fmt`).
- No frontend/app-layer change → no `pnpm test` or on-device step.

## Risks

- **Test churn** — the plaintext test encodes current behavior and must change. Handle deliberately
  (the 0002 prerequisite-fix note already flagged this test as behavior-encoding).
- **Silent reader left behind** — `clippy`/compile is the safety net; that's why `just lint` is
  gating.

## NOT in scope

- The SSH cache itself — **0013** (required first).
- `has_stored` liveness — **0015**.
