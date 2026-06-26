# Authoritative Re-Lock Signal for the App-Launch Biometric Lock

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Make the app-lock re-trigger authoritative by sourcing the "app returned to the
foreground" signal from the native Android activity lifecycle instead of the
WebView's visibility event, so the master key is reliably wiped and re-challenged
on every resume regardless of how a given OEM WebView fires visibility
notifications.

## Why

0028's app lock wipes the master key on resume, and that wipe only happens if the
frontend receives a foreground signal. Today that signal comes from the WebView's
visibility event. The wipe is therefore _on resume, not at the instant of
backgrounding_: while the app sits in the background, before the resume signal
fires, the master key remains loaded. That is consistent with the threat model —
a process-running attacker is an explicit non-goal, and the key sits in memory no
longer than the git credentials gpm already holds during a clone or sync.

The residual concern is robustness, not a model gap: the WebView's visibility
event is the norm on Android but is not contractually guaranteed on every OEM
build. If a resume ever failed to fire that event, the key would stay loaded
until the next resume that does — the lock would silently fail to re-engage for
that one open, with nothing observing that the wipe did not happen. Sourcing the
signal from the native activity lifecycle makes it authoritative, because that
callback is what Android itself guarantees on every foreground transition,
independent of WebView behavior. This removes the silent-failure mode rather than
closing a threat-model hole.

## Context

The app lock's re-challenge is a foreground trigger: it does not matter exactly
when during the background period the key is wiped, only that it is gone before
app content is reachable on return. Two signals could drive it:

- The **WebView visibility event** (current) is convenient — it already flows into
  the existing frontend lifecycle wiring — but its fire-on-resume behavior is a
  de-facto norm, not a platform contract, and its failure is silent.
- The **native activity resume callback** is the platform's authoritative
  foreground transition. It is guaranteed by Android on every resume and is
  independent of the WebView. Emitting the foreground signal from here makes the
  re-lock the OEM-independent, WebView-independent truth.

The hook should drive the same downstream path the visibility event drives today
(wipe the master key and the identity cache, then re-raise the app-lock overlay)
— it is a signal-source swap, not a new mechanism. The native-side lifecycle
integration follows the existing plugin pattern gpm already uses to bridge
Android platform events to the frontend, so it needs no new kind of wiring.

Two edge cases shape it:

1. **The loop guard survives.** The gate already ignores foreground signals while
   a biometric prompt is in flight, so the prompt's own show/dismiss cannot
   re-trigger the gate. Moving the signal source must preserve that guard, or it
   risks a prompt-shows-then-immediately-re-locks loop.
2. **Android process death is unaffected.** If the OS killed the process in the
   background, the key was never restored after the last wipe; a cold start
   already re-challenges. This hardening is about the warm-resume path
   specifically, where the process lived and the key was re-loaded.

**Threat model.** No change. A process-running attacker remains an explicit
non-goal; this hardening removes an OEM-dependent silent failure in the re-lock
for the local-opportunistic attacker the model _does_ defend, so that the lock
re-engages as reliably as the platform allows. Desktop is unaffected (no app lock
there).

## Alternatives considered

- **Keep the WebView visibility signal.** Rejected as the long-term answer: it
  works in practice but rests on OEM behavior that is not guaranteed, leaving a
  silent-failure mode in a security-critical trigger. Acceptable to ship behind
  for now (as 0028 does), but it should not be the resting state.
- **Wipe at the instant of backgrounding rather than on resume.** Rejected: it is
  tempting because it would shrink the in-memory window to the backgrounding
  moment, but the background transition is the _less_ reliable of the two WebView
  events on Android, and even a native background callback offers no real gain —
  the threat model already accepts the key in memory while the process runs.
  Wipe-on-resume is the right semantic; this RFC only makes that resume signal
  authoritative.
- **Add a background-duration threshold so only long backgrounds re-challenge.**
  Rejected (as in 0028): adds a knob and a timing race for no security gain over
  "every resume," and the re-challenge is already every resume.

## Effort

~S (human) / ~S (CC): a native activity-lifecycle hook in the existing Android
plugin layer that emits a foreground signal, plus swapping the app-lock's signal
source to it while preserving the in-flight-prompt loop guard. No crypto, no new
persistence, no threat-model change — the validation work is confirming the loop
guard and the warm-resume path across a background/foreground cycle.

## Depends on / Supersedes

Extends 0028 (whose master-key wipe-on-resume this makes authoritative) and 0022
(whose resume-re-challenge model and in-flight-prompt loop guard it preserves).
Does not supersede either.
