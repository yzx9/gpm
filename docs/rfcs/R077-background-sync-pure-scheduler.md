# Strip background-sync down to a pure WorkManager scheduler

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Reduce `tauri-plugin-background-sync` to a generic periodic-WorkManager
scheduler — schedule a cadence, cancel, report whether scheduled, network-gated
— carrying **no** sync, crypto, or gpm semantics. The actual sync work (the
headless worker, the headless master-key retrieve, and the crossing into Rust
that performs the git pull) relocates from the plugin to the app's own Android
code, where it sits beside the key constants the app already owns.

Serves `docs/specs/005-git-storage/` (background sync), building on R061
(periodic sync) and R064 (sync under App Lock). Like R076, this is an internal
refactor of shipped internals, not a new feature.

## Why

The plugin's name promises it owns "background sync," but the only
sync-specific things it contains are the headless master-key retrieve (a
self-contained duplicate of the keystore plugin's auth-free retrieve — the D8
"third copy") and the worker that drives the JNI into Rust's git-pull. The
scheduling itself — enqueueing and cancelling periodic, network-gated
WorkManager work — is a generic primitive with nothing gpm-specific about it.

Two problems follow from bundling them:

- **The third copy.** The headless retrieve is a duplicate of one keystore
  plugin's auth-free retrieve path, kept in sync with the canonical alias/prefs
  only by a prose cross-reference (the D8 fallback adopted because the
  cross-plugin Gradle dependency is unproven). A rename of the canonical alias
  passes every fast gate green while silently breaking the headless decrypt.
- **Mixed altitude.** A "background-sync" plugin that also owns crypto access
  and a JNI binding is not a publishable primitive; the other refactored
  plugins are generic, and this one is the holdout.

The pure-primitive refactor already moved the canonical alias/prefs into the
app layer. Co-locating the headless retrieve with those constants is more
cohesive than keeping it in a plugin that is supposed to be generic.

## Context

**Current shape.** The plugin enqueues a periodic, network-gated worker. When
the worker fires, it (a) skips if the app is foregrounded (the foreground sync
owns convergence and holds the cross-process repo lock), (b) reads the auth-free
master key directly from the Android Keystore, and (c) crosses into Rust via a
JNI entry that performs the git pull and reports status. Steps (b) and (c), and
the worker itself, are gpm-specific; the enqueue/cancel is not.

**The crux — the headless key problem (do not pretend to remove it).** The
worker runs in a WorkManager process that has no Tauri app handle, so it cannot
use the keystore plugin's retrieve (which is mediated by the plugin handle).
Any headless sync that needs the auth-free key must reach the Keystore
_directly_. This is why the headless retrieve exists at all, and it is
inherent to WorkManager's process model — not a mistake this RFC corrects.
What this RFC _does_ is stop pretending that direct access is a "plugin"
concern: it relocates it from a supposed-to-be-generic plugin into the app,
where it lives next to the key constants the app already owns, and shrinks the
plugin to the genuinely-generic scheduling surface.

**What moves where.**

- The plugin keeps only: schedule a periodic cadence, cancel, report
  scheduled, network preconditions. Rename it to reflect that it schedules
  work, not that it syncs.
- The app takes ownership of: the worker (its foreground-skip and retry policy
  are sync-specific), the headless master-key retrieve (now beside the
  canonical alias/prefs), and the JNI binding the worker calls. The app
  registers its worker class with the scheduler.

**Migration: none.** The cadence setting, the work request, and the
on-disk state are unchanged; only the code location of the worker and the key
retrieve moves. A scheduled tick before and after the refactor does the same
work against the same Keystore entry.

**Threat-model impact: none.** The same key is retrieved headlessly by the same
process family, for the same pull-only purpose, under the same R064 residual
(git-credential residency under App Lock). The RFC moves code, not the
security boundary.

## Alternatives considered

- **Status quo + D8 prose cross-reference.** The current state. Rejected as the
  resting position: the third copy stays a plugin concern, guarded only by a
  comment, and the plugin stays un-publishable as a generic primitive.
- **D8 primary — a shared Kotlin key-access module.** Both the app's worker and
  the keystore plugin would call one shared retrieve. Dedupes genuinely, but
  depends on cross-module Gradle wiring that is unproven under Tauri's
  composite build, and adds a build module to maintain. This RFC is the lower-
  risk path; the shared module remains a future option if the wiring is ever
  proven (and would then dedupe across app + the merged keystore plugin from
  R076).
- **Eliminate headless sync; foreground only.** Rejected: it reverts R061/R064
  and defeats the feature for the heavy-autofill user who rarely opens the app.
- **Keep the worker in the plugin but drop the key retrieve (have Rust own it).**
  Rejected: Rust in the headless worker's process has the same lack of app
  handle — moving the retrieve into Rust does not remove the need for a direct
  Keystore touch, it only hides it behind another layer.

## Residual risks (what we accept)

- **The headless direct-Keystore access remains.** It is inherent to
  WorkManager headless execution. This RFC relocates it to the app (co-located
  with the key constants); only a shared Kotlin module would truly dedupe it,
  and that wiring is deferred.
- **App-owned Android Kotlin.** The app gains a small Android source set (the
  worker, the retrieve, the JNI binding). Tauri permits app-level Android code;
  the exact home is an implementation detail, but it is a new place that must be
  kept in sync with the workspace Gradle wiring.

## Effort

~M (human) / ~M (CC). Move the worker, the headless retrieve, and the JNI
binding into the app's Android sources; shrink the plugin to schedule/cancel/
is-scheduled; rewire worker registration; rename the plugin; re-run the full
Android build and confirm a headless tick still pull-syncs on a device. Larger
than R076 because it touches the app's Android surface and the JNI ownership,
not just crate packaging.

## Depends on / Supersedes

- Builds on R061 (periodic background sync) and R064 (sync under App Lock); the
  worker's foreground-skip, retry policy, and pull-only contract come from
  there and stay.
- Resolves the D8 `MasterKeyAccess` duplicate by **relocating** it to the app
  rather than deduping it in place — the cross-plugin shared module (D8 primary)
  remains a future option, now against a single merged keystore plugin (R076).
- Serves `docs/specs/005-git-storage/`; preserves the R064 threat-model
  residual (git-credential residency under App Lock is unchanged).
