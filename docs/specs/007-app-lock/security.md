<!--
Feature-level threat model for app lock, auto-lock, and at-rest encryption. Complements
docs/SECURITY.md. Living.
-->

# 007 — App lock & auto-lock: threat model

## Assets

The **app key** = the age/SSH `identity` plus the git credentials in `repo.json` (which
also carries the `authenticity` trust set). At-rest encryption and App Lock protect
these at rest, and (with App Lock on) in memory.

## At-rest encryption (Android)

Both `identity` and `repo.json` are encrypted at rest (hardware-backed AES-GCM in the
Android Keystore), but R064 split the at-rest key into two: `repo.json` (and the
`app.json` behavior slot) sit under an **auth-free** master key — permanently
retrievable without a prompt — while `identity` (and its passphrase slot) sit under a
**vault key** that follows the App Lock toggle. An attacker who can _read_ the app's
private storage — a stolen backup, a forensic dump, a non-root malicious app with
storage access — gets ciphertext, not the git credentials, trust set, or identity.
The same authenticated encryption gives these files **integrity**: a modified
`repo.json` (flipping the verification mode, injecting an attacker signing key) or a
swapped `identity` fails the auth tag and is rejected, not silently accepted.

The auth-free master key lives in app memory for the session — no more sensitive than the
git credentials already held in memory while cloning/syncing, and consistent with the
non-goal of not defending against a compromised OS or the app's own process (which could
ask the Keystore to unseal it anyway). The vault key is loaded only after the app-unlock
biometric and wiped on re-lock (see below). If a Keystore key is lost (app data cleared,
Keystore wiped, factory reset) the encrypted files become unreadable and re-setup is
required; there is no escrow, since an escrow key on disk would defeat the purpose.

**Still assumed, not solved by at-rest encryption** (system-wide): gpm assumes no local
write attacker. A write attacker can roll a file back to an older plaintext;
authenticated encryption prevents _forging_ a new ciphertext but not a rollback. On
Android the no-write assumption rests on the app sandbox; on desktop there is no
Keystore equivalent, so files stay plaintext and the assumption rests on the user
account — a documented asymmetry.

## App-launch biometric gate (opt-in App Lock)

The optional App Lock raises the at-rest defense into a real lock screen. When on, the
**vault key** — the key that seals `identity` and its passphrase slot — is placed behind
a **biometric-gated** Keystore key (still hardware-backed AES-GCM, but every use requires
a STRONG biometric). The age `identity` is then unreadable — on disk _and_ in memory —
until the user authenticates: gpm builds without the vault key at launch, injects it only
after the app-unlock biometric prompt, and **wipes it when the app returns to the
foreground**, so a locked app cannot decrypt secrets even from a memory snapshot. One
biometric prompt gates the identity; the auth-free master key (which seals `repo.json`)
stays loaded so the headless background worker can still pull-sync while the app is
locked. The identity `UnlockModal` is suppressed while the app-lock overlay is up so the
two never race.

**Wipe-on-resume, not wipe-on-background.** The wipe fires on resume (foreground
return); while backgrounded, before the resume fires, the vault key remains in memory
until the re-lock runs. Consistent with the threat model (a process-running attacker is
a non-goal), but the guarantee rests on the WebView firing `visibilitychange` on resume —
the norm on Android but not contractually guaranteed on every OEM build. An authoritative
`Activity.onResume` signal is tracked as future hardening.

**In-app idle re-lock (opt-in).** Beyond resume, App Lock can also re-lock after a
configured stretch of foreground inactivity ("Re-lock when inactive": Off, or 5/15/30
min; off by default for existing users, 5 min for new). The idle wipe is the same as
the resume wipe — vault key + identity cache gone, non-dismissable mask up — but it
carries an `idle` reason that suppresses the auto-biometric-prompt: the user is
present but idle, so the mask waits for a tap rather than firing a prompt. The
foreground-return lock still auto-prompts as always. The idle timer is backend-owned
(a throttle-able JS timer could delay the wipe) and shares the in-app activity signal
with the identity idle timer; like the resume wipe it can fire mid save / autosync /
delete / divergence-resolve and surface `SealKeyUnavailable` — accepted (consistent
with the gate's no-write-mutex design), recoverable in git, with exposure bounded by
the per-operation timer reset to ops longer than the idle window.

**Enrollment does not brick** — the biometric-gated vault key is _not_ invalidated by
enrolling a new fingerprint/face. **Removing all biometrics does** invalidate it → the
identity becomes unreadable → re-setup (re-clone, re-enter git token) is the only
recovery; no escrow. (`repo.json` stays readable under the auth-free master key, so only
the identity — not the git credential — is lost.) This is the accepted residual risk of
the opt-in.

**One prompt, not two.** A separate _Identity Auto-Unlock_ toggle (off by default,
independent of the Auto-Lock timing presets) seals the identity passphrase under the
vault key; when on, a successful app-unlock also unlocks the identity session with no
second prompt.

The gate re-challenges on every foreground return (cold start and warm resume alike).
Desktop has no Keystore equivalent → App Lock unavailable, files stay plaintext.

## Auto-lock (identity cache lifecycle)

The in-memory identity is wiped on inactivity rather than left sitting in memory. Modes:
Immediate (decrypt per operation, wipe right after — key in memory only for the op),
Idle (cached until timeout; the timer resets on in-app activity, not just secret ops),
Never (cached for the session). A failed op also clears the cache. During divergence
resolve the Immediate wipe is deferred (see `005/security.md`).

When **Identity Auto-Unlock** is on, the identity has no independent auto-lock — its
cache is restored by the gate's unlock and wiped by the gate's lock, so Immediate,
Idle, and Never are all ignored (the per-op Immediate wipe is suppressed too). The
settings UI reflects this by disabling the Auto-lock control with a "Managed by App
Lock" note. When Auto-Unlock is off, the three modes above run unchanged.

## Cross-references

- System-wide non-goals / no-write-attacker assumption: `docs/SECURITY.md`.
- Divergence-resolve identity-cache deferral: `005/security.md`.
