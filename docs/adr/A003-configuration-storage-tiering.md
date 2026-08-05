# A003: Configuration Storage Tiering — Git, Sealed Files

**Status:** Accepted

**Date:** 2026-08-05

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
migrations, how moved values cross the crate boundary, the reset file surface)
live in the code and are out of scope here.

## Decision

Adopt a **two-tier persistence model** (no plaintext config tier):

1. **Git** — the cloned gopass repository of age-encrypted secrets,
   version-controlled and synced via `git pull`/`push`. The only tier that leaves
   the device.
2. **Sealed files** — `repo.json` (repository-scoped config), `identity` + its
   passphrase slot, and the single sealed `app.json` (all application-scoped
   preferences), sealed at rest with authenticated encryption where the platform
   supports it (Android); plaintext passthrough where it does not (desktop).

The application config is a **single sealed file** (`app.json`) holding both the
display preferences (language, color scheme, the verbose-logging deadline, the
background-sync cadence, the migration schema version) and the behavior
preferences (auto-lock mode, the auto-clear timers, autosync, the app-lock flag,
the screen-capture mode). The WebView's `localStorage` is **not** a tier.

R064 splits the sealed tier's key into two: `repo.json` + `app.json` sit under
an **auth-free** master key (permanently retrievable without a biometric prompt),
while `identity` + its passphrase slot sit under a **vault key** that follows the
App Lock toggle (biometric-gated when App Lock is on). **R074 loads that auth-free
master key at app startup, always — including under App Lock** — so the sealed
`app.json` is readable at first paint (the pinned locale/theme bake into the
WebView's init scripts before the window is created). This is what collapsed the
former plaintext `pref.json` tier: the premise for it ("display prefs must render
before the at-rest key is available") no longer holds.

### Key boundary — the auth-free master key is not what App Lock protects

App Lock gates the **vault key** (the identity, `setUserAuthenticationRequired`).
The auth-free master key seals only `repo.json` (git credentials) + `app.json`
(app preferences) and is **not** gated by App Lock — it is loaded at startup even
while the gate is locked. This is safe because:

- The auth-free key is the git-credential tier — already loaded by the headless
  background worker while the app is locked, so a locked app reading `app.json`
  adds no new exposure.
- A process/memory attacker is an explicit non-goal of the threat model (the seal
  defends a _read_ attacker / forensic dump, not an in-process one).

So loading the auth-free key at `.setup()` vs `app_unlock` is security-irrelevant:
the identity (the thing App Lock actually protects) stays behind the vault key
either way. This removes a pre-R074 asymmetry: the locked foreground previously
could not read `repo.json`/`app.json` (the auth-free key was deferred to
`app_unlock`), while the headless worker always could — both now read them at
startup, matching the worker's pre-existing access. No secret the gate protects
became readable. (This boundary must not be mistaken for "locked ⇒ no key at all.")

## Why these tiers

Each tier answers a different combination of scope and protection need:

- **Git** is for data that is per-repository and meant to be shared across
  devices — the secrets and their history. It is the only tier that crosses the
  device boundary, so it carries only what should cross it.
- **Sealed files** are for local metadata that must never be committed and that
  needs protection — **confidentiality** (git credentials, the identity) or
  **integrity** (the authenticity trust set is public data, but tampering with
  it — injecting a signing key, flipping the verification mode — is a
  first-class defended threat, and authenticated encryption is what detects it).
  R064 splits the key so repository-scoped data + app preferences are auth-free
  (readable at startup and by the background worker) while the identity sits
  behind the vault key.

An earlier revision of this ADR carried a third, **plaintext** tier (`pref.json`)
for display preferences that had to render before the at-rest key was available.
R060 sealed the behavior half but kept the display half plaintext for that
reason; R064 then made the master key auth-free, and R074 loads it at startup —
which removed the premise entirely. The config tier now holds **zero plaintext**.

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
3. **Pre-unseal readability** — must the value be readable or writable before
   the at-rest seal can be opened?

The placement rule, in priority order:

- If the value is the secret itself or its history and should travel across
  devices → **Git**.
- Else if it is repository-scoped and needs confidentiality or integrity →
  **sealed files** (`repo.json`, under the auth-free master key).
- Else (application-scoped) → **sealed files** (`app.json`, under the auth-free
  master key). **The default is sealed.**

The third axis is currently **degenerate — it has zero occupants.** Because the
auth-free master key is loaded at startup (R074), every sealed value is readable
at first paint, so no value needs a plaintext file. The axis stays defined
because it is still the right question to ask: a future value that genuinely
cannot tolerate the startup window before the Keystore unseal lands (or that
must never be sealed at all) would require a new plaintext file — but none exists
today, and adding one would be a deliberate, visible exception, not the default.

Two non-obvious placements fall out of this rule:

- **The commit author identity stays repository-scoped**, even though it looks
  application-scoped ("the user's" identity). It varies per repository —
  different repos, different signing identities — so it belongs with the
  per-clone metadata, not with device preferences.
- **`localStorage` is never authoritative.** The operating system may clear it,
  so it cannot back any setting; it is at most a transient, self-healing cache,
  and no setting relies on it.

## Consequences

- **The repository-scoped unit is self-contained.** With application preferences
  out of `repo.json`, a future multi-repository design is a relocate into a
  per-repo directory, not a disentanglement. (The restructure itself is
  deferred.)
- **The plaintext config surface is empty.** There is no plaintext config file;
  the entire config tier (`repo.json` + `app.json`) is sealed at rest on Android.
  The threat model is unchanged: at-rest encryption defends a read attacker and
  provides integrity for the sealed tier; the local write attacker remains an
  explicit non-goal. On desktop there is no Keystore, so sealed files are plaintext
  passthrough there — desktop has no at-rest protection by design.
- **App Lock's scope is precise.** App Lock gates the identity (the vault key),
  not the app config or git credentials. The auth-free master key is loaded at
  startup regardless of the lock, which is safe per the key boundary above.
- **Placement is now a written rule.** New state is placed by the three axes
  above, not by ad-hoc judgment; a value that does not fit the rule is a signal
  that either the value or the rule needs a second look.
