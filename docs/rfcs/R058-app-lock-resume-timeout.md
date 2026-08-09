# App-Launch Lock: Opt-In Timeout for Cross-Background Re-Lock

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Today the gate re-locks on every foreground return — cold start and warm resume alike. Add an opt-in setting that replaces every-resume with timeout-based resume re-lock: within the timeout, a foreground return resumes the app unlocked; past it, the gate has engaged and the user re-authenticates. The setting is off by default, so today's every-resume behavior remains the default; users who accept the trade for better UX opt in.

Serves the app-lock feature (`docs/specs/007-app-lock/`).

## Why

Every-resume is the strongest stance but the most intrusive: a momentary app switch and back forces a fresh biometric prompt. This RFC makes the timeout relaxation opt-in rather than flipping the default, so the secure behavior stays the default and the regression is a conscious user choice.

The regression lands on the threat model's named attacker — "brief physical access to an unlocked device." Today, any background→foreground transition wipes the master key before content is reachable, so an attacker who picks up an unlocked device and switches into gpm is stopped. Under the timeout, that attacker succeeds if they resume gpm within the window: the master key is still resident and the store opens unlocked. The change widens the accepted exposure from "zero seconds after the app leaves the foreground" to "up to the timeout," against that same in-scope attacker. The out-of-scope lines do not move: a process-running or root attacker could already ask the Keystore to unseal the master key regardless, so for that attacker the two designs are equivalent.

## Context

**Evaluation happens at the foreground-return instant, not via a background timer.** An in-process timer cannot be relied on to fire while the app is backgrounded — the OS may suspend the process under Doze, or kill it outright. The robust shape is to record the instant the app leaves the foreground and, on every return, compare elapsed time against the timeout. A process killed in the background never evaluates — it cold-starts locked, which is the desired result (the master key died with the process). This is the same "background-duration threshold" shape the every-resume design was chosen over; this RFC makes that reversal opt-in.

**Reuses the mask and reason from R057.** A foreground return past the timeout raises the same non-dismissable mask, and because the user just came back it auto-prompts (the reason is "return past timeout," not "idle"). Within the timeout, no lock fires at all.

**Threat-model posture.** This is the one of the two app-lock timeout RFCs that modifies the security model: it is an opt-in widening of the in-scope exposure window, documented as a deliberate user-chosen trade rather than a silent default flip.

## Alternatives considered

- **Flip the default to timeout (make the relaxation the default).** Rejected: it makes the regression the default for every user. Opt-in keeps the secure default and makes the trade explicit.

- **Wipe at the instant of backgrounding rather than on resume/timeout.** Rejected for the same reason as before: the background transition is the less reliable of the WebView's two foreground/background signals, and it does not serve the timeout goal anyway.

- **Tie the wipe to OS screen-lock instead of a timeout.** The strongest available "the user is gone" signal and the option with the smallest regression (if the screen is locked, the device is OS-protected regardless). Rejected for now because Android does not expose screen-lock to the WebView reliably — it needs the same kind of native lifecycle hook the authoritative-resume work envisions — and because screen-lock alone does not cover in-app idle. Worth revisiting once a native hook exists; it could subsume part of this design.

- **Short grace-only debounce (re-lock only if away more than a few seconds).** A milder version that avoids re-locking on accidental rapid app switches. Rejected: it opens the same in-scope window as a full timeout, just shorter, while not delivering the minutes-long grace that motivates the change.

## Effort

~S–M (human) / ~S (CC): the foreground-return timeout evaluation, the opt-in setting and its settings surface, and tests around the timestamp-on-return comparison and the within-window vs. past-window split. Reuses the mask and reason infrastructure from R057.

## Depends on / Supersedes

Depends on R057 (in-app idle timeout) for the mask overlay and reason field. Reverses, as an opt-in, the "every resume re-challenge" policy that the authoritative foreground-resume signal (sourced from Android's `Activity.onResume` via `RunEvent::Resumed`) hardened; that signal's goal of being authoritative stays valid and complementary — only the re-challenge policy is relaxed, and only for users who opt in.
