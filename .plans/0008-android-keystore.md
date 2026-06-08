# 0008: Android Keystore for identity storage

**Priority:** P3
**Status:** TODO
**Phase:** Post-MVP (v1.1)

## What

Replace plaintext identity file storage with Android Keystore-backed encryption. The age identity (and optionally SSH key / PAT) is encrypted at rest using a hardware-backed key.

## Why

Currently the age identity is stored as raw bytes in `{config_dir}/identity` — an app-private file but unencrypted. On a rooted device or through backup extraction, the identity is exposed. Android Keystore provides hardware-backed key storage where the encryption key never leaves the secure element.

## Context

### Current storage

```rust
// rustpass/src/config.rs
pub fn save_identity(&self, identity: &[u8]) -> Result<(), Error> {
    std::fs::write(self.identity_path(), identity)?; // plaintext
}
```

### Options

1. **`tauri-plugin-keystore` (impierce)** — Third-party Tauri plugin wrapping Android Keystore and iOS Keychain. Provides `store()`, `retrieve()`, `remove()` API. Requires Android 9+ (API 28+), which matches our minSdk. Recommends pairing with `tauri-plugin-biometric` for user-presence requirement.

2. **Custom Kotlin mobile plugin** — Write a Tauri mobile plugin in Kotlin that wraps `KeyStore` + `Cipher` directly. More control, no third-party dependency, but more development effort.

3. **`tauri-plugin-stronghold`** (official) — Iota Stronghold encrypted vault. Cross-platform but requires password initialization. Heavier than needed for a single key.

### Recommended approach

Use `tauri-plugin-keystore` (impierce). It handles the Android Keystore complexity (key generation, AES encryption, biometric binding) with a simple Rust API. The identity storage trait in `config.rs` would call the plugin instead of `fs::write`.

### Migration path

- New installs: identity goes directly to Keystore
- Existing installs: detect plaintext identity file → migrate to Keystore → delete plaintext file
- Desktop: Keystore plugin falls back to OS keychain (macOS Keychain, Linux secret-service). If unavailable, keep plaintext with a warning.

### Key files

- `rustpass/src/config.rs` — Replace `fs::write`/`fs::read` with keystore plugin calls
- `src-tauri/src/lib.rs` — Register keystore plugin
- `src-tauri/Cargo.toml` — Add `tauri-plugin-keystore` dependency

### Kotlin-side rule (from design doc)

When the identity crosses the Kotlin→Rust boundary, always use `ByteArray`, never `String`. JVM `String` is immutable and cannot be zeroed. `ByteArray` is mutable and can be zeroed with `.fill(0)`.

## Effort

~1 day (human) / ~30 min (CC)

## Depends on

None — but pairs well with 0002-app-lock-biometric.md (Keystore can require biometric unlock).
