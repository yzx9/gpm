# Merge the two Keystore plugins into one generic seal

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Collapse `tauri-plugin-biometric-keystore` and `tauri-plugin-secure-keystore`
into a single generic Android Keystore seal plugin — caller-supplied alias,
prefs, key policy, and prompt text, auth-free or biometric-gated per the
policy. The two crates are already homomorphic (identical handle surface and
shared type/pure-fn shapes) after the pure-primitive refactor; this RFC makes
the merge that the homomorphism was built to enable, deleting one crate and
retargeting the app at the survivor.

Serves `docs/specs/007-app-lock/` (the App Lock gate) and the biometric-unlock
/ at-rest-seal plumbing under it. This is an internal-architecture refactor of
already-shipped internals, not a new product feature; no new spec is required.

## Why

The two crates hold **byte-identical copies** of a large surface with no
cross-crate guard:

- Rust: `KeyPolicy`, `PromptText`, `ResolvedPromptText`, `AliasState`,
  `BiometricState`, and the `resolve_prompt_text` pure function.
- Kotlin: `mapErrorCode`, `mapBiometricState`, `safeName`, the key-generation
  spec builder, the cipher init helpers, `readCipherData`/`storeCipherData`,
  `promptInfo`, and the `@InvokeArg` shapes.

The within-crate characterization tests (the D4 drift-catchers) pin each
plugin's _own_ copy; they do not detect divergence between the two. A one-sided
change — adding a `KeyPolicy` field, retuning a `map_*` bucket, tightening the
cipher spec — drifts silently and the suites stay green. The pre-landing review
of the pure-primitive refactor flagged exactly this: the merge is the
mechanical dedup the homomorphism exists for, and leaving the crates separate
forfeits it.

Publishability is the secondary motive: two near-identical crates published
separately is noise; one generic "Android Keystore seal under a caller-chosen
policy" primitive is the clean artifact.

## Context

**Current state.** Both plugins are backend-only and fully caller-parametrized:
the alias, prefs, policy, and prompt text arrive as parameters, and the plugin
crate carries no gpm identifiers or brand strings. `KeyPolicy` already spans
both regimes — `auth_required` selects auth-free (direct seal, no prompt,
survives biometric changes) versus biometric-gated (per-use STRONG
`BiometricPrompt`, optionally enrollment-invalidating). The app layer already
centralizes gpm's aliases, policies, and slots, and calls the two plugins'
_identical_ handle methods; it is the single point that would retarget.

**The merge is mechanical precisely because the refactor did the hard part
already.** The remaining divergences to reconcile at merge time are naming
only:

- Biometric availability is exposed under two names (one crate's availability
  probe returns the quad-state; the other's is the same quad-state under a
  different name). Pick one.
- The Security-settings deep-link (the enrollment surface offered when no
  STRONG biometric is enrolled) lives in one crate. Keep it — it is useful for
  any biometric-gated alias.
- A bool "is the Keystore present" probe was dropped from one crate in the
  refactor (the Android Keystore is always present). Do not restore it.

**Migration: none, at the key level.** The merge is a crate / re-export
reshuffle. The on-disk Keystore aliases, the SharedPreferences files, the
AES/GCM ciphertext format, and every key-generation policy are byte-identical
before and after — existing sealed keys decrypt unchanged, no re-seal, no
migration registry entry. The app's import paths move; the Keystore entries do
not. This is the same "no migration" property the conditional keygen
preserved during the refactor.

**Threat-model impact: none.** Same keys, same policies, same crypto, same
aliases. The merge changes packaging, not the security boundary.

## Alternatives considered

- **Status quo — two homomorphic crates.** Rejected: the duplication is
  unguarded across crates, the maintenance is double, and the homomorphism was
  built specifically so this merge would be safe and mechanical. Deferring
  indefinitely leaks drift risk for no benefit.
- **Shared types crate only (Rust side).** A tiny crate both plugins depend on
  would dedupe the Rust types. Rejected as a half-measure: it leaves the
  larger Kotlin duplication (cipher helpers, `map_*`, `@InvokeArg`) untouched,
  and there is no composite-build blocker on the Rust side — but solving only
  the Rust half leaves the harder half in place. The Kotlin side is where most
  of the drift surface lives.
- **Shared Kotlin module across the two plugins.** Rejected on the same grounds
  the D8 dedup was deferred: cross-plugin Gradle dependencies are unproven
  under Tauri's composite build. A full merge sidesteps the question — there is
  only one module.

## Effort

~S–M (human) / ~S (CC). The work is mechanical because the homomorphism is
already in place: pick the surviving crate, fold in the one-crate-only bits
under their merged names, delete the other crate, retarget the app's import
paths, drop one entry from the workspace and the Android Gradle wiring. The
real cost is re-running the full Android build and re-confirming the sealed-key
decrypt path end to end on a device.

## Depends on / Supersedes

- Builds on the pure-primitive refactor that made the two crates homomorphic
  (the caller-parametrized handle, the shared `KeyPolicy`/types, the flattened
  `@InvokeArg` contract).
- Relates to the D8 `MasterKeyAccess` duplicate in the background-sync plugin
  (addressed by R077) — that duplicate is of _one_ plugin's retrieve path, so
  it shrinks from "two crates' worth" to "one" here regardless, and R077
  decides where it ultimately lives.
