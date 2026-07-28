---
pm: Zexin Yuan
created: 2026-07-24
version: 1.0.0
---

# 008 — Android Autofill (cross-app fill)

> Status: Planned · Last verified: 2026-07-28
> Future — planned, not yet committed to start.

## 1. Introduction

An Android autofill service: when a login field is focused in another app, the system
calls gpm and has it drop the account and password straight into that field, never
touching the clipboard. It sits alongside 001's copy / show as a third access path —
but cross-app and filling the target field directly.

## 2. Motivation / Objective

Copy-paste is the old friction on mobile: leave the target app, open gpm, copy,
switch back, paste, with the password taking a trip through the clipboard. Autofill
drops the account and password straight into the target field and never puts them on the
clipboard — easier, and safer than copying.

## 3. Use Cases

- **Jordan**, when logging into various apps on the phone, wants gpm to drop the account
  and password straight into the login field instead of the switch-copy-paste dance;
  for them it's the natural extension of 001's access path, and skipping the clipboard
  suits them.
- **Casey** autofill is the lowest-effort path for them — none of the copy-paste
  routine. But they first have to set gpm as the autofill service in system settings
  (one-time, and they may need a little guidance).

## 4. Key Aspects

### Product Design

- A system-registered service any app can invoke; it reuses 001's existing unlock +
  search + select flow rather than spinning up a parallel UI. The account and password
  go straight into the target field and never touch the clipboard.

### Functionality

- Focus a login field in another app → the system calls gpm → gpm recognizes which app /
  site you're logging into, matches it to an entry → biometric unlock → drop the account
  and password straight into the target field (Future).

### Compatibility

- Match entries by app identity / website; a correspondence the user picks once is
  remembered and auto-filled next time; follows gopass's URL convention.

### Interactive

- The system surfaces gpm's candidates → reuse the existing unlock and search UI → pick
  an entry → fill; no new search screen is built.

### Adaptive

- Android only (the Autofill Framework); desktop has no equivalent.

### Security

See <./security.md>.

### Reliability

- The service sees only the focused-screen region the system hands it; it does not scan
  the screen on its own. An app / site it can't recognize falls back to manual search
  and selection.

## 5. Open Questions & Key Decisions

- How to associate an entry with a target app / website (combining learned mappings with
  the URL convention in the entry body).
- Whether to offer an optional assisted-recognition fallback for apps the framework
  can't identify.
- Credential Manager / passkey as a follow-on layer above autofill.

## 6. Roadmap

- **Future:** autofill service + entry association (recognize the target app / site,
  fill the login field).
