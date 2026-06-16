# Password generator

**Priority:** P3
**Status:** Deferred (split out of the create-secret UI integration)
**Phase:** Post-MVP
**Depends on:** the in-app Create flow — the password field it augments already exists in `CreatePage.vue`.

> High-level intent only. No implementation detail — that comes when this plan
> is picked up. Sections per the split request.

## Context

gopass `create` generates a random password for the new secret by default, and
users expect a password manager to offer that. gpm's new Create flow currently
requires the user to type or paste the password themselves — there is no
generator anywhere in `rustpass` (no RNG usage today). This plan adds a
"generate" affordance to the Create password field so creating a secret doesn't
force a round-trip through another tool.

## Process

Add a generator in `rustpass` (CSPRNG — `getrandom` works on Android; pick a
sensible default: ~24 characters over an alphabet of `[A-Za-z0-9]` plus a few
safe symbols, excluding visually-ambiguous characters), exposed as a
`generate_password(length?, alphabet?)` Tauri command. In `CreatePage.vue`, add
a dice button next to the password/PIN field that fills it with a freshly
generated value. Configurable length and alphabet are deferred — ship one good
default first.

## Purpose

A generator removes the friction of switching to another app to invent a
password, and it produces stronger secrets than humans tend to choose. It
brings gpm's create flow to parity with gopass's default behavior.

## Gains

- Parity with gopass `create` (generate-by-default).
- Stronger, unique secrets than user-chosen ones, with zero extra effort.
- One less context switch during the most common "add a new login" task.

## Drawbacks

- New RNG dependency + the need to vet the CSPRNG source on Android (not just
  desktop).
- A policy decision: default length and alphabet (and whether to allow symbols
  that some sites reject).
- The generated value is a transient secret — it must be treated like a
  revealed password (never logged, cleared on unmount / lock), not stored in
  long-lived component state.

## Blockers

- **Transient-secret handling.** The generated password sits in the Create
  form's password field until the secret is saved. It must be wiped on unmount
  (the Create flow already wipes in-form secret values on unmount) and must
  never be logged. A "show / mask" toggle for the generated value is a policy
  call worth making before shipping.
- **Alphabet/length policy.** Decide the default and whether to expose it in
  Settings before or alongside the generator.
- **No existing RNG in the crate.** `rustpass` introduces its first randomness
  source here — review the chosen crate/source for Android correctness.
