# Android plugin unit-test coverage

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Extend JVM (Robolectric) — and one Rust — unit-test coverage to the local
Android plugins' logic that carries a security or UX contract and currently has
none. Coverage is organized by the invariant each test protects, not by module
or test harness, across three clusters: keystore drift (pure helpers duplicated
across the two keystore plugins), the clipboard-clear manual-clear invariant (a
Kotlin state machine plus the Rust consume decision that is its other half), and
the Storage Access Framework file-picker's byte-read and display-name path. Tests
follow the established pattern of extracting the pure core and exercising it
against constructed inputs; the clipboard Rust leg additionally needs a small
refactor that makes the consume decision injectable so its load-bearing branch
is reachable from a host test.

The AndroidKeyStore AES/GCM sealing — the plugins' most security-critical logic
— stays explicitly out of scope: it cannot run under Robolectric, and the
injectable pattern the consume-decision refactor introduces is recorded as the
template the crypto successor would follow.

## Why

Until the Test workflow learned to run the plugin Robolectric suite (a single
safe-area test until recently), this code had no CI gate at all. That capability
exists now, so the question becomes what is worth covering. The three uncovered
surfaces below are where a silent regression carries a real cost:

- A new framework biometric error code silently falling into the wrong bucket
  changes the fallback message the app layer surfaces, and the mapping is
  duplicated across the two keystore plugins with nothing keeping them in sync —
  alongside two more duplicated pure helpers carrying the same drift risk.
- A shift in the manual-clear flag's reset/consume ordering revives the exact
  race the design closed — a late auto-clear fire wiping clipboard content the
  user placed after a manual tap. The decision that prevents this is split
  across a Kotlin state machine and a Rust wake task, and the Rust half is the
  load-bearing one.
- A boundary or empty-stream bug in the file-picker read path corrupts
  identity-file bytes on their way into Rust.

None of these is caught today. They are small, mostly pure, and cheap to pin
now that the Robolectric runner is wired into CI — though one (the Rust consume
leg) is only reachable after a small injectable-decision refactor, because on a
host test the Android bridge it consults is an inert stub.

## Context

### The testing pattern

The safe-area plugin established the shape: the per-edge insets computation was
extracted into a pure, Activity-free function and exercised against constructed
insets objects. The same applies here — extract the pure core, test it in
isolation, and do not attempt to drive the command entry points themselves,
which would require mocking the plugin runtime for low signal. Where logic is
currently inlined into a command body or bound to a framework handle, a small
extraction is the precondition for testing it: a read helper that accepts an
input stream rather than a content URI; a visibility change so private helpers
can be exercised directly; a state-machine test driven through its
set/reset/consume transitions. One extension is new to this effort: a plain
Rust unit test, which needs the consume decision made injectable (below)
because the Android bridge it consults is a no-op stub on the host.

### The coverage surface

**Cluster 1 — Keystore drift (pure-JVM).** Three pure helpers are duplicated
verbatim across the two keystore plugins and have no tests: the biometric
error-code mapper, the localized prompt-text resolver (title/subtitle/negative
with generic fallbacks), and the class-name-only exception redactor (the
"never leak secrets in error messages" guard). The invariant is that the two
copies agree. The error mapper deserves a nuance: only a few codes (cancel,
key-invalidated, wrong-passphrase) drive distinct app-layer paths; the rest
collapse to a default branch where the Kotlin-supplied localized message is what
the user actually sees, and one code is absent from the frontend's type union.
So for the collapsed codes the test's value is preserving the correct message
bucket — which sharpens the drift motivation, since a drift can misroute a code
into the wrong bucket with no control-flow change at all. A non-duplicated
sibling is uncovered too: the auth-free store's presence and Base64 round-trip
helpers, and the pre-API guard backstops (the short-circuits that keep the
API-30-only cipher helpers from verify-erroring on older devices).

**Cluster 2 — Clipboard-clear, end-to-end (Robolectric + one Rust unit).** The
invariant: a late auto-clear fire must never clobber clipboard content the user
placed after a manual tap. It is enforced cooperatively across two layers. The
Kotlin layer is a three-way flag state machine — the notification post resets
the flag before showing (post always precedes any tap); the native tap receiver
sets it after clearing the clipboard and dismissing the notification; and the
consume helper atomically reads and resets it. Two API-level guards belong to
the same posture: the Android-13 notification-permission gate and the
immutable-PendingIntent flag branch. The Rust layer is the other half: the
armed wake task consults the consume-flag bridge and, if it returns true,
returns without clearing — the actual decision that prevents the clobber. That
branch is unreachable from a host test today (the bridge is an inert stub
off-Android), so this RFC scopes a small refactor that makes the consume
decision injectable into the wake task; a Rust test then injects a
true-returning decision to prove the self-skip and a false-returning one to
prove the auto-clear fires. This is the same run-mobile-plugin wall the crypto
boundary (below) parks as a successor; the injectable pattern is how this half
is crossed now, and the template for the crypto later.

