---
pm: Zexin Yuan
created: 2026-07-15
revision: 1
scope: secrets
---

# 002 — Secret Management (create, edit, generate)

> Status: Partial · Last verified: 2026-07-28
> Basics shipped: create / edit / delete / generator.

## 1. Introduction

The write loop: add / edit / delete entries, generate a password (saving it or copying
it once), and the conflict **experience** when a save runs into a remote change.

## 2. Motivation / Objective

Let users create, edit, and delete on the phone — and have what they write round-trip
cleanly with desktop gopass (the CLI can read it and change it back). When a strong
password is needed, generate one in the moment without saving it. And when a save
collides with a remote change, give a clear, recoverable choice — never a silent
overwrite.

## 3. Use Cases

- **Jordan** does create / edit / delete on the phone too, not just reads — but they
  hold one iron rule: anything created or edited on the phone must round-trip
  seamlessly with the desktop gopass CLI. What the phone writes, the desktop reads and
  edits; and vice versa. So gopass compatibility is a hard constraint, not a
  nice-to-have. When they share the repo (with a partner or a small team), saves are
  more likely to collide with remote changes — and then what they want is a clear
  "keep mine / keep theirs / cancel" choice, not a coin-flip over who overwrites whom.
- **Casey** starts from scratch — following the create wizard, picks a template (login,
  bank card, that sort of thing), enters their accounts one by one, and when a password
  is needed has the app generate one they could never have come up with themselves.
  Sometimes they're just registering on a site and want to generate a throwaway
  password, paste it, and not save it. They neither understand nor care about complex
  field modeling.

## 4. Key Aspects

### Product Design

- Create is a stepped wizard; the field model aligns with gopass (preset templates +
  custom); "generate without saving" reuses the same generator.

### Functionality

- Create (template / custom), edit, delete, password generator (save or one-time copy).
- Save-conflict experience: a conflict dialog lets the user choose which version to
  keep. (The conflict _mechanism_ — auto-sync rejecting a push, version comparison, a
  cancellable push — belongs to 005; see cross-ref.)

### Compatibility

- What gpm writes, gopass can read and edit; what gopass writes, gpm can read — safe
  two-way round-trip.

### Interactive

- Stepped create wizard; edit form; leaving the page clears any unsaved sensitive
  value.

### Adaptive

- Phone-first stepped flow; the edit / generate screens clear sensitive values on leave
  (same as 001).

### Security

- The edit / generate / create screens clear sensitive values on leave or lock;
  error messages are sanitized.

### Reliability

- Conflicts are recoverable; no data is lost.
- Under **Auto-sync on**, a stale read no longer silently overwrites a newer
  remote change: editing or deleting a secret another device changed since you
  opened it surfaces a per-entry choice (keep your version or theirs), and a
  delete a teammate already did is recognized rather than claiming a commit.
  Under **Auto-sync off** this still defers to a manual-Sync divergence. (A push
  in progress can't be cancelled — the mechanism lives in 005.)

## 5. Open Questions & Key Decisions

- Conflict split is settled: experience in this feature, mechanism in 005.
- How 2FA seeds get entered at create time; whether "generate without saving" should
  keep a "recently generated" record (leaning toward not).

## 6. Roadmap

- **Shipped:** create wizard, edit, delete, generator, conflict dialog.
- **Next:** 2FA seed entry. Reliability improvements land with 005.
