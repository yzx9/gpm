<!--
Feature-level threat model for entry access: copy / show, screen capture, clipboard.
Complements docs/SECURITY.md (system-wide model + the core copy/show principle). Living.
-->

# 001 — Entry Access: threat model

## Assets & trust boundary

The decrypted secret. Boundaries = the Rust ↔ WebView IPC line, the screen, and the
clipboard. Two operation paths with different exposure: `copy_password` (primary) never
crosses the IPC boundary; `show_password` (secondary) intentionally exposes the secret
to the DOM in order to display it.

## `copy_password` — primary (no IPC exposure)

Decrypted in Rust, written directly to the clipboard, never crosses to the WebView. JS
receives only `CopyResult { success, entry_name, cleared_after_secs }`. The clipboard is
auto-cleared after the configured timeout via a background task.

While a secret is on the clipboard, gpm also posts a sticky Android notification
(tappable to clear early). **This is a UX affordance, not a security control** — the
auto-clear timer above is the control and runs independently of it. The notification
needs Android's `POST_NOTIFICATIONS` permission; once the user dismisses or denies that
prompt twice, Android stops re-asking, so the **Settings → Permissions & data** screen
deep-links back to the system's per-app notification settings to re-enable it. That page
closes a discoverability gap, not a safety one: a copy always proceeds and the timer
always clears whether or not the notification posts.

## `show_password` — secondary (intentional IPC exposure)

Decrypted in Rust, returned as `SensitiveContent { password, notes }` for display — the
inherent cost of rendering on screen. Mitigations: auto-clear timer; cleanup on
navigation (`popstate`), unmount (`onBeforeUnmount`), and manual dismiss; never logged
or persisted.

## Screen capture protection (Android)

`WindowManager.FLAG_SECURE` is set on secret-bearing pages (setup, create, generate,
entry detail, settings — settings renders the SSH key on export). It blanks screenshots,
screen recording, and the Recents/task-switcher thumbnail. The entry list (names only)
and history (signatures) carry no secret and stay capturable.

Per-route, gated by a user master toggle (default on): `secure = toggle && route.secret-bearing`.
Applied in the nav guard _before_ the page paints; `MainActivity` sets it on at boot as a
safe default. A guard that cannot confirm the flag on a secret-bearing route aborts the
navigation and toasts, rather than render unprotected.

Caveats: Android-only (desktop has no equivalent); bypassable on rooted devices (e.g.
Magisk "Disable Flag Secure"); non-secret list/history capturable by design; the toggle
is a device preference that survives a repo reset. Component-level granularity (securing
just a reveal on an otherwise-capturable page) is deferred future work.

## `select-all` on password display

The display element uses `select-all` CSS for manual copy; on mobile this may interact
with the system clipboard unexpectedly. The "Copy Password" button is the primary copy
mechanism and avoids this.

## Cross-references

- Core copy/show principle, JS memory & accessibility limits: `docs/SECURITY.md`.
- Auto-clear / lifecycle cleanup align with 007's AutoLock identity-cache lifecycle.
