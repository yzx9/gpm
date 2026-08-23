# Strip background-sync down to a pure WorkManager scheduler

**Priority:** P2
**Status:** Draft
**Phase:** Next
**Revision:** 1

## What

Reduce `tauri-plugin-background-sync` to a generic periodic-WorkManager
scheduler — schedule a cadence, cancel, report whether scheduled, network-gated
— that references **no** worker type: the caller supplies the worker class name
and the plugin builds the `PeriodicWorkRequest` from it at runtime. Carrying no
sync, crypto, or gpm semantics, it renames to `tauri-plugin-background-work`. The
actual sync work (the headless worker, the headless master-key retrieve, and the
JNI binding into Rust's git-pull) relocates from the plugin into the app's **own
Android source set** (`gen/android/app/src/main/java/xyz/yzx9/gpm/`, beside
`MainActivity.kt`), where it sits next to the key constants the app already owns
and the JNI core (`app/src-tauri/src/jni_sync.rs`) that is already app-side. The
worker's master-key retrieve becomes a single canonical copy in a shared
headless-bootstrap module that the future Autofill service (R056) will reuse.

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
  Moving the retrieve into the app source set, beside `keystore.rs`'s
  `MASTER_ALIAS`/`MASTER_PREFS`, makes it the canonical copy a rename touches in
  one place — and the shared bootstrap the future Autofill service (R056) reuses,
  so the dedup pays off twice, not once.
- **Mixed altitude.** A "background-sync" plugin that also owns crypto access
  and a JNI binding is not a publishable primitive; the other refactored
  plugins are generic, and this one is the holdout.

The pure-primitive refactor already moved the canonical alias/prefs into the app
layer, and the JNI core (`run_headless_sync`) is already app-side. Only the
worker's Kotlin — stranded in the plugin while its JNI counterpart sits in the
app — straddles a crate boundary for no benefit. Relocating it completes the
cohesion: both ends of the JNI live in the app, and the plugin shrinks to a
generic, publishable scheduler.

## Context

**Current shape.** The plugin enqueues a periodic, network-gated worker. When
the worker fires, it (a) skips if the app is foregrounded (the foreground sync
owns convergence and holds the cross-process repo lock), (b) reads the auth-free
master key directly from the Android Keystore, and (c) crosses into Rust via a
JNI entry that performs the git pull and reports status. Steps (b) and (c), and
the worker itself, are gpm-specific; the enqueue/cancel is not.

**The crux — the headless key problem (do not pretend to remove it).** The
worker runs in a WorkManager process that has no Tauri app handle, so it cannot
use the keystore plugin's retrieve (which is mediated by the plugin handle). Any
headless sync that needs the auth-free key must reach the Keystore _directly_.
This is why the headless retrieve exists at all, and it is inherent to
WorkManager's process model — not a mistake this RFC corrects. The Keystore key
is also non-exportable (hardware-backed), so the AES/GCM decrypt cannot be taken
over by Rust; it must go through Java's `Cipher`, which is why the retrieve is
Kotlin at all. What this RFC _does_ is stop pretending that direct access is a
"plugin" concern: it relocates it from a supposed-to-be-generic plugin into the
app's own source set, where it lives next to the key constants the app already
owns, and shrinks the plugin to the genuinely-generic scheduling surface.

**What moves where.**

- The plugin (`tauri-plugin-background-work`) keeps only: schedule a periodic
  cadence, cancel, report scheduled, network preconditions. It is worker-
  agnostic — `schedule(intervalHours, configDir, workerClassName)` resolves the
  class via `Class.forName(...)` and builds the request with the classic
  `PeriodicWorkRequest.Builder(Class, ...)`, so the plugin carries no
  compile-time reference to any worker. This is the Rust→Kotlin IPC bridge (the
  `set_background_sync` command calls `schedule` over plugin IPC); the JNI path
  is not the plugin's concern.
- The app's Android source set takes ownership of: the worker (its
  foreground-skip and retry policy are sync-specific), the headless master-key
  retrieve (now the single canonical copy beside `MASTER_ALIAS`/`MASTER_PREFS`),
  and the JNI symbol the worker calls (re-rooted from
  `Java_xyz_yzx9_gpm_backgroundsync_SyncWorker_nativeSync` to
  `Java_xyz_yzx9_gpm_SyncWorker_nativeSync`, so the JNI coupling becomes
  app-internal rather than app-Rust-knowing-plugin-Kotlin). The worker and
  retrieve sit in a shared headless-bootstrap module under
  `gen/android/app/src/main/java/xyz/yzx9/gpm/` that the future Autofill service
  (R056) reuses.
- The app's WorkManager/Keystore dependencies are injected into the `:app`
  Gradle module from `gen/android/settings.gradle` (the `gradle.beforeProject`
  hook that already applies the debug `applicationIdSuffix`) — not from
  `app/build.gradle.kts`, which Tauri regenerates.

