# Configuration Storage Layering and Classification

**Priority:** P1
**Status:** Decided (implemented)
**Phase:** Current

## What

The app persists state across **three tiers**. Their roles, scope, and
protection level are written down here so future state is placed by a rule, not
ad-hoc judgment:

1. **Git — the version-controlled repository.** The secret store itself:
   age-encrypted gopass entries synced by `git pull` / `git push`. This is the
   source of truth for everything per-repository and meant to travel across
   devices — the secrets and their history. It is the only tier that leaves the
   device.
2. **Sealed files — local, locked at rest.** Repo-scoped metadata that must stay
   with the device's clone but is never committed: connection credentials, the
   age identity, and the authenticity trust set. Authenticated-encryption-sealed
   at rest where the platform supports it (Android); plaintext where it does not
   (desktop). Sealed for **confidentiality** (credentials, the identity) and for
   **integrity** (the public trust set, where tampering — injecting a signing
   key, flipping the verification mode — is a first-class defended threat and
   authenticated encryption is what detects it).
3. **Plaintext files — local, unlocked.** Application-scoped behavior preferences
   that survive a repository re-setup and must be readable before the at-rest key
   is available: UI language, the screen-capture toggle, the auto-clear timers,
   auto-lock mode, autosync, the app-lock preference. Plaintext, deliberately.

The WebView's `localStorage` is **not a storage tier.** At most it is a transient
non-authoritative cache that bridges a cold-start window; it is never the source
of truth for any setting, because the operating system may clear it. No user
setting relies on it.

## Why

Three concrete confusions stem from leaving this model implicit:

1. **Scope conflation.** The repository store — the per-clone metadata that holds
   git credentials and the repository authenticity trust set — also carried
   application-scoped behavior preferences (auto-clear timers, lock mode,
   autosync). Those preferences rode on data they do not belong to: they were
   reset when the repository was re-set up, and they traveled with repository
   data that is otherwise per-remote.
2. **Protection-level conflation.** The sealed store protects for two distinct
   reasons that were never separated: **confidentiality** (git credentials, the
   identity) and **integrity** (the authenticity trust set, which is public data
   but tamper-critical). Conflating them made the per-field question "is this
   sensitive?" unanswerable — yet that is exactly the question that governs
   whether a value may move to the plaintext tier.
3. **No ownership rule.** When adding state, there was no written test for which
   tier owns it, and the right answer depends on three orthogonal axes that were
   each only implicit in the code.

## Context

**Axes.** Every persisted value is placed by three axes:

- **Scope** — _repository-scoped_ (tied to a particular remote/clone: git URL,
  credentials, the authenticity trust set, the commit identity) vs
  _application-scoped_ (independent of which repo is connected and surviving a
  repository reset: UI language, the screen-capture toggle, auto-clear timers,
  lock mode, autosync).
- **Protection need**, on two independent sub-axes: _confidentiality_ (would a
  read attacker learning it cause harm?) and _integrity / tamper-value_ (would a
  successful tamper be a meaningful attack?). These decouple: the authenticity
  trust set needs integrity but not confidentiality.
- **Pre-unlock readability** — must the value be readable or writable at a moment
  when the at-rest master key is **not** available (before identity/app unlock,
  or while the app-launch biometric gate is engaged)?

**Resolution — the application store is plaintext (Option A), and this is
forced.** The plaintext tier is not a shortcut but a hard requirement: the UI
language must be readable before unlock — it drives first-paint rendering and the
app-lock biometric screen — and a sealed store is unreadable at setup when the
app-launch gate is engaged, because the at-rest key is exactly what that gate
withholds. None of the application-scoped preferences are confidential, and the
local write attacker is an explicit non-goal of the threat model, so plaintext is
consistent with it. Sealing would couple the app-shell layer to the master-key
injection lifecycle for no threat-model gain, and would push every
pre-unlock-readable value onto a `localStorage` cache — a surface the project
deliberately avoids for settings.

This overturns an earlier draft of this RFC, which recommended sealing the
application store ("encrypt by default"). The encrypt-by-default instinct is
sound in general, but it loses to a concrete pre-unlock-readability requirement
that a cache cannot safely back. The three tiers above are the resting state.

**Scope split — a correctness fix independent of encryption.** The
application-scoped preferences were moved out of the repository store into the
plaintext tier. The split is the part that fixes the scope-conflation bug; the
plaintext-vs-sealed question is settled separately by the readability forcing
function above. Two repo-scoped-looking fields _stay_ repo-scoped by design: the
commit author identity (it varies per repository) and the identity-auto-unlock
flag (meaningful only against the repo-scoped identity).

**Other decisions recorded here:**

- **`reset_config` scope.** Reset wipes the repository clone, the identity, the
  sealed repo-scoped file, and the identity-passphrase slot. It spares the
  plaintext application store (device preferences survive a re-setup) and the
  Keystore-held at-rest key (app-scoped by design, so app-lock survives reset).
- **App-lock preference is write-only.** The Settings toggle and the runtime gate
  both read Keystore truth; the persisted preference flag is written on
  enable/disable for the record but never read for display, so a drifted flag
  cannot mislead the user about whether the gate is actually on.
- **One-time migration.** A schema-version-gated migration copies the
  application-scoped preferences out of the (pre-split) repository store into the
  plaintext tier on the first post-unlock. It is a temporary upgrade bridge for
  existing installs, not a permanent feature; fresh installs skip it (there is no
  pre-split store to read).

**Threat model.** No change. At-rest encryption continues to defend a read
attacker and provide integrity; the local write attacker remains an explicit
non-goal.

## Alternatives considered

- **Sealed application store (the earlier draft's Option B).** Rejected: it
  honors an encrypt-by-default posture but breaks pre-unlock readability of the
  UI language, and forces pre-unlock-readable values onto `localStorage`, which
  the project avoids for settings.
- **`localStorage` as a storage tier.** Rejected: the operating system may clear
  it, so it cannot be authoritative for user settings. It remains permissible
  only as a transient, self-healing cache — and no setting relies on it.
- **A single unified store (no scope split).** Rejected: scope is a real semantic
  boundary — application preferences must outlive a repository re-setup, and
  repository data must not leak across repos — and conflating them is the bug
  this RFC exists to fix.

## Effort

~M (human) / ~S (CC) for the model. ~M (human) / ~M (CC) for the implementation:
the scope split (move the application-scoped preferences, add the one-time
migration), and rewire the app-shell and frontend to the split stores. No crypto
change, no threat-model change.

## Depends on / Supersedes

Informs 0039 (internationalization) — the UI-language preference is the first
application-scoped value placed in the plaintext tier under this model.
