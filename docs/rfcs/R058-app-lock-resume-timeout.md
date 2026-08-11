# App-Launch Lock: Grace Window for Cross-Background Re-Lock

**Priority:** P2
**Status:** Accepted
**Phase:** Implementing

## What

Today the gate re-locks on **every** foreground return (cold start and warm resume
alike). With `gate_idle = After(N)` (R057), a return within `N` of the last activity
now **stays unlocked** (a grace window for quick app switches); past `N` it re-locks
and re-authenticates. `gate_idle = Off` keeps today's every-resume behavior unchanged.

Unifies the resume re-lock into the existing `gate_idle` setting (no new setting):
"re-lock after N seconds of disuse" applies whether the disuse is foreground-idle
(R057's idle timer) or backgrounded-away (this RFC).

Serves the app-lock feature (`docs/specs/007-app-lock/`).

## Why

Every-resume is the strongest stance but the most intrusive: a momentary app switch
and back forces a fresh biometric prompt. Reusing `gate_idle`'s N for the resume
window removes that friction for users who already accepted "re-lock after N of
inactivity," without adding a second setting. `Off` (the m0006 default for existing
users) is untouched, so the secure default is preserved.

The relaxation is a deliberate, opt-in widening of the in-scope exposure window
(from "zero seconds after the app leaves the foreground" to "up to N") against the
threat model's named attacker — "brief physical access to an unlocked device." New
installs default `gate_idle = After(300)`, so they get a 5-minute grace by default;
this is consistent with R057's choice to default the idle timer on. The out-of-scope
lines do not move (a process-running/root attacker could already ask the Keystore to
unseal the vault key regardless).

## Context

**Backend-authoritative, single state.** `last_activity_at` (a monotonic `Instant`)
lives in `AppState` and is updated at one chokepoint — `identity::reset_gate_idle_timer`
— so every caller stays in lockstep: `bump_idle_timer` (frontend taps) **and** the
~15 secret-op paths (read/write/revisions/…) that reset the timer directly, bypassing
`bump_idle_timer`. (Updating it in `bump_idle_timer` alone was the first draft; it
missed those direct callers and broke the invariant.) The resume re-lock
(`applock::app_lock`, made grace-aware via the `apply_resume_relock` core) reads it.

**Evaluated at the resume instant, not a background timer.** Android may suspend
(Doze) or kill a backgrounded process, so an in-process timer can't be relied on to
fire while away. The resume check (`now − last_activity_at ≥ N`) is the reliable
backup for when the idle timer was suspended; both share the same `last_activity_at`
and N, so they agree. A killed process cold-starts locked (desired).

**Reuses the existing `app_lock` command + `Return` reason.** `app_lock` already
disarms the gate idle timer before `do_app_lock` — reusing it (resume-only caller)
gets the disarm for free and needs no new command/registration/wrapper. A warm
resume into an already-locked app is a no-op (the existing overlay stays as-is); an
earlier "re-emit `Return`" promotion was dropped — it sent a spurious cold-start
`app_lock` ping that could race a just-finished unlock and re-lock `Off` users. No
new `AppLockReason` variant.

**D1 (grace timer semantics): total-disuse.** The grace branch does **not** disarm
the idle timer and does **not** stamp `last_activity_at`, so the window keeps counting
toward N across the backgrounding; switching apps can't reset it (that takes a real
secret op through the chokepoint). No lock-evasion by app-switching.

## Alternatives considered

- **A separate opt-in `gate_resume` setting.** Rejected: R057 defaults `gate_idle`
  **on** (`After 300`) for new users, while this RFC wants the resume relaxation tied
  to the same choice. A separate setting would be redundant and would force choosing
  which default wins (idle-on vs resume-strict); unifying makes the relaxation follow
  the user's existing `gate_idle` choice.

- **Option A: a backend `RunEvent::Suspended` → `app-backgrounded` bridge** (record
  the leave-foreground instant). Rejected: not needed. `last_activity_at`
  (foreground-trackable via the activity signal the backend already gets) measures
  total disuse and needs no "entered background" event, avoiding a new backend bridge
  plus the risk that `Suspended`/`Focused(false)` is unreliable on Android (R029
  already rejected the foreground `visibilitychange` as OEM-unreliable; `Focused(false)`
  fires for biometric prompts too).

- **A frontend `lastActivityAt` ref.** Rejected: it duplicates the backend's activity
  state and diverges (the secret-op resets happen server-side). One state, in the
  backend, is correct.

- **Flip the default to timeout.** Rejected: `Off` (the existing-user default) keeps
  every-resume; the relaxation only reaches users who opt into `gate_idle = After(N)`.

## Effort

~M (human) / ~S (CC): the chokepoint stamp, the grace-aware `app_lock`, the frontend
`onAppResume`/`useForegroundSync` adjustments, the `Idle`→`Return` reason promotion,
copy, and tests. Reuses R057's idle timer + R029's resume signal.

## Depends on / Supersedes

Depends on R057 (in-app idle timeout, for `gate_idle`) and R029 (authoritative
`app-resumed` signal). Reverses, as an opt-in via `gate_idle = After(N)`, the
"every resume re-challenge" policy R029 hardened; R029's authoritative-foreground-
signal goal stays valid and complementary — only the re-challenge policy is relaxed.
