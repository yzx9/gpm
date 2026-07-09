# ADR 0003: Configuration Storage Tiering — Git, Sealed Files, Plaintext Files

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
2. **Sealed files** — `repo.json` (repository-scoped config) and `identity`,
   sealed at rest with authenticated encryption where the platform supports it
   (Android); plaintext where it does not (desktop).
3. **Plaintext files** — `app.json` (application-scoped config), always
   plaintext.

Split the two config files by scope: `repo.json` holds repository-scoped data
only (remote URL and credentials, clone path, commit author identity, the
identity-auto-unlock flag, the authenticity trust set); `app.json` holds
application-scoped behavior preferences (display language, the screen-capture
toggle, auto-lock mode, the auto-clear timers, autosync, the app-lock flag). The
WebView's `localStorage` is **not** a tier.

## Why these tiers

Each tier answers a different combination of scope, protection need, and
readability:

- **Git** is for data that is per-repository and meant to be shared across
  devices — the secrets and their history. It is the only tier that crosses the
  device boundary, so it carries only what should cross it.
- **Sealed files** are for local metadata that is per-repository but must never
  be committed, and that needs protection — **confidentiality** (git
  credentials, the identity) or **integrity** (the authenticity trust set is
  public data, but tampering with it — injecting a signing key, flipping the
  verification mode — is a first-class defended threat, and authenticated
  encryption is what detects it).
- **Plaintext files** are for local metadata that is application-scoped (must
  survive a repository re-setup) and must be readable before the at-rest key is
  available. The display language is the forcing case: it drives first-paint
  rendering and the app-lock biometric screen, so it must be readable at setup
  when the app-launch biometric gate withholds the key — a sealed store would be
  unreadable exactly then. None of these preferences are confidential, and the
  local write attacker is an explicit non-goal of the threat model, so plaintext
  is consistent with it.

An earlier draft of RFC 0038 leaned toward sealing the application store
("encrypt by default"). That is rejected here: the encrypt-by-default instinct is
sound in general, but it loses to a concrete pre-unlock-readability requirement
that a `localStorage` cache cannot safely back.

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
  **sealed files** (`repo.json` / `identity`).
- Else if it is application-scoped and must be readable pre-unlock → **plaintext
  files** (`app.json`).

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
- **The plaintext surface is known and bounded.** `app.json` is the only
  plaintext config file, and its contents are explicitly non-confidential
  behavior preferences. The threat model is unchanged: at-rest encryption still
  defends a read attacker and provides integrity for the sealed tier; the local
  write attacker remains an explicit non-goal.
- **Placement is now a written rule.** New state is placed by the three axes
  above, not by ad-hoc judgment; a value that does not fit the rule is a signal
  that either the value or the rule needs a second look.