**A pattern, not a one-off.** The headless-bootstrap problem (an OS-started
process with no `AppHandle` reaching the `Store` and keys) is shared by this
worker and the future Autofill service (R056). Resolving it once, in an
app-owned module, is the load-bearing reason to do this as a shared bootstrap
rather than per-service files — and it pre-answers the store-reachability half
of R056's prototype gate (the identity-unlock-from-cold-service half remains
open: the worker uses only the auth-free master key, never the vault key).

**Migration: near-none.** The cadence setting and on-disk state are unchanged.
The worker's simple name stays `SyncWorker`; its package moves to
`xyz.yzx9.gpm` (the JNI symbol moves with it), and the scheduler receives the
class name as a parameter instead of hard-coding `<SyncWorker>`. WorkManager
keys the periodic work by its unique name (`gpm_background_sync`), not the
class, so the schedule survives — though a tick that fires between the update
and the first post-update launch would fail to instantiate the moved class until
the app re-applies the cadence on launch (`enqueueUniquePeriodicWork(REPLACE)`).
A scheduled tick before and after the refactor does the same work against the
same Keystore entry.

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
  composite build, and adds a build module to maintain. This RFC makes it less
  pressing: the app-side shared bootstrap already dedupes the app's own two
  consumers (this worker + the future Autofill service). D8-primary remains a
  future option if the wiring is proven — then it would additionally dedupe the
  bootstrap's retrieve against the merged keystore plugin (R076).
- **Eliminate headless sync; foreground only.** Rejected: it reverts R061/R064
  and defeats the feature for the heavy-autofill user who rarely opens the app.
- **Keep the worker in the plugin but drop the key retrieve (have Rust own it).**
  Rejected: Rust in the headless worker's process has the same lack of app
  handle — moving the retrieve into Rust does not remove the need for a direct
  Keystore touch, it only hides it behind another layer.
- **A gpm-specific local plugin crate as the Android source-set carrier.** Carry
  the worker, the retrieve, and the Autofill service in one local plugin's
  `android/` dir (where deps live cleanly and the manifest merges), rather than
  in `gen/android/app/`. Rejected: it re-imports the exact altitude confusion
  this RFC removes — a "plugin" crate that is neither generic nor publishable —
  and the app source set already supports deps (via the `settings.gradle` hook)
  and direct manifest edits, so the carrier buys nothing.

## Residual risks (what we accept)

- **The headless direct-Keystore access remains.** It is inherent to headless
  execution (no `AppHandle` → no plugin-mediated retrieve, and the Keystore key
  is non-exportable so Rust cannot take the decrypt over). This RFC dedupes it
  across the app's own consumers (worker + future Autofill service share one
  bootstrap retrieve); only cross-plugin dedupe against the keystore plugin is
  deferred (D8-primary).
- **App-owned Android Kotlin + a deps-injection hook.** The app gains a small
  Android source set (the worker, the bootstrap retrieve, the JNI symbol), and
  its WorkManager/Keystore deps are injected into `:app` from
  `gen/android/settings.gradle`'s `gradle.beforeProject` hook (the same channel
  the debug `applicationIdSuffix` already uses) because Tauri regenerates
  `app/build.gradle.kts`. This is a new place to keep in sync, but it uses an
  existing, asserted mechanism rather than a new one.
- **The plugin JVM test gate does not cover `:app`.** `just test-plugin`'s
  `testPlugins` aggregate fans out across `tauri-plugin-*` subprojects only. The
  worker has no JVM tests today (its R064 regression guard is in Rust), so this
  is latent; if the bootstrap or worker later wants Robolectric coverage, `:app`
  needs a `testDebugUnitTest` target and a decision on whether it joins the gate.

## Effort

~M (human) / ~M (CC). Make the scheduler worker-agnostic (class-name injection)
and rename it `tauri-plugin-background-work`; move the worker, the bootstrap
retrieve, and the JNI symbol into `gen/android/app/src/main/java/xyz/yzx9/gpm/`
and re-root the JNI symbol to the app package; inject the WorkManager/Keystore
deps via `settings.gradle`; re-run the full Android build and confirm a headless
tick still pull-syncs on a device. Larger than R076 because it touches the app's
Android surface and the JNI ownership, not just crate packaging.

## Depends on / Supersedes

- Builds on R061 (periodic background sync) and R064 (sync under App Lock); the
  worker's foreground-skip, retry policy, and pull-only contract come from
  there and stay.
- Resolves the D8 `MasterKeyAccess` duplicate by **co-locating** it with the
  canonical constants in the app's shared headless bootstrap (dedupes the app's
  own consumers: this worker + the future Autofill service). Cross-plugin
  dedupe against the keystore plugin (D8 primary) stays a future option, now
  against a single merged keystore plugin (R076).
- Sets the pattern R056 (Android Autofill) inherits: an OS-started service
  reaches initialized state through this same app-owned bootstrap, not a
  plugin. R056 still carries its own prototype gate for the part the worker
  does not exercise — biometric identity unlock from a cold service process.
- Serves `docs/specs/005-git-storage/`; preserves the R064 threat-model
  residual (git-credential residency under App Lock is unchanged).
