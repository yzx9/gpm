# R086 — Entry-view decrypted-content cache

**Priority:** P1
**Status:** Draft
**Phase:** Next
**Revision:** 1

## What

Kill three cold-start UX warts on the entry detail screen under Immediate auto-lock (and on first entry into any auto-lock mode): the "pile of buttons" shown when the entry's type can't be probed without a passphrase, the dead-ends where tapping a speculative button (Export Attachment / Copy 2FA) unlocks and decrypts only to report "nothing here", and the rapid double-prompt when a user copies, then shows, then copies a 2FA code within seconds. Introduce a short-lived, view-scoped cache of the **decrypted entry** (not the master identity) so one unlock opens the whole entry view, and replace the cold button-pile with a single Unlock gate.

Serves `docs/specs/001-entry-access`.

## Why

The entry detail screen can only show the _correct_ action buttons once it has decrypted the entry enough to know whether it is a password, a 2FA, or a binary attachment — and that decrypt needs the identity, which under Immediate auto-lock is wiped after every operation (and is simply absent on a cold start before the first unlock). The mount-time affordance probe deliberately returns "unknown" rather than prompting, so before any unlock the screen falls back to showing **every** button at once. Three things go wrong:

1. **Button pile.** A plain password entry shows Export Attachment and Copy 2FA buttons that can never succeed — clutter that reads as "I have no idea what this entry is."
2. **Dead-ends.** Tapping one of those speculative buttons costs a full unlock + decrypt, then reports "no attachment" / "no 2FA". The user paid the unlock and got nothing.
3. **Rapid double-prompt.** Under Immediate, the master identity is wiped after each op, so copy → show → copy-2FA in quick succession prompts for the passphrase three times. The user came in knowing what they wanted; being asked repeatedly within ~10 seconds is the worst-feeling case.

## Context

**Core idea — cache the decrypted entry, not the identity.** On the first unlock in the entry view, decrypt the entry once, then **drop the identity immediately** (preserving Immediate auto-lock's contract that the master key does not linger) and keep the _decrypted entry content_ in backend memory for the rest of the view. Every subsequent operation in that view (copy password, show, copy 2FA, export attachment) reads from the cached decrypted content and needs no second unlock. This is a strictly smaller exposure than caching the identity would be: a process-memory disclosure reveals one entry's plaintext (already exposed if the user revealed it on screen) instead of the master key that decrypts the whole store. The project's threat model does not model an in-memory attacker; the residual reason to keep the window short is defense-in-depth against self-inflicted leaks (e.g. an accidental debug log line captured into the diagnostics bundle), not a modelled adversary.

**Cold start — a single Unlock gate.** When the identity is not cached, the entry view shows one neutral **Unlock** affordance instead of the button pile. Tapping it performs the one decrypt, populates the view-scoped cache, and only then renders the real, type-correct action set. Browsing users who open an entry without acting are not interrupted (they see the gate, not a prompt); acting users pay one unlock for the whole view. This eliminates both the pile and the dead-ends: speculative buttons are never shown, because the entry's type is known before any action button appears.

**Size threshold — don't cache large content.** Content above a size threshold is excluded from the cache and re-decrypts per operation. Large attachments are rare and low-frequency (export is a one-shot action with no rapid follow-up), so a re-prompt there is acceptable, and excluding them avoids amplifying the known large-attachment RAM cost (tracked separately as the streaming-decrypt refactor). The cache only ever holds small content; the _first_ decrypt of a large entry still loads it fully, which the streaming work addresses — the two are deliberately kept off each other's path (a note to that effect is on the streaming issue).

**Two timers, each living where its state lives.** The decrypted-content cache and the on-screen reveal are separate state with separate lifecycles, but both keyed to the same user setting — the existing reveal auto-clear window (including its "Never" option), so the cache honours the user's own habit rather than imposing a foreign timer. The **cache timer lives in the backend** (it clears backend memory), mirroring the backend idle-timer already used for identity auto-lock, and **slides** — each operation resets it — so active use keeps the view unlocked. The **reveal timer stays in the frontend** (it clears a screen), keeps today's behavior of arming fresh on each Show and not being extended by an unrelated copy. Because both timers share the same value, they expire together; a reveal never outlives its cache, so there is no "password still on screen but copy demands a passphrase" absurdity. Both wipe on navigation away (back, unmount, switching to another entry) and on a hard lock (manual or idle-timeout).

**Why not one shared timer.** A single timer governing both cache and reveal reintroduces the cold-start bug in a new form: a Show late in the cache window would inherit the cache's near-expired deadline and clear the password almost immediately. Two timers (same value, different reset rules) is the fix.

## Alternatives considered

- **Cache the identity instead of the decrypted content.** Lets every op re-decrypt cheaply, but holds the master key in memory for the view — a larger blast radius and a break of Immediate auto-lock's stated contract. Rejected for caching the _content_, which keeps the master key wiped.
- **Reveal-first, then copy through the UI.** Reuses the already-revealed plaintext for the copy so no second unlock is needed. Breaks for binary attachments (decoded bytes must never reach the UI) and 2FA (the seed must never reach the UI), and would route the copy through the WebView clipboard path — bypassing the sticky clear-notification + guaranteed-clear-timer machinery that makes copy safe. Only clean for the password-copy case; would still need this cache for the other entry types.
- **Frontend-only cleanup (hide the speculative buttons, keep everything else).** Removes the dead-ends and shrinks the pile but cannot remove the rapid double-prompt under Immediate, since it leaves the per-op wiping untouched. Solves the cheapest two of the three warts.
- **Metadata sidecar so the type is known without decrypting.** Would let the list/probe settle affordances with no identity — but leaks which entries carry 2FA or attachments, a metadata disclosure the store currently avoids. Rejected.
- **One shared cache+reveal timer, or a fixed cache timer independent of the reveal setting.** The shared timer causes the "Show near the deadline clears instantly" regression; a fixed cache timer whose value differs from the reveal window causes the "visible but copy re-prompts" absurdity whenever the reveal window is longer. Both rejected for "same value as the reveal setting, two timers."

## Effort

Medium. Backend: a view-scoped decrypted-entry cache + its backend timer (analogous to the existing identity auto-lock timer), and routing the read operations through the cache when populated. Frontend: replacing the cold button-pile with the Unlock gate and wiring the cache's lifecycle to the existing navigation/lock wipe hooks. Plus a threat-model note and tests for the timer/cache lifecycle.

## Depends on / Supersedes

Deliberately _not_ coupled to the large-attachment streaming-decrypt work (separate issue): this RFC caches only small content and routes large content to a re-decrypt path; the streaming work later removes the first-decrypt RAM cost for large entries. Aligns the decrypted-entry cache lifecycle with the identity auto-lock cache lifecycle described in `docs/specs/007-app-lock`.
