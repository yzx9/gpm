# Android plugin unit-test coverage

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Extend JVM (Robolectric) unit-test coverage to three areas of the local
Android plugins whose logic carries a security or UX contract but currently has
none: the biometric error-code vocabulary surfaced by the keystore plugins, the
clipboard-clear notification's manual-clear flag state machine (with its native
tap receiver), and the Storage Access Framework file-picker's byte-read and
name-resolution helpers. Tests follow the existing pattern of extracting pure
logic into testable functions. The AndroidKeyStore AES/GCM sealing — the
plugins' most security-critical logic — is explicitly out of scope: it cannot
run under Robolectric and is left to a future instrumented-test or
injectable-cipher effort.

## Why

Until the Test workflow learned to run the plugin Robolectric suite (a single
safe-area test until now), this code had no CI gate at all, by necessity. That
capability now exists, so the question becomes what is worth covering. The three
areas above are the uncovered surface where a silent regression has a real cost:

- A new framework biometric error code silently falling into the default bucket
  changes the fallback UX the app layer branches on, and the mapping is
  duplicated across two plugins with nothing keeping them in sync.
- A shift in the manual-clear flag's reset/consume ordering revives the exact
  race the design closed — a late auto-clear fire wiping clipboard content the
  user placed after a manual tap.
- A boundary or empty-stream bug in the file-picker read path corrupts
  identity-file bytes on their way into Rust.

None of these is caught today. They are small, mostly pure, and cheap to pin
now that the Robolectric runner is wired into CI.

## Context

### The testing pattern

The safe-area plugin already established the shape: the per-edge insets
computation was extracted into a pure, Activity-free function and exercised
against constructed insets objects. The same pattern applies here — extract the
pure core, test it in isolation, and do not attempt to drive the `@Command`
entry points themselves, which would require mocking the Tauri plugin runtime
for low signal. Where logic is currently inlined into a command body or bound
to a framework handle, a small extraction (a function taking plain values, or a
helper that accepts an input stream rather than a content URI) is the
precondition for testing it.

### The three areas

**Biometric error vocabulary.** The keystore plugins translate the framework's
integer biometric error codes into a small stable string vocabulary the app
layer branches on. The translation is a pure mapping and is presently
duplicated verbatim across the two keystore plugins. A parameterized test pins
every branch and the default fallback, so a newly added framework code cannot
silently land in the wrong bucket and the two copies cannot drift.

**Clipboard-clear manual-clear flag.** The sticky-clear notification's
correctness rests on a three-way flag state machine: the notification post
resets the flag before showing (it always precedes any tap); the native tap
receiver sets it after clearing the clipboard and dismissing the notification;
and the armed-timer wake atomically reads and resets it to decide whether to
skip its own clear. The invariant — post precedes any tap, consume is atomic —
is precisely what prevents a late timer fire from clobbering unrelated
clipboard content the user placed after a manual clear. The state machine and
the receiver's combined effect (clipboard emptied, notification dismissed, flag
set) are testable with Robolectric.

**File-picker read path.** The Storage Access Framework picker streams a picked
content URI fully into memory and resolves a display name, falling back to the
URI's tail when the provider offers no name. The byte-read loop and the fallback
are pure or near-pure; the read helper needs to accept an input stream rather
than resolve the URI itself, so it can be fed constructed streams (including
empty and exact-boundary sizes).

### The boundary: AndroidKeyStore crypto

The keystore plugins' AES/GCM sealing — generate key, init cipher, seal, unseal
— is the most security-relevant logic in the plugin layer, and it is the part
this RFC deliberately does not cover. It is bound to the AndroidKeyStore
Provider, which Robolectric does not provide, so the sealing cannot run on the
JVM. Unit coverage therefore reaches the plumbing and the contracts (error
vocabulary, flag state machine, byte reads) but not the core crypto. Closing
that gap means either instrumented tests on an emulator (a materially heavier
CI lift) or refactoring the cipher provider to be injectable so a software
AES/GCM can stand in for tests (a change to threat-relevant code). Both are
justified on their own and are not bundled here; this RFC records the boundary
so the coverage is not mistaken for complete.

## Alternatives considered

- **Drive the `@Command` methods end-to-end by mocking the Tauri plugin
  runtime.** Rejected — high effort, low signal, and a departure from the
  established extract-and-test pattern.
- **Instrumented tests on an emulator, covering everything including the
  crypto.** Rejected as disproportionate for now; the emulator setup is heavy
  and only the crypto truly needs it. Parked, along with the injectable-cipher
  refactor, as the way to later reach the crypto.
- **A shared Gradle module to de-duplicate the biometric vocabulary across the
  two keystore plugins.** The cleanest long-term home, and the right fix for
  the duplication, but a structural change independent of adding tests. This
  RFC covers the vocabulary where it currently lives; the dedup is noted as a
  follow-up rather than a prerequisite.
- **Leave coverage at the single safe-area test.** Rejected now that CI can run
  these suites cheaply; the uncovered surface is security-relevant and the
  marginal cost is small.

## Effort

~1 day (human) / ~1–2 hours (CC). The biometric vocabulary and the clipboard
flag state machine are quick wins (small extractions plus parameterized /
state-based Robolectric tests). The file-picker read path needs a minor
refactor to accept a stream rather than a content URI before it can be fed
constructed inputs.

## Depends on / Supersedes

Depends on the CI capability to run the plugin Robolectric unit tests in the
Test workflow (recently added, alongside gradle dependency caching for the
Android build). Complementary to **0037-clipboard-clear-notification**, whose
manual-clear state machine this covers, and to the keystore plugins'
stored-key liveness design.
