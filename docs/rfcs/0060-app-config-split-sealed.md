# Seal app-shell behavior preferences at rest

**Priority:** P2
**Status:** Draft
**Phase:** Now

## What

gpm keeps every app-shell preference in one plaintext on-disk file. This splits
that file into a small plaintext display-preferences file (the handful of fields
that must render before the master key is injected) and a master-key-sealed
behavior-preferences file, applying the rule "seal at rest unless a field must be
readable before unlock." The screen-capture protection toggle moves into the
sealed half by hardcoding a safe (secure) pre-unlock default, so it is never read
before authentication.

## Why

The plaintext file held lock policy, clear timers, autosync, the biometric-gate
intent, and the screen-capture mode alongside genuinely pre-unlock fields
(language, theme, log level). None are secrets, but leaving them readable by a
local read attacker / forensic dump is inconsistent with the threat model that
motivated sealing the repository config and identity. Sealing the behavior half
shrinks the plaintext surface, makes tampering of those prefs detectable (the
AEAD authentication tag), and gives the project one uniform storage rule instead
of two.

The screen-capture toggle deserves special treatment: a tampered "off" value
disables capture protection for the credential entry surfaces. Sealing it would
seem impossible (it must apply before unlock), but the unlock surface is itself a
credential that should always be secured — so the toggle is sealed and the
pre-unlock state is hardcoded secure, relaxing only after authentication reads
the sealed value. This also closes a pre-existing warm-resume gap where the
app-lock surface could render capturable.

## Context

This reopens and revises the shipped scope-split decision. The three persistence
tiers are unchanged (Git; sealed files; plaintext files); this change moves the
behavior prefs from the plaintext tier into the sealed tier, alongside the
existing repository config, identity, and identity-passphrase slot.

Key design-level points:

- The split is driven by read timing, not confidentiality: a field is sealed
  iff it is not needed before the master key is injected. Language, theme, log
  level, and the migration schema version stay plaintext (read pre-unlock);
  everything else is sealed.
- The screen-capture toggle is sealed with a safe pre-unlock default (secure),
  relaxing post-unlock. Its degradation on master-key loss is also safe (defaults
  to the sensitive mode), unlike autosync, which must degrade to "off" on key loss
  to avoid silently re-enabling sync.
- Desktop has no key store, so the sealed tier degrades to plaintext passthrough
  there, identical to the existing repository config and identity. Sealing is
  therefore Android-only in effect; desktop tamper of the behavior file remains
  accepted (the local write attacker is out of scope on desktop).
- A one-shot migration transitions existing installs from the legacy single-file
  shape to the split, deferring the sealed write when the master key is absent
  (app-lock cold start) and completing it on the first authenticated unlock. The
  migration writes the plaintext half before repurposing the shared file name, so
  display preferences are never lost mid-transition.
- The frontend caches that read behavior prefs are one-shot by design; they gain
  a reload step that runs after the first authenticated unlock, because their
  cold-start load (under the app-lock surface) now reads defaults.

## Alternatives considered

- **Keep one plaintext file (status quo).** Rejected: the plaintext surface is
  larger than the threat model justifies and the rule is non-uniform.
- **A parallel sealed store owned by the app shell, separate from the library's
  seal.** Rejected: the library already hosts one app-shell sealed concept (the
  identity-passphrase slot), so adding a second sealed store would duplicate the
  seal machinery and blur the layering for no benefit.
- **Seal the screen-capture toggle by reading it pre-unlock from the sealed
  file.** Impossible without the master key; replaced by the hardcoded-secure
  pre-unlock default, which is strictly safer.
- **Protect the credential capture surface with an overlay hook instead of
  sealing the toggle (leave the toggle plaintext).** Considered and retained as
  part of the implementation regardless: the credential surfaces are force-secured
  by runtime overlay state under every mode, which is tamper-proof independent of
  storage. Sealing the toggle additionally protects the non-credential surfaces
  under "off," so both are applied.

## Effort

M / L (CC). One library slot plus accessors, an app-shell store split, a one-shot
migration with regression coverage, a frontend reload + overlay-wiring change,
and the test rewrites that the two-file layout forces.

## Depends on / Supersedes

Supersedes the shipped app-config scope split (reopened and revised).
