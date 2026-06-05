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
- Android `BiometricPrompt` before each decrypt
- App-level PIN/password
- Tauri plugin wrapping Android biometric API

## Effort

~1-2 days (human) / ~30 min (CC)

## Depends on

Phase 3 (Android target working)
