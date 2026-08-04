# Collapse the display-prefs file back into the sealed app config

**Priority:** P2
**Status:** Accepted
**Phase:** Next

## What

gpm split its application config into two on-disk files (R060): a small plaintext
file for the handful of display preferences that must render before the at-rest key is
available, and a sealed file for the behavior preferences read after unlock. This RFC
collapses them back into a single sealed app config file, so the config tier holds zero
plaintext. It is feasible because R064 made the at-rest master key auth-free — always
retrievable without a biometric — which removed the pre-unlock-readability premise that
forced the plaintext split in the first place.

Serves the app-config storage tier (A003); its notable interaction is with
`docs/specs/007-app-lock` (cold-start unlock timing).

## Why

R060's split exists for one reason: the display language (and a few siblings) had to be
readable before the master key was injected, so they could not be sealed. R064 then split
the at-rest key and made the metadata key auth-free — permanently retrievable by the app
process without a prompt. Once the auth-free key is available at launch, sealed config is
readable the moment the app process is up, and the "must render pre-unlock" constraint
dissolves. The plaintext display file is now a vestige: it is the only plaintext config
file, its contents are non-confidential, and it exists solely to satisfy a requirement
that no longer holds. Collapsing it restores a single uniform storage rule — seal
everything at rest — and shrinks the plaintext surface to zero at the config tier.

## Context

**The pre-unlock-readability premise, retired.** The forcing case was first-paint
rendering: the display language drives the very first frame and the app-lock biometric
screen, both of which can appear before the identity is decrypted. With the master key
auth-free, the app can unseal the config within a frame of the process starting. The one
path that genuinely runs before the process exists — the WebView initialization that bakes
a best-effort locale — already uses the system locale and reconciles a pinned preference
one frame after mount. So a pinned locale in the sealed config is readable one frame after
launch; the only window that cannot read it is the pre-process init, which already
degrades to the system locale. No first-paint regression.

**The App-Lock cold-start wrinkle.** At a cold start with the app-launch gate on, the
foreground deliberately defers loading the auth-free master key until the unlock biometric
(it keeps the lock overlay meaningful). So the sealed config — locale, theme, the
verbose-logging deadline, the schema version, the background-sync cadence — is not
readable until that first unlock. During the brief locked window the app renders with the
system locale and defaults, then re-applies the pinned values within a frame of unlock.
This is acceptable: the lock screen itself uses the system locale and is secure-by-default
regardless, and the window lasts only until the first biometric. The display values are
non-confidential, so deferring them changes UX slightly, not security.

**The migration schema-version gate.** Today the one-shot config migrations decide what
to run by peeking the schema version out of the plaintext file. Once that version lives
inside the sealed file, the gate cannot read it until the auth-free key is loaded. The
engine must then tell two states apart: "no config yet" (a fresh install — skip the chain)
and "sealed config exists but the key is deferred" (an App-Lock cold start — defer the
chain until the first unlock, then resume). The app-lock-deferred migration path already
exists; this extends it to the schema-version peek. Getting this distinction right is the
one piece that needs care — misclassifying a locked-but-present config as "no config"
would skip migrations that should run.

**Threat-model impact — strictly smaller.** The config tier's plaintext surface goes to
zero. No new exposure is introduced: the display preferences are non-confidential today
and stay so; the win is uniformity (one storage rule instead of two) and the elimination
of the last plaintext config file. The local-write-attacker rollback risk on the display
file — accepted today because the file had to be plaintext — goes away too (sealed files
still roll back, but the display values are non-security-relevant either way). Desktop is
unaffected: with no Keystore the seal is a plaintext passthrough, so "merging" there
simply combines two plaintext files into one.

## Alternatives considered

- **Keep the split (status quo).** Rejected: R064 removed the premise the split was built
  on. The split now carries a plaintext config file the threat model would rather not
  have, for no remaining benefit.

- **Keep the display file but seal it under its own auth-free key.** Rejected: it
  duplicates the seal machinery (the auth-free master key already seals app config) and
  preserves the two-file split for no reason. One sealed config file is simpler.

- **Seal only the non-locale display values; keep locale plaintext.** Rejected: it
  preserves a smaller vestige of the same dissolved constraint, and locale is readable
  post-launch like the rest — there is no need to special-case it once the key is
  auth-free.

## Effort

~M (human) / ~M (CC). A forward one-shot migration that merges the two files into one
sealed config and deletes the plaintext one; re-ordering early startup so the sealed
config loads immediately after the auth-free key is available; extending the
schema-version migration gate to defer (rather than skip) when the sealed config is
present but the key is not yet loaded; re-applying locale, theme, the verbose deadline,
and the background-sync cadence after the first unlock (the unlock path already re-seeds
part of this); and retiring the display/behavior type split, or keeping it as a logical
split serialized into one file.

## Depends on / Supersedes

Supersedes the shipped app-config split (R060) — reopens and reverses it. Builds on R064's
auth-free master key, without which sealed config would not be readable pre-identity-unlock
and the collapse would not be safe. When this ships, A003's "plaintext tier = the display
file" consequence is updated to "no plaintext config file remains."