**Cluster 3 — File-picker read path (pure-JVM).** The Storage Access Framework
picker streams a picked content URI fully into memory and resolves a display
name, falling back to the URI's tail when the provider offers no name. The
byte-read loop (empty, boundary, exact-boundary sizes) and the display-name
fallback are siblings; both need to accept a stream or a cursor rather than
resolve the URI themselves, so they can be fed constructed inputs (including
empty and exact-boundary sizes) without a real provider.

### Prerequisites — the gate these tests depend on

The coverage above is invisible to CI until a gate exists. Stated at design
altitude (the durable run recipe and build wiring live in CLAUDE.md's testing
section and the gradle/CI files, not here, because this file is deleted when
the feature ships and that knowledge is perennial):

- Only one plugin currently has the Robolectric and JUnit test dependencies
  wired; every other target plugin module has an empty test-dependency block,
  so each must gain them before any test compiles.
- Both the local test command and the CI workflow hardcode that single module,
  so tests added to any other module are not exercised. An aggregated entry
  point that fans out across every plugin's JVM test is the durable fix — and
  the reason it matters is the point of this RFC: a test the gate never runs
  regresses silently, which is exactly the failure mode these tests exist to
  close.
- Gradle cannot even configure for these JVM-only tests without a
  build-generated, gitignored settings file produced only by a full
  single-ABI build. The CI already pays this cost (a debug build runs purely
  to materialize the file); it is the gating constraint on the whole effort.
- The aggregator must guard that file's absence on a clean checkout, since the
  plugin projects do not exist in the configuration until it is generated.

### The boundary: AndroidKeyStore crypto

The keystore plugins' AES/GCM sealing — generate key, init cipher, seal, unseal
— is the most security-relevant logic in the plugin layer, and the part this
RFC deliberately does not cover. It is bound to the AndroidKeyStore Provider,
which Robolectric does not provide, so the sealing provably cannot run on the
JVM (it is unavailable on the harness, not merely inconvenient). Closing that
gap means either instrumented tests on an emulator (a materially heavier CI
lift, needing an API-30+ emulator and the real Provider) or refactoring the
cipher provider to be injectable so a software AES/GCM can stand in for tests
(a change to threat-relevant code). The clipboard cluster's injectable-consume
refactor is the small-scale proof of the latter pattern; the cipher version is
the same shape applied to crypto, and would also unlock the seal-liveness
probes (the "is the stored key still usable?" gates that steer cold-launch UX).
Both are justified on their own and are recorded here as the named successors
so the coverage is not mistaken for complete.

## Alternatives considered

- **Drive the command methods end-to-end by mocking the Tauri plugin runtime.**
  Rejected — high effort, low signal, and a departure from the established
  extract-and-test pattern.
- **Instrumented tests on an emulator, covering everything including the
  crypto.** Rejected as disproportionate for now; the emulator setup is heavy
  and only the crypto truly needs it. Promoted from a parked note to an
  explicit crypto successor, alongside the injectable-cipher refactor.
- **A shared Gradle module to de-duplicate the keystore drift across the two
  plugins.** The cleanest long-term home and the right fix for the duplication,
  since it lets one test cover both plugins where this RFC otherwise writes each
  test twice. A structural change independent of adding tests, so deferred —
  but the per-plugin tests this RFC adds double as characterization coverage
  that locks current behavior ahead of that refactor.
- **Reach the clipboard Rust consume branch via an instrumented Android test
  instead of the injectable refactor.** Rejected — the injectable refactor
  makes the branch reachable in a cheap host test; reserving an emulator for it
  is disproportionate when only the crypto truly needs one.
- **Leave coverage at the single safe-area test.** Rejected now that CI can run
  these suites cheaply; the uncovered surface is security-relevant and the
  marginal cost is small.

## Effort

~3–4 days (human) / ~0.5–1 day (CC). The test-infrastructure prerequisite
(per-plugin dependency wiring, the aggregated entry point, the missing-file
guard) is the dominant cost; the test-writing itself is the smaller half. The
clipboard injectable-consume refactor is a small change to one wake task, and
the Rust true-branch test that depends on it is near-free once injected.

## Depends on / Supersedes

Depends on the CI capability to run the plugin Robolectric unit tests in the
Test workflow (recently added, alongside gradle dependency caching for the
Android build), and on the per-plugin test wiring and aggregated gate described
in the Prerequisites section. The durable run recipe belongs in CLAUDE.md's
testing section once that aggregated gate exists. Complementary to the keystore
plugins' stored-key liveness design. The two crypto successors (instrumented
emulator tests; an injectable-cipher refactor) are the recorded forward path for
the out-of-scope boundary, not dependencies.
