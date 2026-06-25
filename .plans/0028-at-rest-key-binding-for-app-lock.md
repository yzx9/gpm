# At-Rest Key Binding for the App-Launch Biometric Lock

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Extends 0022's app-launch biometric gate with a genuine at-rest guarantee: when
the opt-in gate is enabled, the at-rest master key is re-sealed behind a
biometric-gated Keystore key (not the auth-free one), so the whole store is
unreadable — on disk and in memory — until the user authenticates on launch or
resume. Adds a second, independent, opt-in toggle so that a successful app-unlock
also unlocks the identity session (no separate identity prompt), defaulting off.

## Why

0022 deliberately kept the master key auth-free so a fingerprint change could
never brick the store, making the gate a UI-layer challenge only. That is safe
but weaker than what users expect from a password-manager "app lock": they expect
that without their biometric, the entries are not merely hidden but
inaccessible — not just masked by an overlay while the process still holds the
decrypt key. Binding the at-rest key (opt-in) delivers that expectation, and the
bricking risk that made 0022 reject it is mitigated by a different Keystore key
policy that survives fingerprint-enrollment changes.

The identity-auto-unlock toggle exists because, once the master key is the single
biometric gate, re-prompting the user again for the identity on the very next
copy/show is redundant friction. It must stay optional and separate from the
existing auto-lock timing presets, because some users want the app gate and the
per-operation identity auth to remain independent.

## Context

gpm now has **three** Android Keystore mechanisms with deliberately different key
policies:

- The **auth-free master key** (default) seals `repo.json`/`identity` and unseals
  silently at startup. Survives any biometric change. Never bricks. This stays
  the default for users who do not enable the app lock.
- The **biometric-gated identity passphrase** (existing "biometric unlock") seals
  the identity passphrase behind a prompt, invalidated by enrollment, self-healing
  via passphrase re-entry. Unchanged.
- The **biometric-gated master key** (new) is the same AES/GCM mechanism as the
  auth-free master key, but the key is `setUserAuthenticationRequired` and **not**
  `setInvalidatedByBiometricEnrollment`. It gates the whole store behind one
  biometric prompt. Adding a fingerprint does not invalidate it; removing all
  biometrics does (documented re-setup).

The **app-lock toggle is the master key's location.** Enabling migrates the master
key blob from the auth-free store to the biometric-gated store; disabling
migrates it back. The existence of a sealed biometric-gated master key — probeable
non-promptingly, exactly like the existing passphrase liveness probe — _is_ the
"app lock is on" signal. This sidesteps the chicken-and-egg of needing to read the
(config-encrypted) toggle to decide whether to prompt: the key's location is read
without any prompt and without touching `repo.json`.

**One biometric prompt per foreground.** A `BiometricPrompt` carries a single
`CryptoObject`, so one prompt unseals exactly one key. The app-unlock prompt
unseals the master key (which gates `repo.json` and therefore the entire store).
For the identity-auto-unlock toggle, the identity passphrase is sealed by the
master key itself via a new at-rest AEAD slot, so once the master key is unsealed
the passphrase decrypts with _no second prompt_. When the toggle is off, identity
auth keeps its existing per-operation/session behavior over the unchanged
identity overlay.

**The master key becomes lazily injectable.** At launch, if the app lock is on,
the store is constructed with the master key absent; `repo.json` cannot be read
until the app-unlock prompt retrieves and injects it. On app-lock (the process
going to background and returning to the foreground), the master key is wiped
from memory alongside the identity cache, so a locked app cannot read the store
even from a memory snapshot. This is the property that makes the gate a real
cryptographic lock rather than a UI mask. It coexists with the existing identity
cache lock: the identity lock wipes only the identity cache and keeps the master
key; the app lock wipes both.

**Resume semantics.** The gate re-challenges on every return to the foreground —
cold start and warm resume alike (0022's model). A background-duration threshold
is deliberately not introduced: it adds a config knob and a timing race for no
security gain over "every resume."

**Threat model.** Consistent with the existing one (local opportunistic access).
For opt-in users the at-rest defense strengthens from "a read attacker who extracts
the auth-free key can decrypt the files" to "decryption requires the user's
biometric." A process-running attacker remains an explicit non-goal. The toggle
and the auto-unlock flag live inside `repo.json` under the master key; only the
non-prompting probe of which master-key store holds a key is readable without
auth, and it reveals only "is the app lock on," never content.

## Alternatives considered

- **0022's auth-free master key, UI-only gate.** Rejected for opt-in users: it
  does not meet the "store inaccessible without biometric" expectation, since the
  key stays in memory and the overlay only masks. Retained as the default for
  users who do not enable the app lock, preserving the no-brick guarantee there.
- **`setInvalidatedByBiometricEnrollment(true)` for the master key.** Rejected: a
  fingerprint change would brick the entire store (git credentials, trust set,
  identity) with no self-heal — the master key cannot be re-derived the way a
  passphrase can be re-entered. Using `(false)` accepts the smaller residual risk
  (all biometrics removed ⇒ re-setup) in exchange for not bricking on the common
  case of enrolling a new finger.
- **Two biometric prompts (master key + passphrase).** Rejected: one
  `CryptoObject` per prompt makes this strictly worse UX with no gain; sealing the
  passphrase under the master key collapses both into one prompt.
- **Background-duration threshold before re-challenge.** Rejected: adds a knob and
  a race; "every resume" is simpler and matches the banking-app expectation.

## Effort

~L (human) / ~L (CC): a new biometric-gated master-key plugin capability, lazy
master-key plumbing in `rustpass`, an app-lifecycle → frontend event path, an
app-lock backend module with enable/disable/unlock/lock/migrate commands, a
distinct frontend app-lock overlay + state, a Settings section with two toggles,
and SECURITY/CHANGELOG updates. Several reviewable commits.

## Depends on / Supersedes

Extends and revises 0022 (its "master key stays auth-free" stance now applies
only to the default, non-app-lock case). Builds on the existing biometric-gated
Keystore pattern. Does not supersede the identity-passphrase biometric unlock.
