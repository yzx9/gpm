# App-Launch Lock: In-App Idle Timeout & Identity Coupling

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Add an in-app idle timeout to the opt-in app-launch biometric lock: when the app stays foregrounded but untouched for longer than a configured timeout, the gate engages — wipes the master key and the identity cache — and raises a non-dismissable mask overlay. Introduce a reason tag on the gate's lock state so an idle re-lock raises the mask without auto-firing the biometric prompt (the user is present but idle; let them tap), while a cold start or foreground return keeps today's auto-prompt. And: while identity-auto-unlock is on, the identity session's independent auto-lock is ignored and its lifecycle follows the gate; while it is off, the two stay fully independent. The cross-background resume behavior is unchanged by this RFC — it still re-locks on every foreground return; relaxing that is a separate decision (see R058).

Serves the app-lock feature (`docs/specs/007-app-lock/`).

## Why

Today the gate re-locks only on foreground return — there is no in-app idle trigger on the gate. A user who leaves the app foregrounded and walks away keeps the master key resident until the next resume. An in-app idle timeout closes that gap. This is purely additive: it can only engage an extra lock, never defer one, so it tightens the posture slightly and does not alter the threat model. The identity-coupling rule is a lifecycle/UX simplification — it makes the auto-unlock toggle the explicit "couple identity to the gate" switch — not a security change (the identity passphrase is already accepted to sit in memory for the session, like the master key).

## Context

**The mask overlay is load-bearing.** The gate seals the whole store — config included — so once the master key is wiped, nothing sealed is readable. Unlike the identity-only lock (where the master key remains and secret-free browsing continues under a dismissable prompt), a gate lock must raise a non-dismissable mask: a secret-bearing page left visible after the key is gone makes the lock cosmetic.

**The idle timer reuses the in-app activity signal the identity idle timer already consumes** — it arms on activity and fires after the timeout.

**A reason tag drives the auto-prompt rule.** The gate's lock state today carries only whether it is enabled and whether it is locked. This RFC adds a reason (idle vs. cold-start/return): an idle re-lock raises the mask but does not auto-fire biometric; a cold start or foreground return auto-prompts as it does today. The foreground-return lock continues to fire on every resume here — only the idle case is new.

**Identity coupling follows the auto-unlock toggle.** While identity-auto-unlock is on, the identity session has no independent auto-lock — it is restored by the gate's unlock and wiped by the gate's lock. While it is off, the identity keeps its own immediate / idle-timeout / never policy and is not auto-restored on gate unlock (it re-authenticates per operation). A gate lock always incidentally wipes the identity cache; "independent" refers to the identity's own auto-lock policy and restoration.

**Settings surface.** A timeout control for the in-app idle. The identity auto-lock control is hidden while identity is coupled to the gate.

## Alternatives considered

- **One shared idle timer for the gate and the identity.** Rejected: the gate seals config too, so its idle must raise the non-dismissable mask — a different overlay behavior than the identity's dismissable prompt. Coupling the timers would tangle the two overlays. Keep the timers separate, fed by the same activity signal.

- **Auto-fire the biometric prompt on idle too.** Rejected: the user is present but idle (likely right there, screen still on); firing a prompt in their face is jarring. Let them tap.

- **Make the idle mask dismissable, like the identity prompt.** Rejected: the gate wipes the master key, so a dismissable mask would leave a secret-bearing page reachable with the store unreadable — a broken half-state. It must be non-dismissable.

## Effort

~M (human) / ~M (CC): the in-app idle trigger, a reason field on the gate state, frontend wiring for the non-dismissable mask and the reason-based auto-prompt rule, the identity-coupling rule keyed off the auto-unlock toggle, the settings surface, and tests around the idle firing and the idle-vs-return reason split.

## Depends on / Supersedes

Builds on the shipped app-launch biometric lock and the existing identity-auto-unlock opt-in. Provides the mask-overlay and reason-field infrastructure that R058 (opt-in cross-background timeout) reuses.
