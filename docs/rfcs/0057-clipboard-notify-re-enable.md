# Clipboard-clear notification: in-app re-enable path

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

After Android permanently denies the `POST_NOTIFICATIONS` permission, gpm has no in-app path back to re-enabling the clipboard-clear notification. Add a Settings entry that detects the current notification-permission state and, when blocked, deep-links the user into the system's per-app notification settings so they can flip the toggle themselves.

## Why

The clipboard-clear notification is a UX affordance — a sticky notice while a secret is on the clipboard, tappable to clear it early. It needs `POST_NOTIFICATIONS` (Android 13+). gpm now requests that permission directly via the system dialog on the first copy, with no intermediate in-app prompt. Android's permission model stops re-showing that dialog after the user dismisses or denies it twice; from then on every copy proceeds (the auto-clear timer still guards the clipboard) but no notification appears, and the user gets no signal that _they_ hold the lever to bring it back. A user in this state reasonably concludes the notification feature is broken. CHANGELOG text reaches only the minority who read it. A Settings entry that reflects the live state and links to the system toggle closes the discoverability gap without re-introducing the friction of an in-app prompt on the copy path.

## Context

gpm's threat model treats the clipboard auto-clear _timer_ as the security control; the notification is purely informational and runs independently. So the gap is a discoverability problem, not a safety one. Relevant platform behavior: Android 13+ records two dismisses/denials of `POST_NOTIFICATIONS` as "don't ask again" and thereafter returns denied without UI; the system per-app notification-settings screen is the only recovery surface. The detection should read the live permission state, and the deep-link should target the system's app-notification-settings intent, not a re-prompt. A rationale shown only when blocked (not a pre-permission confirm) keeps the copy path prompt-free while still explaining _why_ the user might want notifications.

## Alternatives considered

- **Restore the in-app confirm before the system dialog.** Rejected: that is the friction this same work just removed, and it only ever appeared on the copy path, not where a permanently-denied user would look.
- **Periodically re-prompt the system dialog.** Rejected: the platform suppresses it after two denials, so this is both unreliable and annoying.
- **Do nothing.** The current state (this RFC's baseline): safe, but a discoverability trap for users who denied twice.

## Effort

~1–2 days (human) / ~20 min (CC): a new Settings row, a live enabled-probe on view, and a deep-link intent wired through the clipboard-notify plugin.

## Depends on / Supersedes

None.
