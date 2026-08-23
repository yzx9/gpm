# iOS as a second mobile target (parity minus autofill and screen-capture protection)

**Priority:** P2
**Status:** Draft
**Phase:** Future
**Revision:** 1

## What

Bring gpm to iOS as a second mobile target on the same Tauri v2 + Rust +
Vue core, at near-parity with Android: list / decrypt / copy, create /
edit, foreground git sync, age- and GPG/OpenPGP-store support,
encryption-at-rest, and biometric (identity + app-lock) unlock. Three
Android capabilities are out of scope or deferred: autofill (a separate
iOS Credential Provider Extension, sibling to R056), screenshot /
screen-recording protection (no iOS equivalent to Android's secure-window
flag — see Residual risks), and — sequenced last — background autosync
(iOS's background-task model is best-effort, not a reliable scheduler; see
Alternatives). The deliverable this RFC scopes is an iOS-Simulator-runnable
build; real-device validation is a stated gap, not a claim.

Serves the cluster of mobile-relevant feature specs — 001 entry-access,
005 git-storage, 006 identities, 007 app-lock — by extending them to a
second platform. There is deliberately no single `NNN-ios` spec: this is a
platform port of already-specced features, not a new product capability,
and this RFC is the how for "those features run on iOS." Builds on the
foundational ADRs A001 (the Tauri / Rust / Vue / age-only stack that makes
the core portable), A002 (rust-first-without-gopass, so no `gpg` binary to
port), A003 (configuration-storage tiering, which this extends with a
third tier), and A006 (pure-Rust OpenPGP, already iOS-clean).

## Why

Today gpm is Android + desktop; iOS users (Casey, the mobile-first
newcomer, and any self-hoster who carries an iPhone) have no path. The
architectural bets the project already made — a platform-free core that
only ever sees a key as plain bytes, pure-Rust crypto (age, rpgp,
ssh-key) with no C or subprocess dependency in the runtime path, and
vendored libgit2 with credentials carried in-memory — were not made with
iOS in mind but turn out to be exactly what an iOS port needs. So the
question is not "can the core run on iOS" (it can, essentially for free)
but "how much platform plumbing and Swift plugin work stands between that
core and a runnable app." This RFC records that the work is concentrated
and bounded rather than scattered, and names the two real cliffs.

## Context

**What ports for free.** The encryption envelope (AES-256-GCM, master key
injected as bytes), age encryption/decryption, GPG/OpenPGP via rpgp
(in-process, no `gpg` / `gpg-agent`), and git sync via vendored libgit2
with credentials supplied in-memory (no `~/.ssh` or credential-file
assumptions) all cross-compile and run on iOS with no architecture change.
age-plugin recipients (e.g. Yubikey) already fail cleanly when the plugin
binary cannot run — the same path Android takes. This is the dividend of
A001 / A002 / A006.

**The platform split today is two-way, not three-way.** Every platform
gate in the codebase is Android-vs-everything-else; iOS today inherits the
"everything-else" (desktop) branch everywhere, which means plaintext config
and inert plugin stubs. The at-rest encryption seam is deliberately clean:
the envelope takes a master key as plain bytes and is platform-free, and a
small app-layer trait abstracts the keystore so the whole logic is unit-
testable with a mock. Adding iOS means widening the platform gates from
Android-only to mobile (Android + iOS) where the real plugin handle should
be used, and leaving the desktop passthrough as-is — no new abstraction,
just a third consumer of an existing one.

**The work is the plugins.** Each of the eight local plugins is a
hand-written Android (Kotlin) module with no iOS counterpart; Tauri's
mobile-plugin layout expects an iOS (Swift) module alongside. The port is
one Swift implementation per plugin, behind the same OS-agnostic Rust
contract (commands, events, base64 byte payloads, and the mirror-pinned
error-code set). The plugins split into three groups by difficulty:

