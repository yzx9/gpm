---
pm: Zexin Yuan
created: 2026-07-24
revision: 1
scope: autofill
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
  site you're logging into, matches it to an entry → unlock (following the app-lock mode,
  like copy/show) → drop the account and password straight into the target field (Future).

### Compatibility

- Match entries by app identity / website. For web logins the entry path
  (`websites/<domain>/...`) is a plaintext, browseable key, so a match can be offered
  before any unlock; a correspondence the user picks once is remembered and auto-filled
  next time. Native apps expose a package id rather than a domain, so they rely on that
  learned mapping and need one manual pick before they auto-fill. Matching follows
  gopass's URL convention.

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
  and selection. At first the service fills only fields that declare username/password
  autofill hints; fields without hints are not filled by the framework path until a
  later phase.

## 5. Open Questions & Key Decisions

- **Decided — entry association.** Match on three signals: the entry path
  (`websites/<domain>/...`, plaintext, usable before unlock) for web logins; a learned,
  encrypted `app/site → entry` mapping built from the user's first pick, primary for
  native apps and fallback elsewhere; and the `url:` field the website template writes
  into the body as a secondary accelerator.
- **Decided — hint-poor apps.** The MVP fills only fields that declare username/password
  autofill hints; the Accessibility-based variant is held in reserve as the fallback for
  apps the framework cannot identify.
- Credential Manager / passkey as a follow-on layer above autofill (still open).

## 6. Roadmap

- **Future:** autofill service + entry association (recognize the target app / site,
  fill the login field).
