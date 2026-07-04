# Clipboard-clear sticky notification

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

When a secret is copied to the clipboard, post a sticky Android notification
that (a) signals a secret is currently held on the clipboard, (b) clears the
clipboard immediately when the user taps it — silently, without bringing the
app to the foreground — and (c) destroys itself, or replaces itself with a
brief "cleared" notice, when the clipboard is cleared, whether that clear came
from the user's tap or from the existing auto-clear timer. The notification is
a UI affordance layered over the already-armed backend timer; the timer remains
the security backstop, never the notification.

## Why

Today the only path to an empty clipboard is waiting out the auto-clear timeout.
There is no visible signal that a secret is sitting on the clipboard, and no way
to short-circuit the window once the user is done pasting. The most common
workflow — copy from gpm, switch to another app, paste, done — leaves the secret
on the clipboard for the full timeout with no reminder and no manual clear.

A notification fixes both, but only if tapping it clears *without* yanking the
user back into gpm. The user taps "clear" from inside the browser or login form
they just pasted into; foregrounding gpm on the tap destroys that context. So
the manual-clear action must be processed natively on the Android side, with no
activity launch and no WebView round-trip.

## Context

### Current behavior

Copy decrypts, writes the clipboard, and arms a fire-and-forget backend timer
that writes empty text after the configured timeout (default 45s). There is no
notification, no manual-clear path, and no cancellable handle on the armed
timer — each copy spawns an independent timer that runs to completion.

### Notification lifecycle

Three points: **post** on successful copy; **manual-clear** on user tap;
**auto-clear** when the armed timer fires. Manual-clear and auto-clear must
converge on the same end state (empty clipboard, notification gone or replaced
with a transient "cleared" notice) and must not race each other.

### The hard constraint: native tap handling

The official Tauri notification plugin routes every tap through an activity
intent, which foregrounds the app. That is incompatible with the
paste-elsewhere workflow. So the notification, its in-place update logic, and
its tap handler must live in a local Android plugin, sibling to the existing
keystore / safe-area / file-picker plugins. The tap intent is a broadcast
handled natively: clear the clipboard, dismiss or replace the notification, and
emit a backend event so the armed timer can be cancelled. The secret never
crosses into the WebView for the clear action — consistent with the existing
posture where the clipboard write itself is driven from the backend and the
password never reaches the WebView.

### State coherence: a single cancellable armed timer

The armed timer must become cancellable so a manual clear cancels the pending
auto-clear instead of racing it. That means replacing the fire-and-forget spawn
with a single tracked, replaceable handle held in backend app state. This
change is justified independently of the notification: today, copy-A then
copy-B leaves two independent timers running, and A's earlier fire clears B's
secret short of its full timeout. The notification work adopts the same
single-active-handle model — at any moment there is at most one armed timer and
one live notification, keyed together so a fresh copy re-arms both and any
clear tears both down.

### The notification is not authoritative

The timer is the security backstop; the notification is best-effort UI. Two
cases must not be modeled backwards:

- **Android 14+** lets the user dismiss even an "ongoing" notification by swipe.
  A user-dismissed notification must *not* cancel the armed timer — the secret
  is still on the clipboard and the timer must still fire. Dismissal is purely
  cosmetic.
- If the app process is killed mid-window, both timer and notification die and
  the clipboard retains the secret. The presence of a notification is never
  proof that the timer will run.

### Permission and graceful degradation

Android 13+ requires runtime `POST_NOTIFICATIONS`. Denial (or the permission
being revoked later) must degrade silently: copy and auto-clear continue to work
exactly as today, with no notification. The feature is an opt-in UX improvement
and never a dependency of the clear guarantee.

### Desktop

There is no equivalent "persistent shade notification" on desktop. Desktop
either skips this feature or falls back to a transient toast on copy. That
decision is platform-specific and out of scope here; the local plugin is
Android-only, mirroring the keystore plugins' platform asymmetry.

### Foreground service — explicitly not now

The one way to harden the process-kill case is a foreground service for the
clear window (which Android requires to show an ongoing notification anyway, so
the notification and process-persistence become one mechanism). For a ~45s
window this is disproportionate and surfaces a persistent "gpm is running in the
background" notice that is alarming for a password manager. Rejected for now;
revisit only if the timer proves unreliable on real devices in practice.

## Alternatives considered

- **Official `tauri-plugin-notification`**: rejected because its taps are
  activity intents that foreground the app, breaking the paste-elsewhere
  workflow that is the whole motivation for the feature.
- **Foreground the app on tap and clear via JS**: rejected for the same workflow
  reason.
- **Foreground service for the clear window**: rejected as disproportionate for
  ~45s; revisitable if process-kills prove real in the field.
- **Keep the JS timer as the clear owner and bolt on a notification**: the timer
  is already backend-owned (correctly — background WebView timers throttle);
  the notification does not change that, and the manual-clear path is added
  natively alongside.
- **Do nothing**: auto-clear already guarantees eventual emptying, so this RFC
  is a UX/affordance improvement, not a security fix. It exists because the
  manual-clear path and a visible "secret is on the clipboard" signal are
  material to the day-to-day copy-paste workflow and shorten real exposure.

## Effort

~2–3 days (human) / ~2–3 hours (CC). A new local Android plugin (notification
post/update/dismiss plus a native broadcast tap handler), a small backend
command and event surface, the single-cancellable-handle refactor of the armed
timer (justified on its own by the copy-overlap fix), and the Android manifest
plus capability permission wiring. Desktop deliberately excluded.

## Depends on / Supersedes

None. Builds on the existing copy + auto-clear flow; complementary to
**0003-secure-clipboard**, which addresses JVM-side secret retention in the
clipboard write path — orthogonal to the notification surface this RFC adds.