- _Trivial-to-moderate, mechanical_ — safe-area, file-picker, file-save
  (also fixes a latent bug: the current desktop dialog branch would be
  selected on iOS and fail at runtime, because that dialog library has no
  iOS backend), device-info.
- _The security core_ — the keystore plugin: Keychain-backed seal,
  LocalAuthentication (Face ID / Touch ID) for the two biometric flows
  (identity passphrase unlock, and app-lock vault-key unlock). The
  plugin's policy type maps onto Keychain accessibility and access-control
  flags. The non-exportable-key constraint that forces Android to do the
  AES/GCM in Kotlin applies equally on iOS (Secure Enclave), so the Swift
  side owns the crypto, as Kotlin does.
- _The capability periphery_ — clipboard-notify (an Android-flavored
  sticky-notification-and-tap-to-clear UX, ported via iOS local
  notifications) and background-work (the BGTaskScheduler question — see
  Alternatives).

**Two structural gaps that are platform, not effort.** (1) Android's
secure-window flag prevents screenshots and screen recording at the
surface level; iOS has no equivalent — an app can only _detect_ screen
recording and react (e.g. blur), not prevent capture. True parity is
impossible here; deferring it means sensitive iOS screens are capturable
(Residual risks). (2) Android's WorkManager gives a reliable,
reboot-persistent, constraint-respecting periodic scheduler; iOS's
BackgroundTasks framework is best-effort, OS-scheduled,
cadence-not-guaranteed, and fires far less for infrequently-used apps.

**Build / config is greenfield.** `tauri ios init` has never been run;
there is no iOS bundle config, no iOS capabilities row, no iOS CI, no iOS
build recipes. Tauri v2's iOS support is stable, but practitioners report
it as rougher than Android (Xcode steps mediated through Tauri, an
immature plugin story, lagging docs) — a friction tax to absorb, not a
blocker. The mobile entry point already applies to iOS through Tauri's
`mobile` alias, so the app's run wiring needs no change.

## Alternatives considered

- **Minimal scope (drop encryption-at-rest + biometric on iOS, ship a
  list / decrypt / copy shell).** Rejected as the resting position:
  without at-rest protection and biometric unlock it is not a usable
  password manager — the security model is the product. The minimal shell
  is retained only as a milestone (Phase 1 below), not as the goal.
- **Background autosync: drop entirely vs. implement via BGTaskScheduler.**
  Dropping is simplest and safest under a Simulator-only constraint.
  BGTaskScheduler _can_ host a periodic, network-gated sync, but: cadence
  becomes OS-decided best-effort (the existing interval parameter degrades
  to a floor hint, not a schedule); it reintroduces the whole headless
  architecture — an OS-started process with no app handle that must reach
  the store and the auth-free master key _directly_ (the same
  headless-bootstrap problem R077 resolves on Android, now in Swift: a
  Swift entry point into the Rust static lib, and a direct-Keychain
  master-key retrieve alongside the canonical alias constants); and it is
  the single feature least validatable on the Simulator (BGTaskScheduler
  does not fire naturally there, only via a debug simulate-launch), so a
  headless path that exercises divergence-resolve, the cross-process repo
  lock, and the deferred identity-wipe would ship essentially untested.
  Decision: implement it (parity is the goal), but sequence it last and
  treat real-device validation as a prerequisite to trusting it.
- **screen-secure: a degraded "detect-and-blur" port vs. defer entirely.**
  Detect-and-blur is possible but meaningfully weaker than Android (no
  prevention, only reaction, and only for screen _recording_, not
  screenshots). Decision: defer for now and record the capturable-screens
  regression as an accepted residual, rather than ship a half-feature that
  implies parity it cannot deliver.
- **clipboard-notify: port via local notifications vs. drop.** Porting
  preserves parity; the foreground clear timer (pure Rust, platform-free)
  works on iOS regardless. Decision: port, accepting iOS
  notification-permission friction and that the Simulator's notification
  surface is limited — the weakest fit of the eight plugins, kept for
  parity rather than UX fit.
