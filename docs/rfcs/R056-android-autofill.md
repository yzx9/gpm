# Android autofill (Autofill Framework)

**Priority:** P1
**Status:** Accepted
**Phase:** Now
**Revision:** 2

## What

Add an Android autofill service so that, when a login field is focused in any
app, the OS offers to fill credentials from gpm — dropping the account and
password straight into the target field without touching the clipboard. The
service is an app-owned Android component living in the app's own source set
(`gen/android/app/src/main/java/xyz/yzx9/gpm/`), not a plugin — following the
pattern R077 establishes for the background-sync worker, where an OS-started
service reaches initialized state through an app-owned headless bootstrap rather
than a plugin crate. This RFC records the decision to scope to the Android
Autofill Framework and to defer Accessibility-based and Credential-Manager-based
variants; the functional requirements, the matching mechanisms, and the threat
model live in `docs/specs/008-android-autofill`.

Two things distinguish this from a generic autofill service and shape the rest
of the RFC. First, matching leans on a plaintext signal gpm already has — the
entry path — so a web login can be offered before any unlock. Second, the
load-bearing unknown is neither the matching nor the fill but reaching the store
and biometric unlock from an OS-invoked service whose process the OS may have
just cold-started.

## Why

The motivation (copy-paste is the standing mobile friction, and the secret
transits the clipboard) is in the PRD. The RFC-level question is why the feature
was excluded at launch and why that no longer holds: A001 deferred autofill on
the grounds that the store, the at-rest sealing, the app-lock biometric gate, and
the local-plugin pattern all needed to exist first. They do now, so this RFC
re-evaluates that exclusion and records the shape the service takes — and the
build is now committed as the minimal MVP below.

## Context

**Approach.** Register an Autofill Framework service as an app-owned component
in the app's Android source set; its `<service>` is declared directly in the app
manifest (`gen/android/app/src/main/AndroidManifest.xml`), so no plugin manifest
merge is needed. The user enables the service in the system autofill settings —
it cannot self-enable — after which the OS routes focused-login-field events to
it system-wide. Detection is push-based: the OS hands the service a snapshot of
the focused screen's view tree, and the service never scans on its own.

**The load-bearing unknown is process reachability, not matching.** The store,
the at-rest master key, and the biometric unlock are all initialized when the
app's main activity starts. An autofill service, though, is invoked out-of-band by
the OS while the user is in a different app, and on low memory the OS will have
killed gpm's process entirely; spinning the service up does not start the main
activity, so none of that state exists when a fill request arrives. R077's
background-sync worker already proves half of this — a cold, OS-started
WorkManager process reaches an initialized `Store` and reads the auth-free master
key through an app-owned headless bootstrap (load `libgpm_lib.so`, resolve the
config dir, retrieve the key, construct the `Store`). The Autofill service
reuses that same bootstrap, so basic store reachability is no longer the open
question. What remains open — and is still prototype-gated — is the half the
worker never exercises: reaching the **biometric identity unlock** (the vault
key) from a cold service process, since the worker is pull-only and reads only
the auth-free master key. It is resolved by the first milestone of the committed
build: prove that a fill returned from a service-launched surface actually lands
in the target fields after a cold start, with the vault key unsealed by a
BiometricPrompt hosted on that surface. If it cannot
reach the unlocked identity, the design pivots — toward a service-side bootstrap
that does not depend on the main activity (the deferred identity-agent, R042), or
a native picker — before the rest is invested in. The field-id plumbing the OS
provides (the view-tree snapshot handed back to a service-launched surface) is,
by contrast, a solved platform pattern, not the risk.

**Why the path is the match key.** gpm keeps secret bodies encrypted and unindexed
by design, so it has no plaintext metadata index to match against the way
Bitwarden or 1Password do. The one piece of plaintext structure it does have is
the entry path, and the website template writes the domain into it
(`websites/<domain>/<user>`), which is browseable without decryption. That turns
the cold-start lookup into a filename walk that can offer a web candidate before
any unlock — something index-dependent providers cannot do. The matching
mechanisms themselves (path-primary for web, a learned encrypted map for native
apps, body-`url:` mining, Public-Suffix-List-aware domains, a pre-unlock picker
for multiple matches) are specified in the PRD; this RFC only records why the
path is the key rather than a built index.

## The minimal MVP (committed scope)

The first build is not the throwaway skeleton the original effort sketch
described — it is the MVP pulled forward, cut to the minimum a user can actually
fill a password with, and it ships straight to release (the service is inert
until the user enables it in the system autofill settings; there is no in-app
entry point). The prototype gate survives as the first hard-stop milestone
inside the build; if it fails, the design pivots as above.

- **Native fill surface, not the reused WebView UI.** The PRD sketched reusing
  001's unlock/search UI ("no new search screen is built"). That does not
  survive contact with the platform: a second Tauri-hosted activity is not
  viable (Tauri's `PluginManager` is a process-wide singleton bound to one
  activity, and the mobile runtime bootstraps a single WebView), and relaunching
  the singleTask launcher `MainActivity` in a fill mode leaves no clean way to
  hand the fill result back to the OS. The fill surface is therefore a dedicated
  plain (non-Tauri) activity in the app source set. This supersedes the PRD's
  reuse decision; the deviation and the follow-up plan are recorded in a
  follow-up RFC once the MVP ships — the PRD itself is not amended.
