# DRY: Extract identity format validation

Priority: P2 (code quality)
Discovered by: /plan-eng-review on 2026-06-12
Status: Open

## Problem

Identity format prefix check is copy-pasted 4 times across the codebase:

1. `rustpass/src/store.rs:198-208` — `Store::save_identity`
2. `rustpass/src/store.rs:244-253` — `Store::configure`
3. `rustpass/src/crypto.rs:90-94` — `decrypt_bytes`
4. `rustpass/src/recipient.rs:205-209` — `identity_to_recipient`

Each copy checks the same three prefixes:

- `AGE-SECRET-KEY-`
- `-----BEGIN OPENSSH PRIVATE KEY-----`
- `-----BEGIN RSA PRIVATE KEY-----`

And returns the same error pattern:

```rust
Err(Error::new(ErrorCode::InvalidIdentity,
    "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key"))
```

If a new key type is added (e.g., PKCS#8 `-----BEGIN PRIVATE KEY-----`), all 4 sites must be updated — easy to miss one.

## Fix

Extract a validation function to `identity.rs`, which already contains `classify_identity`:

```rust
// rustpass/src/identity.rs

/// Validate that `identity_bytes` starts with a recognized identity format prefix.
///
/// Returns `Ok(())` if valid, or an error describing the expected formats.
pub fn validate_identity_format(identity_bytes: &[u8]) -> Result<(), Error> {
    let Ok(text) = std::str::from_utf8(identity_bytes) else {
        return Err(Error::new(ErrorCode::InvalidIdentity,
            "Identity is not valid UTF-8"));
    };
    let trimmed = text.trim();
    if trimmed.starts_with("AGE-SECRET-KEY-")
        || trimmed.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        || trimmed.starts_with("-----BEGIN RSA PRIVATE KEY-----")
    {
        Ok(())
    } else {
        Err(Error::new(ErrorCode::InvalidIdentity,
            "Identity must be an age secret key (AGE-SECRET-KEY-...) or SSH private key"))
    }
}
```

Then replace all 4 call sites with `identity::validate_identity_format(identity_bytes)?`.

## Files to change

- `rustpass/src/identity.rs` — add `validate_identity_format`
- `rustpass/src/store.rs` — replace inline checks in `save_identity` and `configure`
- `rustpass/src/crypto.rs` — replace inline check in `decrypt_bytes`
- `rustpass/src/recipient.rs` — replace inline check in `identity_to_recipient`
- `rustpass/src/lib.rs` — re-export if needed

## Timing

Best done during or after the async migration (0008), which already touches 3 of 4 sites. Minimal extra diff.
