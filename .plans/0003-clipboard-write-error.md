# 0003: Handle clipboard write failure in copy_password

**Priority:** P1
**Status:** TODO
**Phase:** Phase 2 (desktop Tauri app)

## What

Return an error when clipboard write fails instead of returning `CopyResult { success: true }` silently.

## Why

Failure mode analysis identified a critical gap: if the Android clipboard service is unavailable, `copy_password` returns success but the password was never copied. The user thinks they copied their password but didn't. No test, no error handling, silent failure.

## Context

`copy_password` is the primary operation (90%+ of usage). It must be reliable. The current design calls `clipboard::write()` without checking the result, then returns `CopyResult { success: true }`.

Fix: check the clipboard write result before returning success. Add `CLIPBOARD_ERROR` to the error codes table.

```
| `CLIPBOARD_ERROR` | Clipboard write failed (service unavailable, permission denied) | Yes |
```

## Effort

~30 min (human) / ~5 min (CC)

## Depends on

Phase 2 (clipboard integration via `tauri-plugin-clipboard-manager`)
