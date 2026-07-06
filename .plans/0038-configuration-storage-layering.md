# Configuration Storage Layering and Classification

**Priority:** P1
**Status:** Draft
**Phase:** Next

## What

The app persists state across three mechanisms — a repository-scoped config
store, an application-scoped app-shell config store, and the WebView's
`localStorage` — whose roles, scope, and protection level are documented only in
scattered code comments. This RFC defines a classification model (by scope, by
protection need, and by pre-unlock readability), assigns each existing kind of
state to a tier, and adopts an "encrypt by default" posture, so the placement of
future state (starting with UI language) follows a written rule rather than
ad-hoc judgment.

## Why

Three concrete confusions stem from the undocumented model today:

1. **Scope conflation.** The repository store — the per-clone config that holds
   git credentials and the repository authenticity trust set — also carries
   application-scoped behavior preferences (auto-clear timers, lock mode,
   autosync, commit-identity metadata). Those preferences ride on data they do
   not belong to: they are reset when the repository is re-set up, and they
   travel with repository data that is otherwise per-remote.
2. **Protection-level conflation.** The repository store is sealed at rest for
   two distinct reasons that are never separated: **confidentiality** (git
   credentials, the identity) and **integrity** (the authenticity trust set,
   which is public data but tamper-critical). Conflating them makes the
   per-field question "is this sensitive?" unanswerable — yet that is exactly
   the question that governs whether a value may move to a less-protected tier.
3. **No ownership rule.** When adding state, there is no written test for which
   store owns it, and the right answer depends on three orthogonal axes that
   today are each only implicit in the code.

## Context

**Axes.** Every persisted value is placed by three axes:

- **Scope** — _repository-scoped_ (tied to a particular remote/clone: git URL,
  credentials, the authenticity trust set, commit identity) vs
  _application-scoped_ (independent of which repo is connected and persisting
  across a repository reset: UI language, the screen-capture toggle, auto-clear
  timers, lock mode, autosync). Application-scoped is the codebase's existing
  "device-level / app-shell" notion.
- **Protection need**, on two independent sub-axes: _confidentiality_ (would a
  read attacker learning it cause harm?) and _integrity / tamper-value_ (would a
  successful tamper be a meaningful attack?). These decouple: the authenticity
  trust set needs integrity but not confidentiality.
- **Pre-unlock readability** — must the value be readable or writable at a
  moment when the at-rest master key is **not** available (before identity/app
  unlock, or while the app-launch biometric gate is engaged)?

**Tiers.** The existing mechanisms map to:

- **Repository store** — repository-scoped data needing confidentiality or
  integrity. Sealed at rest where the platform supports it; plaintext otherwise.
  Holds git credentials, the identity, and the authenticity trust set with its
  verification mode. Integrity — not confidentiality — is why the public trust
  set lives here; tampering with it (injecting an attacker signing key, flipping
  the verification mode) is a first-class defended threat, and authenticated
  encryption is what detects it.
- **Application store** — application-scoped data, independent of which repo is
  connected, that must survive a repository reset. Its protection level is the
  open decision below.
- **WebView-side cache** (`localStorage`) — non-authoritative, synchronously
  readable hints. Never the source of truth; always a cache over an
  authoritative tier; self-healing on mismatch. Exists to bridge cold-start
  windows where an authoritative value is not yet readable.

**Posture — encrypt by default; plaintext only when necessary.** Encryption
cost is negligible and the store is unlocked on every launch anyway: the at-rest
master key is auth-free in the common case, so sealed state is available
immediately with no user friction. The burden of proof is therefore on
_declassifying_ state to plaintext, not on encrypting it. "Necessary" means a
hard pre-unlock-readability requirement that a WebView-side cache cannot meet.

**The one open decision — the application store's protection level.**

- _Option A — plaintext application store (today's state)._ Threat-model
  consistent: application preferences are not confidential, and the local write
  attacker is explicitly out of scope, so the integrity bonus on them is
  marginal. It also preserves the existing decoupling: the app-shell layer does
  not depend on the master-key/store lifecycle.
- _Option B — sealed application store._ Honors the encrypt-by-default posture
  and empties the plaintext surface, but couples the app-shell layer to the
  master-key injection lifecycle that Option A deliberately avoids. It also
  forces every pre-unlock-readable value onto the WebView-side cache — the
  screen-capture toggle is the first such case, and its secure-by-default boot
  behavior already makes a post-unlock reconciliation safe, so it needs no
  cache, only tolerance of a later reconcile.

**Recommendation:** adopt the scope split unconditionally (move
application-scoped preferences out of the repository store — a correctness fix
independent of encryption), and resolve the application-store protection per the
posture: **Option B**, with pre-unlock readability handled by the WebView-side
cache rather than a plaintext carve-out, so the plaintext surface stays empty by
default. Option A remains the documented fallback if the coupling cost proves
unjustified; it is not threat-model-required, only posture-preferred.

**Threat model.** No change. At-rest encryption continues to defend a read
attacker and provide integrity; the local write attacker remains an explicit
non-goal.

## Alternatives considered

- **Status quo (no model).** Rejected: the three confusions above are concrete
  and already blocking a placement decision for UI language; leaving placement
  undocumented guarantees the same confusion on the next piece of state.
- **Option A as the resting state.** Threat-model-defensible and cheapest, but
  it does not honor the encrypt-by-default posture the project has adopted; kept
  as the fallback above.
- **A single unified store (no scope split).** Rejected: scope is a real
  semantic boundary — application preferences must outlive a repository
  re-setup, and repository data must not leak across repos — and conflating them
  is the existing bug this RFC exists to fix.

## Effort

~M (human) / ~S (CC) for the model and classification alone. ~M (human) / ~M
(CC) for the Option B implementation: seal the application store, migrate the
application-scoped preferences out of the repository store, and wire the
WebView-side cache for any pre-unlock-readable values. No crypto change, no
threat-model change.

## Depends on / Supersedes

Informs 0039 (internationalization) — the UI-language preference is the first
application-scoped value placed under this model.
