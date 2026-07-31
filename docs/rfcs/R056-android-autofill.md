# Android autofill (Autofill Framework)

**Priority:** P1
**Status:** Draft
**Phase:** Future

## What

Add an Android autofill service so that, when a login field is focused in any
app, the OS offers to fill credentials from gpm — dropping the account and
password straight into the target field without touching the clipboard. The
service is the OS-facing entry point of a new local plugin (the same shape as
the existing keystore plugins). This RFC records the decision to scope to the
Android Autofill Framework and to defer Accessibility-based and
Credential-Manager-based variants; the functional requirements, the matching
mechanisms, and the threat model live in `docs/specs/008-android-autofill`.

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
re-evaluates that exclusion and records the shape a service would take — without
yet committing to build it.

## Context

**Approach.** Register an Autofill Framework service as a new local plugin; the
plugin's manifest is merged into the app manifest, so no app-manifest edits are
needed (the clipboard-notify plugin already contributes a manifest component the
same way). The user enables the service in the system autofill settings — it
cannot self-enable — after which the OS routes focused-login-field events to it
system-wide. Detection is push-based: the OS hands the service a snapshot of the
focused screen's view tree, and the service never scans on its own.

**The load-bearing unknown is process reachability, not matching.** The store,
the at-rest master key, and the biometric unlock are all initialized when the
app's main activity starts. An autofill service, though, is invoked out-of-band by
the OS while the user is in a different app, and on low memory the OS will have
killed gpm's process entirely; spinning the service up does not start the main
activity, so none of that state exists when a fill request arrives. There is no
established path from an OS-started service — or a separate fill surface — to
that initialized state. This is the single assumption the whole "reuse the
existing store and unlock" thesis rests on. It is resolved by a minimal prototype
before any matching or UI is built: prove that a fill returned from a
service-launched surface actually lands in the target fields after a cold start.
If it cannot reach initialized state, the design pivots — toward a service-side
bootstrap that does not depend on the main activity (the deferred identity-agent,
R042), or a native picker — before the rest is invested in. The field-id plumbing
the OS provides (the view-tree snapshot handed back to a service-launched surface)
is, by contrast, a solved platform pattern, not the risk.

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

Medium-large, and prototype-gated. The first step is a minimal prototype that
proves a fill returned from a service-launched surface lands in the target fields
after a cold start — the go/no-go for the whole feature, and small (a new plugin
shell and a hard-coded fill, no store, no biometric, no search). Past that gate,
the MVP — the service, the auth-required suggestion that launches the fill-mode
search-and-pick, and the fill-back into the target fields — is ~1-2 weeks
(human), since it leans almost entirely on existing unlock, search, and store
plumbing and adds one new plugin. The learned association index (encrypted at
rest), the path/body matching, and the inline phase add ~3-5 days plus a phase.
The cold-process reachability question dominates the risk; the human cost
dominates the effort. ~30 min (CC) to scaffold the plugin shell and merged
manifest.

## Depends on / Supersedes

Re-evaluates the autofill exclusion in A001 (launch scope). Relates to
`0042-identity-agent`: an autofill service is the clearest second consumer of
unlocked-identity state outside the main activity — the pressure that RFC
anticipates and defers — and if the prototype shows the service cannot reach
initialized state any other way, it forces the agent extraction 0042 parks.
Builds on the local-plugin pattern the keystore plugins already establish and on
the at-rest AEAD sealing and app-launch biometric gate (their own RFCs shipped
and were removed).
