# 0007: User journey emotional arc and trust-building moments

**Priority:** P2
**Status:** TODO
**Phase:** Phase 2 (desktop Tauri app)

## What

Add trust-building elements to the user journey: setup reassurance, clone progress, and return-visit confidence indicators.

## Why

The product's core value is trust ("whoa, I can audit exactly what this app does"). But the UI never communicates trust during the setup flow, where the user is most vulnerable. No reassurance when pasting their identity. No progress when cloning. No freshness indicator when returning.

## Context

The outside voice (Codex) identified that the trust story collapses at setup — the user pastes their deepest secrets into a WebView with no reassurance. The eng review accepted this as a documented limitation (D7), but the emotional arc can still be improved with copy and UX patterns.

Key trust moments to address:
- Setup page: brief trust statement below identity field ("Stored locally. Nothing leaves your device.")
- Clone operation: progress indication (spinner + "Cloning repository..." → "Decrypting test entry..." → "Ready")
- Entry list: "Last synced 2h ago" header with pull-to-refresh
- Error states: calm, specific messages (not "Error occurred")

## Effort

~2h (human) / ~15min (CC)

## Depends on

Phase 2 (Tauri app UI)