- **Unlock once per process, gating the list.** The first entry into a cold
  fill surface (App Lock on) runs the STRONG BiometricPrompt over the real
  vault key BEFORE the entry list loads — entry names are store metadata and
  sit behind the same wall as the in-app list; the key is then cached for the
  process lifetime, and later fills in the same process skip the prompt. This
  is a deliberate relaxation of the
  immediate-wipe per-fill default the threat model describes — the accepted v0
  trade-off, with per-fill re-lock / idle TTL deferred to the security-balance
  pass. The cache also does not wipe on an explicit in-app re-lock in the MVP
  (there is no lock signal reaching the fill surface); closing that gap is the
  top candidate of the follow-up RFC.
- **No matching.** Every fill goes through the full entry list — unsorted
  paths, plus a substring type-to-filter. Sorting, recents, path/domain
  matching, and the learned app→entry map all remain future phases.
- **Fill values follow gopass semantics:** password = the secret's first line;
  username = the body `login:` field (then `username:`), falling back to the
  entry path's last segment; a password-only field gets only the password.
- **The service callback stays trivial.** `onFillRequest` does a hint scan and
  returns one auth-required dataset — zero store access, zero precondition
  checks — so it can never block or crash the OS's fill dispatch. Precondition
  failures (store not set up; App Lock on with no enrolled biometrics; a
  passphrase-locked identity without auto-unlock) surface as empty states in
  the activity. App Lock off is NOT a failure mode: the identity is
  master-sealed there, and fills work without any prompt.
- **Platform floor.** The Autofill Framework is API 26+ while the app's minSdk
  is 24; the service code is guarded accordingly and simply inert on 24/25 (the
  OS never binds it there).
- **Verification.** The provider cannot fill its own package, so the repo
  carries a minimal standalone target app (`tools/autofill-target`, two
  hint-declared fields; its README documents the build) as the deterministic
  device-smoke target. Automated coverage follows the R077 pattern: host tests
  for the headless Rust cores, JVM tests for the Kotlin logic (hint-scan
  classification, fill-value mapping, JSON contract round-trip, unlock cache).

## Alternatives considered

1. **Accessibility-based autofill.** More capable — it reads the full view tree
   (not just the on-demand snapshot), works on apps that declare no autofill
   hints, and can fill fields the framework cannot — and the Play accessibility
   policy that constrains it is moot for a sideloaded or F-Droid app. Rejected as
   the primary path: it grants the service visibility into all apps' full screen
   content (a far larger read surface than the framework's on-demand snapshot),
   its fill path is more fragile (direct text-set with clipboard and gesture
   fallbacks), and it needs a custom overlay UI — more code and more trust
   surface for a case the framework already covers. Held in reserve as the
   opt-in fallback for the hint-poor apps the MVP framework path does not cover.

2. **Credential Manager / passkeys.** The modern platform credential layer.
   Rejected as the starting point: it is built on top of an autofill service, so
   plain autofill is still the foundation, and passkey storage and rostering is a
   larger scope than password autofill. Land autofill first; Credential Manager
   is a natural later layer over the same service.

3. **An IME / keyboard add-on.** Rejected: the heaviest maintenance burden, it
   requires the user to adopt gpm as their default keyboard, and it offers
   nothing over the framework path for a sideloaded app.

4. **Copy-and-paste only (do nothing).** Rejected as the long-term answer, though
   it is the acceptable first cut the project is at today: copy is the standing
   mobile friction and it places the secret on the clipboard, both of which
   autofill removes.

## Effort

Medium. The committed scope is the minimal MVP above, with the cold-process
biometric reachability as the first hard-stop milestone inside the build rather
than a separate pre-commitment prototype. It leans almost entirely on existing
unlock, store, and R077-bootstrap plumbing, so the earlier ~1-2 week (human) MVP
estimate shrinks by the matching and search-quality work it now excludes; the
native fill activity and the two headless Rust entry points (list, decrypt) are
the main new surface. The learned association index (encrypted at rest), the
path/body matching, and the inline phase remain future (~3-5 days plus a phase
when taken). The cold-process reachability still dominates the risk.

## Depends on / Supersedes

Re-evaluates the autofill exclusion in A001 (launch scope). Builds on R077's
app-owned headless bootstrap: the Autofill service reaches an initialized `Store`
through the same `gen/android/app/` module the background-sync worker uses, so
store reachability is inherited rather than re-proven (only the biometric
identity unlock from a cold service process is still prototype-gated). Relates to
`0042-identity-agent`: an autofill service is the clearest second consumer of
unlocked-identity state outside the main activity — the pressure that RFC
anticipates and defers (the MVP's process-lifetime identity cache is the first,
smallest step in that direction) — and if the milestone shows the service cannot reach the
unlocked identity from a cold process, it forces the agent extraction 0042 parks.
Builds on the at-rest AEAD sealing and app-launch biometric gate (their own RFCs
shipped and were removed); the "new local plugin" framing an earlier draft of
this RFC used is superseded by the R077 app-owned-source-set pattern.
