# 0001: Move FLAG_SECURE to Phase 3 prerequisite

**Priority:** P2
**Status:** TODO
**Phase:** Post-MVP (recommended: Phase 3 prerequisite)

## What

Add `FLAG_SECURE` to the Android build before `show_password` works on Android. Currently deferred to Phase 4 (Polish). Should be Phase 3.

## Why

Outside voice (Codex) identified that deferring `FLAG_SECURE` means the first Android build can leak passwords via screenshots and the recents screen. This is baseline containment, not polish.

## Context

The design doc puts `FLAG_SECURE` in Phase 4 (Polish & publish). The code is ~20 lines of Kotlin (`window.setFlags(FLAG_SECURE, FLAG_SECURE)`). Without it, any password shown via `show_password` is capturable by system screenshot and the recent apps overview.

## Effort

~2 hours (human) / ~10 min (CC)

## Depends on

Phase 3 (Android target setup)
