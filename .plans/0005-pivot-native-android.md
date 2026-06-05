# 0005: Pivot plan — native Android UI + Rust core if Tauri mobile fails

**Priority:** P3
**Status:** TODO (contingency)
**Phase:** Triggered if Phase 3 (Android target) fails

## What

If Tauri v2 mobile proves unworkable in Phase 3, pivot to native Android UI (Kotlin/Jetpack Compose) with the Rust core via JNI. The Rust core (Phase 1) drops in unchanged.

## Why

Outside voice (Codex) argues that Tauri creates the WebView boundary the design then spends the entire doc routing around. Native Android removes it entirely — secrets never touch a WebView. The security architecture becomes trivially simple.

## Context

This is a contingency, not a planned path. The user has chosen Tauri twice (D1: proceed despite maturity risk, D7: acknowledge setup page contradiction). Phase 3 will reveal whether Tauri mobile works for this use case.

If triggered:
- Phase 1 output (Rust core library) is reusable without changes
- Phase 2 output (desktop Tauri app) can remain as the desktop client
- New work: Kotlin/Jetpack Compose UI, JNI bindings to Rust core, Android Keystore integration
- Estimated effort: ~3-5 days for native Android app

## Effort

~3-5 days (human) / ~2 hours (CC) if triggered

## Depends on

Phase 1 (Rust core must be complete). Triggered only if Phase 3 fails.
