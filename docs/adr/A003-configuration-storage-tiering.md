# A003: Configuration Storage Tiering — Git, Sealed Files, Plaintext Files

**Status:** Accepted

**Date:** 2026-07-09

**Context**

gpm persists state across several mechanisms — a version-controlled git
repository, a repository-scoped config file, an application-scoped config file,
and the WebView's `localStorage` — whose roles, scope, and protection level were
documented only in scattered code comments. Two concrete harms came from that:

1. **Scope conflation.** The repository-scoped config (`repo.json`) — per-clone
   metadata holding git credentials and the authenticity trust set — also carried
   application-scoped behavior preferences (auto-lock mode, the view/clipboard
   auto-clear timers, autosync, the app-lock flag). Those preferences were wiped
   when the repository was re-set up and traveled with per-remote metadata that
   is otherwise repository-local.
2. **No placement rule.** When adding state, there was no written test for which
   store owns it, and the right answer depends on three orthogonal axes that
   were each only implicit in the code.

The conflation also blocks a future multi-repository design: repository-scoped
data must be cleanly separable into a per-repo unit, which it cannot be while
application preferences are mixed in.

This ADR records the tiering model and the placement rule. The full
classification rationale is in RFC 0038; implementation details (the one-time
migration, how moved values cross the crate boundary, the reset file surface)
live in the code and are out of scope here.

## Decision

Adopt a **three-tier persistence model**:

1. **Git** — the cloned gopass repository of age-encrypted secrets,
   version-controlled and synced via `git pull`/`push`. The only tier that leaves
   the device.
2. **Sealed files** — `repo.json` (repository-scoped config), `identity`, and the
   sealed `app.json` behavior slot, sealed at rest with authenticated encryption
   where the platform supports it (Android); plaintext where it does not
   (desktop).
3. **Plaintext files** — `pref.json` (application-scoped display preferences),
   always plaintext.

The application config is split across two files by read timing. `pref.json`
holds the few display preferences that must render before the at-rest key is
available (display language, color scheme, the verbose-logging deadline, the
migration schema version, the deprecated screen-capture bool). `app.json` holds
the rest — the behavior preferences that are security-relevant choices but are
not read until after unlock (auto-lock mode, the auto-clear timers, autosync, the
app-lock flag, the screen-capture mode). The WebView's `localStorage` is **not**
a tier.

R064 further splits the sealed tier's key into two: `repo.json` + `app.json` sit
under an **auth-free** master key (permanently retrievable without a prompt, so
the headless background worker can pull-sync), while `identity` + its passphrase
slot sit under a **vault key** that follows the App Lock toggle (biometric-gated
when App Lock is on). The placement default is "seal": a value is plaintext only
when it must be readable before the at-rest key is available.

## Why these tiers

Each tier answers a different combination of scope, protection need, and
readability:

- **Git** is for data that is per-repository and meant to be shared across
  devices — the secrets and their history. It is the only tier that crosses the
  device boundary, so it carries only what should cross it.
- **Sealed files** are for local metadata that must never be committed and that
  needs protection — **confidentiality** (git credentials, the identity) or
  **integrity** (the authenticity trust set is public data, but tampering with
  it — injecting a signing key, flipping the verification mode — is a
  first-class defended threat, and authenticated encryption is what detects it).
  Repository-scoped data (`repo.json`) and application-scoped behavior prefs
  (`app.json`) both land here; R064 then splits the key so repo-scoped data is
  auth-free (the worker reads it) while the identity sits behind the vault key.
- **Plaintext files** are for local metadata that is application-scoped (must
  survive a repository re-setup) and must be readable before the at-rest key is
  available. The display language is the forcing case: it drives first-paint
  rendering and the app-lock biometric screen, so it must be readable at setup
  when the app-launch biometric gate withholds the key — a sealed store would be
  unreadable exactly then. None of these preferences are confidential, and the
  local write attacker is an explicit non-goal of the threat model, so plaintext
  is consistent with it.

An earlier draft of RFC 0038 leaned toward sealing the whole application store
("encrypt by default"). A003 originally rejected that for the pre-unlock-
readability reason below; R060 later revised the stance to "seal by default,
plaintext only when a field must render before the at-rest key is available" —
so the behavior half of the app config is sealed, and only the display half
stays plaintext.

## How a value is placed

Every persisted value is placed by three axes:

1. **Scope** — _repository-scoped_ (tied to a particular remote/clone: git URL,
   credentials, the authenticity trust set, the commit identity) vs
   _application-scoped_ (independent of which repo is connected and surviving a
   repository reset: UI language, the screen-capture toggle, auto-clear timers,
   lock mode, autosync).
2. **Protection need** — _confidentiality_ (would a read attacker learning it
   cause harm?) and _integrity_ (would a successful tamper be a meaningful
   attack?). These are independent: the authenticity trust set needs integrity
   but not confidentiality.
3. **Pre-unlock readability** — must the value be readable or writable when the
   at-rest master key is **not** available (before identity/app unlock, or while
   the app-launch biometric gate is engaged)?

The placement rule, in priority order:

- If the value is the secret itself or its history and should travel across
  devices → **Git**.
- Else if it is repository-scoped and needs confidentiality or integrity →
  **sealed files** (`repo.json`).
- Else if it is application-scoped and must be readable pre-unlock → **plaintext
  files** (`pref.json`).
- Else (application-scoped and not needed pre-unlock) → **sealed files**
  (`app.json` behavior slot). The default is sealed.

Two non-obvious placements fall out of this rule:

- **The commit author identity stays repository-scoped**, even though it looks
  application-scoped ("the user's" identity). It varies per repository —
  different repos, different signing identities — so it belongs with the
  per-clone metadata, not with device preferences.
- **`localStorage` is never authoritative.** The operating system may clear it,
  so it cannot back any setting; it is at most a transient, self-healing cache,
  and no setting relies on it. This is a project-wide stance, recorded here
  because the pre-unlock-readability axis is exactly where the temptation to
  reach for `localStorage` is strongest.

## Consequences

- **The repository-scoped unit is self-contained.** With application preferences
  out of `repo.json`, a future multi-repository design is a relocate into a
  per-repo directory, not a disentanglement. (The restructure itself is
  deferred.)
- **The plaintext surface is known and bounded.** `pref.json` is the only
  plaintext config file, and its contents are the few non-confidential display
  preferences that must render before unlock. The threat model is unchanged:
  at-rest encryption still defends a read attacker and provides integrity for
  the sealed tier; the local write attacker remains an explicit non-goal.
- **Placement is now a written rule.** New state is placed by the three axes
  above, not by ad-hoc judgment; a value that does not fit the rule is a signal
  that either the value or the rule needs a second look.
