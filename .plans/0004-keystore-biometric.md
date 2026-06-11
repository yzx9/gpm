# Keystore + biometric unlock

**Priority:** P2
**Status:** TODO
**Phase:** Post-MVP (v1.1)

## What

Use Android Keystore to store the identity passphrase (from 0002) with hardware-backed encryption, and biometric authentication (fingerprint/face) to retrieve it. Users with a passphrase can unlock gpm with biometrics instead of typing their password every time.

If the user did not set a passphrase (0002 optional), biometric unlock is not applicable — the identity is stored as plaintext and no unlock is needed.

## Why

Plan 0002 adds optional passphrase encryption for the age identity. Requiring password entry on every app launch is poor UX. Biometric lets users skip password entry while maintaining real security — the passphrase is protected by hardware-backed biometric authentication.

## Context

### Security chain

```
User taps "Copy password"
  → Biometric prompt
  → Android Keystore unlocks (hardware-backed, biometric-gated)
  → Passphrase retrieved from Keystore
  → Passphrase decrypts age identity
  → Age identity decrypts .age file
  → Password copied to clipboard
```

### Components

1. **Android Keystore** — Hardware-backed encrypted storage for the passphrase. The AES key is bound to the device's secure element (TEE/StrongBox). Even with root access, the Keystore key cannot be extracted.

2. **Biometric prompt** — Required to use the Keystore key. Fingerprint, face, or PIN fallback.

3. **Custom Kotlin plugin** — `gpm-plugin-keystore` wrapping Android Keystore directly. Follows existing `SafeAreaPlugin.kt` pattern.

### Implementation options

1. **Custom Kotlin mobile plugin** (recommended) — Write a Tauri mobile plugin that wraps `KeyStore` + `Cipher` directly. Full control, no third-party dependency for a security-critical component. Biometric gating is a Keystore key configuration (`setUserAuthenticationRequired(true)`).

2. **`tauri-plugin-keystore` (impierce)** — Third-party plugin wrapping Android Keystore + iOS Keychain. Less control over security implementation.

3. **`tauri-plugin-biometry`** — Community plugin. Lower adoption, third-party risk for security-critical path.

### Implementation

1. **New plugin: `gpm-plugin-keystore`**
   - Kotlin: `KeystorePlugin.kt` — wraps Android Keystore
     - `store(alias, data: ByteArray, requireBiometric: Boolean)` — encrypt data with Keystore key
     - `retrieve(alias, reason: String)` — prompt biometric if required, decrypt and return data
     - `delete(alias)` — remove stored secret
     - `isAvailable()` — check if Keystore + biometric hardware exists
   - Rust: plugin registration + IPC bridge

2. **`src-tauri/src/lib.rs`** — Keystore + biometric commands:
   - Register keystore plugin
   - `enable_biometric_unlock()` — store passphrase in Keystore with biometric requirement
   - `biometric_unlock()` — retrieve passphrase via biometric → decrypt identity → cache
   - `disable_biometric_unlock()` — delete passphrase from Keystore

3. **Frontend** — Biometric UX:
   - Settings: toggle "Biometric unlock" (only shown if passphrase is set)
   - App launch: if biometric enabled, show biometric prompt instead of password input
   - Fallback: if biometric fails 3× or is unavailable, fall back to password entry
   - `UnlockPage.vue`: detect biometric availability, prefer biometric over password input

4. **Edge cases**:
   - User changes passphrase (0002): must re-encrypt in Keystore
   - User disables biometric: delete from Keystore
   - Biometric enrollment changes (new fingerprint): Android invalidates Keystore key, user must re-enable
   - Desktop: no Keystore → always show password input (no equivalent)

### Kotlin-side rule

Always use `ByteArray`, never `String`. JVM `String` is immutable and cannot be zeroed. `ByteArray` can be zeroed in-place with `.fill(0)`.

## Effort

~2-3 days (human) / ~1 hour (CC)

## Depends on

0002-age-encrypted-identity.md — passphrase must exist before Keystore can protect it.
