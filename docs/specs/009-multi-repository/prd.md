---
pm: Zexin Yuan
created: 2026-08-06
revision: 1
scope: repo
---

# 009 — Multi-Repository

> Status: Planned · Related: A001, A003 · Last verified: 2026-08-06

## 1. Introduction

Today gpm holds exactly one repository. This feature lets a user connect and switch
between several independent repositories ("vaults") on the same device — each its own
git remote, identity, authenticity policy, and sync settings — while a single App Lock
gates the whole app. The user is "in" one vault at a time: switching is a frequent,
header-level action, and adding / removing / configuring vaults is a rare Settings
action. A single-vault user sees no multi-repository surface at all.

## 2. Motivation / Objective

The gap is **vault-level isolation**. A user who keeps a work repo (company host, SSH,
commit verification at Enforce) and a personal repo (self-hosted host, different trust
circle) can today connect only one; "switching" means wipe-and-re-setup. Multi-repository
serves the case where the right boundary is a **separate git repository** — a different
remote, a different trust circle, or a different device origin — not merely different
roles within one repo (role-level separation, by directory and decrypting identity, is a
separate concern handled by multi-identity).

A001 recorded multi-repository as a deliberate post-MVP deferral, and A003 isolated the
per-repository configuration unit precisely so that this feature would be a **relocation
into a per-repository directory, not a re-architecture**. This PRD defines the product
requirements for that relocation.

## 3. Use Cases

- **Jordan** keeps several repositories that must stay separate at the transport layer: a
  work repo on the company Forgejo (SSH key, commit verification at Enforce, a work commit
  identity) and a personal repo on their own Forgejo (PAT, Audit mode, a personal identity)
  — possibly a third shared only with their partner. The trust circles must not be
  enumerable from one another's remote. They switch vaults from the header several times a
  day and configure each vault's authenticity, sync, and commit identity independently in
  Settings. A friend-shared repo or a local-only experiment also lives as its own vault.
- **Casey** has exactly one local vault; multi-repository is invisible to them — no
  switcher, no vault-management surface, an app identical to today. The only path to a
  second vault is an ignorable "add another repository" affordance in Settings. The one
  realistic Casey scenario — a vault a technical friend shared, plus their own local vault
  — works as two vaults, the shared one possibly read-only-ish.

## 4. Key Aspects

### Product Design

- The user is in **one vault at a time**. A header vault switcher — hidden until a second
  vault exists — switches the active vault, and is **switch-only**: no management actions
  live inside it. All add / remove / rename / per-vault configuration lives in Settings.
- **Single-vault invisibility (Casey):** with one vault, the main UI is identical to today;
  the only addition is an ignorable Settings affordance to add a second vault. A vault
  switcher or management list appears only once a second vault exists.
- **Role-level separation within one repo** (work / personal / partner directories under
  different decrypting identities) is explicitly _not_ what multi-repository is for; that
  is the separate multi-identity concern, and the two axes are orthogonal.

### Functionality

- Connect multiple repositories; switch the active one; add, remove, and rename each.
- **Search is scoped to the active vault** in v1; cross-vault search is deferred (post-v1).
- On launch, the app returns to the **last active vault**.
- Each vault carries its own remote URL, git credentials, commit identity, authenticity
  trust-set and verify mode, storage / crypto backend, age identity, recipients pin, and
  autosync toggle.

### Compatibility

- Each vault is an independent gopass-layout repository with its own git remote and standard
  Git protocol. This is **not** gopass-style transparent path-overlay mounts (a single
  unified tree routed by path) — gpm uses explicit vault switching.

### Interactive

- Header switcher (vaults ≥ 2) for frequent switching; a Settings "Repositories" area for
  rare add / remove / rename / configure. Vault names auto-derive from the remote URL and
  are editable; local-only vaults get an editable default name. The switcher shows each
  vault's name plus a hint (remote host, or "Local").
- **Removing a vault** deletes its local clone and per-vault config but **never touches the
  remote** (always re-cloneable). Confirmation differentiates recoverable (has a remote)
  from permanent-loss (local-only). Removing the last vault returns the app to first-run
  onboarding.

### Adaptive

- **Per-vault autosync** (each vault syncs automatically or manually on its own); a single
  background-sync schedule fans out to every vault with autosync on (no per-vault cadence).
- **Transparent upgrade:** an existing single-repository user becomes a one-vault user with
  zero data loss and no re-setup — identity, authenticity trust-set, sync settings, git
  credentials, and commit identity are all preserved; only the display name is auto-derived.

### Security

- **App Lock is one gate over the whole app:** one biometric prompt, and locking / wiping
  applies to all vaults uniformly.
- **Only the vault in use is decrypted.** Each vault has its own identity cache and unlocks
  independently, preserving the per-operation wipe model; a vault may opt into "unlock
  together with the app" via its existing per-vault toggle, so a trusted vault needs no
  separate prompt.
- **Shared app-lock keying:** one at-rest key seals every vault's config and identity.
  Compromising that key compromises all vaults' at-rest identity — consistent with the
  per-device threat model, where defeating the key already implies the device's App Lock is
  broken and all vaults are in scope regardless.
- See `docs/SECURITY.md` for the threat model this inherits.

### Reliability

- Upgrade is non-destructive and transparent (above). Removing a vault is recoverable
  whenever a remote exists. A vault whose key or config becomes unsealable degrades to
  re-setup **for that vault only**, not the others.

## 5. Open Questions & Key Decisions

Decisions (rationale in A001 / A003):

- **Explicit vault switching**, not gopass path-overlay mounts — the mobile-natural model.
- **Search scoped to the active vault** in v1; cross-vault search deferred.
- **Vault-level isolation only**; role-level separation stays with multi-identity.
- **Per-vault autosync**; **global background-sync cadence** (one schedule, fan-out).
- **Per-vault independent unlock**; **App Lock is one global gate**.
- **Shared app-lock keying** (one key seals all vaults); accept the all-vaults blast radius.
- **Transparent, zero-loss upgrade.**

Non-goals:

- gopass-style transparent path-overlay mounts / a unified tree.
- Cross-vault search in v1.
- Role-level separation within a repository (that is multi-identity).
- Hosted / cloud sync — sync stays git pull/push to your own repositories.
- Enterprise IAM / SSO / RBAC / audit.
- A hard cap on the number of vaults (soft guidance only).
- Merging / unifying two vaults' trees into one view.

Open questions:

- How the (not-yet-shipped) autofill feature resolves which vault an entry belongs to — a
  coordination point for when autofill lands.
- Per-vault sync-status and "attention" surfacing, since background-sync divergence is now
  per-vault rather than global.

## 6. Roadmap

- **Shipped:** nothing — this feature is greenfield; gpm is single-repository today.
- **Now:** this PRD (requirements). Design rationale and implementation follow.
- **Next / Future:** cross-vault search; per-vault background-sync cadence if real need
  emerges; coordination with autofill.
