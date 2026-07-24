# Android autofill (Autofill Framework)

**Priority:** P1
**Status:** Draft
**Phase:** Future

## What

Add an Android autofill service so that, when a login field is focused in any
app, the OS offers to fill credentials from gpm. The service is the OS-facing
entry point of a new local plugin (the same shape as the existing keystore
plugins); it does not scan the screen — the OS invokes it, hands it a snapshot of
the focused screen's view tree, the service locates the username/password fields,
unlocks through the existing biometric path, and fills, reusing the existing
search-and-pick UI rather than introducing a parallel one. This RFC records the
decision to scope to the Android Autofill Framework and to defer
Accessibility-based and Credential-Manager-based variants.

## Why

Copy-and-paste is the project's standing mobile friction: to use a credential
the user leaves the target app, opens gpm, copies, switches back, and pastes —
and the secret transits the clipboard (mitigated today by clipboard auto-clear,
but on the clipboard nonetheless). For a password client this is the largest gap
against the apps users compare it with, and the one a native autofill service
removes: the credential goes straight into the target field and never reaches the
clipboard, consistent with how the primary copy path already keeps the password
out of the webview.

The feature was deliberately excluded at launch (ADR 0001) on the grounds that
the store, the at-rest sealing, the app-lock biometric gate, and the local-plugin
pattern all needed to exist first. They do now, so this RFC re-evaluates that
exclusion and records the shape a service would take — without yet committing to
build it.

## Context

**Approach.** Register an Autofill Framework service as a new local plugin; the
plugin's manifest is merged into the app manifest, so no app-manifest edits are
needed. The user enables the service in the system autofill settings (it cannot
self-enable), after which the OS routes focused-login-field events to it
system-wide.

**Detection is push-based, not scanning.** The OS invokes the service when a
field gains focus and matching save information is declared, handing it a
snapshot of the current screen's view tree. The service walks that snapshot and
identifies username/password fields by their autofill hints and input type. It
never polls, never reads the screen on its own initiative, and sees only the
focused screen the OS chooses to share.

**Identification.** The target app's package id, and for in-browser fields the
web domain, are first-class fields of that view-tree snapshot — both are the
natural keys for matching entries.

**Unlock stays where it is.** The store is encrypted at rest and the identity is
unlocked per operation, so the service returns an auth-required suggestion that
lazily launches an Activity — the same FragmentActivity that already hosts the
biometric prompt — rather than attempting biometric from a background component.
Decryption and fill happen only after that unlock, so the per-op identity
lifecycle is unchanged; an autofill request is simply another operation that
triggers it.

**Association is the real gap.** Entries carry no app-or-domain field, and secret
bodies are encrypted and never indexed, so there is no key to join on today. Two
complementary mechanisms close it, both operating after unlock: a learned,
encrypted mapping from package id / web domain to entry, built up as the user
picks entries (the cold-start pattern every autofill provider bootstraps with);
and matching against the URL convention the website template already writes into
the body. Search itself is already implemented — a name/path fuzzy match that
needs no identity to browse — so the picker the user lands on is the existing
list; only the chosen entry's fill needs the unlock.

**UI is layered, not rebuilt.** The OS supplies the suggestion container; each
suggestion is a constrained snippet (the platform's remote-views / inline
presentation), which cannot host an interactive search, so the
unlock-search-and-pick flow runs in the Activity the suggestion launches and
reuses the existing webview UI wholesale. No new search surface is built.

**Threat-model impact.** Autofill does not change at-rest encryption or the
per-op identity lifecycle; it adds an OS-registered entry point invocable from
any app. The new surfaces to reason about: the service receives the focused
screen's view-tree snapshot, which may include other visible fields and text, so
a fill request must never persist or log that snapshot; every fill is gated by
the same biometric unlock used elsewhere, with no cached bypass; and the learned
association mapping is a new at-rest artifact that must receive the same
AEAD-at-rest treatment as the config and identity, so a reader learns nothing
about which entries exist. Filling off the clipboard is a net security
improvement over copy.

## Alternatives considered

1. **Accessibility-based autofill.** More capable — it reads the full view tree
   (not just the on-demand snapshot), works on apps that declare no autofill
   hints, and can fill fields the framework cannot — and the Play accessibility
   policy that constrains it is moot for a sideloaded or F-Droid app. Rejected as
   the primary path: it grants the service visibility into all apps' full screen
   content (a far larger read surface than the framework's on-demand snapshot),
   its fill path is more fragile (direct text-set with clipboard and gesture
   fallbacks), and it needs a custom overlay UI — more code and more trust
   surface for a case the framework already covers. Held in reserve as a possible
   opt-in fallback if hint-poor apps prove common in practice.

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

Medium-large, phased. The MVP — the service, the auth-required suggestion that
launches the existing unlock-and-search flow, and the fill-back into the target
fields — is ~1-2 weeks (human), since it leans almost entirely on existing
unlock, search, and store plumbing and adds one new plugin. The learned
association index (encrypted at rest) and the body-URL matching add ~3-5 days.
~30 min (CC) to scaffold the plugin shell and merged manifest; the human cost
dominates.

## Depends on / Supersedes

Re-evaluates the autofill exclusion in ADR 0001 (launch scope). Relates to
`0042-identity-agent`: an autofill service is the clearest second consumer of
unlocked-identity state outside the main activity — the pressure that RFC
anticipates and defers — and building it may force the agent extraction 0042
parks. Builds on the local-plugin pattern the keystore plugins already establish
and on the at-rest AEAD sealing and app-launch biometric gate (their own RFCs
shipped and were removed).