- **A new `NNN-ios` feature spec.** Rejected: iOS is a platform port of
  already-specced features, not a new capability. This RFC serves the
  existing mobile-relevant specs collectively; a redundant spec would
  re-state them.

## Residual risks (what we accept)

- **No iPhone → the security path is built but not hardware-validated.**
  The user developing this has macOS but no iOS device. The iOS Simulator
  runs Keychain and a _simulated_ biometric but has no Secure Enclave, so
  the hardware-backed-key path and the biometric-gated Keychain items run
  there without proving real-device behavior. The keystore and
  background-sync work — the two hardest buckets — are exactly the two the
  Simulator validates least. Acceptable for a Draft / Future scoping; a
  real device must validate before any "shipped" claim.
- **Capturable sensitive screens.** Deferring screen-capture protection
  means iOS has no equivalent of Android's screenshot / recording block.
  A real, if minor, security regression vs. Android; accepted by decision.
- **Biometric grace window.** Android supports a short auth-validity
  window (re-use a successful biometric for N seconds without re-prompting).
  iOS has only a coarser, per-context reuse-duration analog with different
  semantics; this maps best-effort and is recorded as a known UX downgrade.
- **Tauri v2 iOS maturity.** The Xcode-through-Tauri build pipeline and the
  iOS plugin authoring story are rougher than Android. Treated as an
  unknown-unknowns friction tax on the build/config bucket, not a designed
  risk.
- **Headless-sync correctness untested.** Per the BGTaskScheduler decision
  above, background autosync ships Simulator-blind against the most
  stateful code paths (divergence resolve, cross-process repo lock,
  deferred identity wipe). Real-device validation is the mitigation,
  sequenced as a prerequisite.

## Effort

~L (human) / ~L (CC). The core (crypto, age, rpgp, git2) ports for free;
the work is concentrated in (a) standing up the iOS build / config from
scratch and absorbing the Tauri-iOS toolchain tax, (b) the keystore plugin
Swift port (Keychain + LocalAuthentication, both biometric flows) — the
single largest chunk, (c) the remaining plugin Swift ports (safe-area,
file-picker, file-save incl. the dialog-branch fix, device-info,
clipboard-notify), and (d) the background-sync headless architecture in
Swift (the BGTaskScheduler port, the Swift → Rust entry point, and the iOS
headless bootstrap). Ordered so that something is always runnable and the
riskiest, least-Simulator-validatable work comes last:

1. _Light up the Simulator, no security layer_ — run `tauri ios init`,
   widen the workspace to compile for the iOS Simulator target, add the
   iOS capabilities row and build recipes, port safe-area, fix the
   dialog branch, and get list / decrypt / copy / foreground sync working
   on plaintext config (reusing the existing desktop branch). This retires
   the biggest unknown: does it stand up at all.
2. _Security core_ — the keystore plugin Swift port, then the at-rest tier
   wiring (widen the platform gates), then the two biometric flows. The
   largest and most review-sensitive chunk.
3. _Periphery_ — clipboard-notify (local notifications), device-info.
4. _Background sync_ — the BGTaskScheduler port and the iOS headless
   architecture, ideally once a real device is available.

## Depends on / Supersedes

- Builds on A001, A002, A003, A006 — the stack decisions whose dividends
  (platform-free core, pure-Rust crypto, clean at-rest seam) make the port
  tractable.
- Extends the feature specs 001 (entry-access), 005 (git-storage),
  006 (identities), 007 (app-lock) to a second platform.
- Inherits the headless-bootstrap pattern from R077 (the worker-agnostic
  scheduler and the app-owned, OS-started-process-with-no-app-handle
  bootstrap that reaches the store and the auth-free key directly); the
  iOS background worker is the Swift-side instance of that pattern.
- Defers the iOS sibling of R056 (Android Autofill): an iOS Credential
  Provider Extension is a separate process with its own entitlement and
  Keychain-sharing needs, out of scope here.
