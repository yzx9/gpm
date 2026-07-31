# Stack-based overlay back-handler

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

On Android, the hardware/gesture back button reaches the webview as a single
broadcast event that **every** registered listener receives — so when two
overlays are on screen at once, one back press dismisses both. gpm's overlay
primitive today registers a back listener **per instance** and assumes only one
user-facing overlay is ever up, coordinating the rare two-overlay case by hand
within a single component. That assumption breaks the moment a dismissable
dialog (a confirm/prompt) opens on top of an already-open sheet that lives in a
different part of the component tree: back cancels the dialog **and** tears down
the sheet underneath.

This RFC proposes replacing the per-instance listeners with one **app-wide,
stack-based back-handler registry**: each overlay pushes a handler on mount and
pops on unmount, and a back press invokes **only the top of the stack**. Serves
the in-app dialog/overlay system — most directly the app-lock and identity
overlays in spec `007-app-lock`, and the shared dialog host.

## Why

Today's model is correct for a single overlay and wrong for any cross-component
stack. The first user-reachable cross-component stack is a label-entry prompt
opening over the authenticity-block sheet: pressing back to cancel the prompt
also closes the sheet, silently dropping the context the user was acting on.
It looks like a bug.

The current workaround — a component neutralizes its own lower layer's listener
while an upper step is open — only works when **one component owns both layers**.
It cannot coordinate two layers owned by different components, and it forces
every multi-layer component to remember to wire the neutralization. That's easy
to forget and only discoverable by manual testing on a device.

A stack registry fixes the whole class: any future overlay can stack safely
with zero per-call-site coordination, the existing within-component workaround
can be retired, and back behaves the way Android users expect — it closes the
topmost thing only.

## Context

**Android back is one global signal.** It is surfaced to JavaScript as an event
that all listeners receive (a broadcast, with no DOM-style capturing/bubbling
and no "topmost-only" semantics). Whoever wants back to close an overlay instead
of navigating the webview away must subscribe to it.

**How gpm handles it today.** A single shared overlay primitive takes over back
while any overlay is up; each mounted instance subscribes its own listener. The
design chose per-instance over a shared registry on purpose, reasoning that the
only multi-overlay case was the app-launch lock stacked over a page modal — and
there both listeners firing is harmless, because the lock overlay doesn't act on
back. The scroll-lock, solving the analogous "two stacked things, only the last
release should undo" problem, already uses a shared ref-counted controller — so a
stack/counter pattern is an established idiom in this codebase, not a new idea.

**Why the old reasoning held, until now.** Every multi-overlay case was either
(a) within one component (coordinated by neutralizing the lower listener while
the upper step is open), or (b) a lock overlay where double-fire is benign. A
user-reachable dialog-over-sheet from **separate components** is a new shape
that neither assumption covers — and the broadcast means it isn't caught by any
DOM-level event isolation (a backdrop tap is isolated; the back key is not).

**Threat model.** None. This is pure navigation/UX correctness. No change to how
secrets are handled, decrypted, or wiped; overlays already clean up their own
state on close regardless of how they were dismissed.

## Alternatives considered

1. **Per-instance, with a "close the lower sheet before opening the upper
   dialog" rule at each call site.** Rejected: pushes a correctness burden onto
   every caller, is easy to forget, doesn't generalize, and leaves the latent
   double-fire for any future stack. Treats the symptom, not the cause.
2. **Per-instance, plus a global "a dialog is open" flag every overlay reads to
   disable its own listener.** Rejected: couples every overlay to the dialog
   system, is leaky, and still broadcasts — listeners merely choose to ignore.
   Fragile and no cleaner than today.
3. **Let the dialog host own the back key only while a dialog is up, suppressing
   others ad hoc.** Rejected: same cross-cutting coupling, and it doesn't help
   non-dialog stacks (e.g. a future sheet-over-sheet).
4. **The proposed app-wide stack registry (recommended).** One chokepoint, LIFO,
   only the top fires. Generalizes to every overlay and every future stack, and
   retires the within-component neutralization workaround rather than adding to
   it. The registry itself is small; the cost is the test pass, not the design.

## Effort

~1 day (human) / ~30 min (CC). The registry is small and mirrors an existing
ref-counted controller pattern already in the app. The effort — and the risk —
is the **blast radius**: every overlay (lock, identity, divergence,
authenticity, dialogs) routes its back handling through the new chokepoint, so
the work is mostly a careful test pass: stack ordering, pop-on-unmount, and
preserving the existing stale-registration guards (a listener superseded or
unmounted mid-registration must still be dropped so a single back press can
never fire into a dead overlay).

## Depends on / Supersedes

None. It **enables** (but does not depend on) the in-app `prompt` migration,
which can safely stack a text-entry dialog over an open sheet once this lands.
