---
pm: Zexin Yuan
created: 2026-07-15
version: 1.0.0
---

# 001 — Entry Access (finding & using a password)

> Status: Shipped · Last verified: 2026-07-28

## 1. Introduction

The read-access loop: browse / search → decrypt → copy or show a password. This is
the most frequent thing users do. The password stays out of the UI whenever it can —
copy is the default, and what's copied is cleared right after.

## 2. Motivation / Objective

Make "find a password and use it" as fast and as safe as possible: the list never
decrypts while you browse, copying is the primary action (the password never reaches
the UI), and showing is a secondary action that clears itself on a timer.

## 3. Use Cases

- **Jordan** keeps the master vault in desktop gopass and syncs that same repo to the
  phone over git. Out and about, they need a password on the phone — open gpm, search
  the entry name, copy, switch to the target app, paste. What matters most to them is
  **compatibility**: whatever the phone reads and writes back must round-trip cleanly
  with desktop gopass — the entry format and the 2FA convention have to match, and git
  sync must never leave one side unable to read what the other wrote. For entries with
  a second factor, they also want to grab the current code in the same motion.
- **Casey** has no desktop gopass — everything lives on the phone. They set up a local
  vault and entered their everyday accounts one by one; from then on the daily flow is
  fingerprint unlock → search the site → copy the password → log in. They won't clear
  the clipboard themselves, so the "auto-clear after copy, with a notification tapping
  them to confirm" behavior is what makes it feel safe.

## 4. Key Aspects

### Product Design

- Entries are password entries organized by path in the repo; the list shows names
  only, with the path as a footnote on the detail screen.
- Copy (primary, stays out of the UI) and show (secondary, reaches the UI but
  auto-clears) are two distinct safety models.

### Functionality

- Browse / search the list (no decryption); decrypt a single entry; copy (primary) /
  show (secondary, auto-clears after a while).
- The clipboard auto-clears after a copy and a clear-notification is posted; entries
  with a second factor can copy the current code; the password-show screen blocks
  screenshots / screen recording.

### Compatibility

- Entry format and 2FA both interoperate with gopass — what desktop gopass writes, the
  phone reads, and vice versa.

### Interactive

- Single-row list, pull-to-refresh; copy / show carry a countdown; controls are large
  enough for one-handed phone use.

### Adaptive

- Phone-first; desktop has no equivalent of the Android clipboard-clear notification
  (platform asymmetry).

### Security

See <./security.md>.

### Reliability

- Decrypt failures and missing keys surface a clear message; entries that can't be
  decrypted degrade gracefully (cross-ref 006); both manual and automatic clipboard
  clearing are dependable.

## 5. Open Questions & Key Decisions

- Whether the show-screen screen-capture block should be component-level (protect only
  the password field, not the whole screen).
- Whether the first 2FA release only copies the code, or also shows it with a
  countdown.

## 6. Roadmap

- **Shipped:** browse / search / decrypt, copy / show + auto-clear, clear-notification,
  show-screen screen-capture block, copying the 2FA code.
- **Future:** component-level screen-capture block.
