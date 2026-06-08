# 0002: Add app lock / biometric re-auth

**Priority:** P2
**Status:** TODO
**Phase:** Post-MVP (v1.1)

## What

Require biometric or PIN before decrypt operations. No app lock exists in MVP — if the device is unlocked, anyone can open the app and decrypt everything.

## Why

Outside voice (Codex) identified that the design relies entirely on the OS sandbox for protection. For a password manager, no re-auth before reveal/copy is a significant gap. A device left unlocked and unattended exposes all passwords.

## Context

The MVP design has no app-level authentication. The assumption is that the device lock screen is sufficient. This is acceptable for personal use on a device you control, but should be addressed for broader distribution.

Implementation options:

1. **`tauri-plugin-biometric`** (recommended) — Official Tauri plugin wrapping Android `BiometricPrompt` and iOS `LAContext`. Prompts fingerprint/face/PIN before decrypt.
2. **`tauri-plugin-keystore`** (impierce) — Device-native key storage that can require biometric unlock to retrieve the identity. Combine with 0008-android-keystore.md.
3. **App-level PIN/password** — Simple but less secure than hardware-backed biometric.

Recommended approach: Use `tauri-plugin-biometric` to gate `copy_password` and `show_password` commands. The biometric prompt appears before each decrypt operation. On failure or cancel, the operation is rejected.

### Key files

- `src-tauri/src/lib.rs` — Register biometric plugin, add auth check before decrypt commands
- `src-tauri/Cargo.toml` — Add `tauri-plugin-biometric` dependency
- `rustpass/src/store.rs` — `get()` method is the single decrypt entry point, add auth gate here or at Tauri command level

### UX considerations

- First launch after setup: prompt user to enable biometric lock (optional, not forced)
- Subsequent launches: biometric prompt before decrypt
- Desktop fallback: no biometric available, either skip or use a master password
- Failed attempts: rate-limit after N failures (Android BiometricPrompt handles this)

## Effort

~1-2 days (human) / ~30 min (CC)

## Depends on

Phase 3 (Android target working)
