# Decouple At-Rest Encryption from the App-Lock Biometric Gate

**Priority:** P1
**Status:** Draft
**Phase:** Next

## What

The app-launch biometric gate today conflates two unrelated concerns: a **UI
privacy gate** (biometric to view the app) and a **cryptographic master-key
gate** (the at-rest master key is sealed behind a biometric-gated Keystore key
and wiped from memory on background, so a locked app holds no key). This RFC
splits them: **keep** at-rest encryption (under an auth-free master key) and
**keep** a biometric UI gate, but **remove** the master-key crypto gate. The
master key becomes retrievable by the app process without a live biometric at
all times — the one change that lets background autofill and background sync run
without a present user. The UI gate keeps defending the physical-access attacker;
it just no longer claims to harden the crypto.

Serves the app-lock feature (`docs/specs/007-app-lock/`). Unblocks the autofill
design (`docs/specs/008-android-autofill`, R056) and the background-sync work
(R061).

## Why

Two planned features need to produce plaintext — a credential to fill, or git
credentials to sync — from a context with **no foreground activity and no present
user to satisfy a biometric prompt**:

- **Background sync** (R061) is OS-scheduled and headless. The on-demand
  background sync path already exists but is forced to **skip while the gate
  holds the master key**, because syncing needs the git credentials that are
  at-rest-encrypted under the master key, and the gate withholds that key until a
  biometric prompt the background scheduler cannot show.
- **Background / zero-step autofill** (R056, spec 008) is user-gesture-initiated,
  but the goal is a single tap that fills directly. Producing the credential
  needs the master key; under the crypto gate that means a biometric at fill time
  — i.e. not zero-step.

A background service runs as the app's own process, so "the background can
decrypt" and "any code in the process can decrypt" are the same statement. That
makes a biometric-gated master key **fundamentally incompatible** with autonomous
background decryption: the gate's whole purpose is to withhold the key from
non-authenticated process code, which is exactly what the background service is.

Meanwhile the crypto gate's security yield is narrow. It defends precisely the
_process-attacker-while-the-app-is-locked_ window — and the system threat model
already names the process-running attacker as a **non-goal** (such an attacker
can ask the Keystore to unseal the key regardless of any gate). R058's own
analysis concedes this equivalence. So the gate blocks two core mobile features
to defend a sliver of a non-goal.

Decoupling removes the conflict at its root and retires the wedge of lock-aware
machinery — the background-sync skip, the wipe-on-resume, the migration path that
defers until the key arrives, and the biometric-gated master-key store plus its
enable/disable move between stores — that exists only to serve that narrow
defense.

## Context

**Three layers, after the split.**

1. **At-rest encryption stays.** The config and identity remain AEAD-sealed under
   a master key wrapped by an auth-free, hardware-backed Keystore key. This is
   what defeats the read/dump/forensics attacker — the wrapping key is absent
   from any app-data dump — and it is independent of whether a biometric gate
   exists. Removing the crypto gate does **not** remove at-rest encryption.
2. **A biometric UI gate stays.** Opening the app's UI requires STRONG biometric.
   This defends the threat model's primary named attacker — _brief physical
   access to an unlocked device_ (someone picks up the phone and opens gpm). It
   does **not** withhold the master key; it is a privacy boundary over the UI,
   not a crypto boundary.
3. **The master-key crypto gate is removed.** No biometric-gated master-key
   store, no wipe-on-background, no resume re-challenge of the _key_.

**Threat-model impact — what moves, what doesn't.**

- _Read / dump / forensics attacker_: **unchanged.** At-rest encryption remains;
  a data dump still yields ciphertext.
- _Physical-access / shoulder-surfer_: **unchanged.** The UI gate still stops a
  human browsing the store through the app's own UI. Any key-wipe was irrelevant
  to this attacker, who interacts via the UI, not memory.
- _Process-running / root attacker_: **the locked-window defense is gone**, but
  that window defended a non-goal. Before: while locked, a process attacker could
  not get the key (wiped; re-unseal needs a biometric). After: the auth-free
  master key is process-retrievable at all times, locked or not. This is an
  accepted widening against an already-out-of-scope attacker, not a new in-scope
  hole. The honest framing: the UI gate defends the _human_ threat; at-rest
  encryption defends the _dump_ threat; nothing defends the _process_ threat, by
  design.
- _Biometric phishing_: the crypto gate's key-bound prompt could be triggered by
  process-attacker code to induce a live biometric; that path disappears with the
  gate. Autofill may still layer its own per-fill biometric (below), but that is a
  dataset-level UX choice, not the app's master-key unlock.

**The two scenarios, after the split.**

- _Background sync_: the scheduler retrieves the auth-free master key on demand,
  decrypts the git credentials, and pull+pushes — no UI, no prompt. The skip
  while the gate holds the key goes away; sync runs whenever the OS schedules it.
- _Background / zero-step autofill_: the autofill service retrieves the auth-free
  master key, decrypts the matched entry, and fills the target field on a single
  tap. A per-fill biometric can still be offered via the OS's autofill
  dataset-auth mechanism for users who want the speed-bump — but it is layered
  _on top of_ the auth-free key, not coupled to it, so it never blocks sync and
  never re-introduces the conflict.

**The config display/behavior split becomes unnecessary.** The config was split
into a plaintext display half and a sealed behavior half specifically so the
display half (locale, theme, the migration schema version) stays readable at cold
start, when the crypto gate had not yet injected the master key. With the master
key always available, the whole config can be read at cold start and the split's
rationale dissolves. A forward migration re-merges the two halves into a single
at-rest-sealed config. **Sequencing constraint:** the merge must land _with or
after_ the decoupling, not before — re-merging while the crypto gate still
withholds the key would re-create the exact cold-start-unreadability the split
was built to fix. Old migrations stay as history; the registry is permanent.

## Residual risks (what we lose)

- **The "locked app holds no key" property is gone.** A memory snapshot of a
  locked app now contains the master key. This matters only to the process/root
  attacker (non-goal), but it contradicts an assertion the current docs make, so
  `007-app-lock/security.md` and `docs/SECURITY.md` must be re-scoped — not
  silently.
- **The biometric gate no longer "hardens the crypto."** Any wording implying
  biometric makes the encryption more resistant to malware must go; the gate is a
  privacy / anti-shoulder-surf boundary. Under-selling the change is the risk to
  avoid.
- **Wider key residency.** The auth-free master key is re-fetchable by the
  process at any time. If the implementation keeps it resident for the whole
  session the memory window widens; a cheap mitigation is to drop the key after
  each operation and re-fetch on demand (prompt-free), bounding residency to
  operation windows.
- **Migration surface.** Existing users with the biometric-gated master-key store
  must move back to the auth-free store, and the config split must be re-merged —
  two real migrations with their own failure/recovery paths, gated on the
  sequencing above.

## Alternatives considered

- **Keep the crypto gate; forgo background sync and zero-step autofill.**
  Rejected: those are core to a mobile password manager (autofill especially),
  and the gate buys defense of a non-goal. The trade runs backwards.
- **Dual mode — auth-free by default, opt-in crypto gate that disables background
  features.** Rejected: it re-introduces the entire lock-aware machinery to defend
  a non-goal, adds a third user-facing mode that muddies the security promise, and
  the "paranoid" mode still falls to a process attacker who can phish a biometric.
  Full complexity for non-goal defense.
- **Keep the crypto gate but cache the master key so background code can use it.**
  Rejected as security theater: a key cached for unattended background use is, by
  definition, a key the process can fetch without a user — indistinguishable from
  auth-free. It keeps the complexity and the illusion without the gate's property.
- **Split the keys — an auth-free sub-key for git credentials (sync) only,
  identity stays biometric-gated.** Rejected: it unblocks sync but not zero-step
  autofill (which needs the identity), adds key-splitting and dual-encryption
  complexity, and a process attacker recovers both keys anyway. Partial fix, full
  complexity.
- **Keep wiping the key; run background work only while the app is foregrounded.**
  Rejected: that is not background work, and it defeats the convergence goal of
  R061 (a device that is never opened).

## Effort

~M–L (human) / ~M (CC). Remove the wipe-on-resume and in-app-idle key-wipe paths,
the biometric-gated master-key store and its enable/disable move between the two
Keystore stores, the lock-aware migration deferral (and likely the whole
wait-for-key-then-resume mechanism in the migration engine, whose main client was
the key-withheld window), and the background-sync skip; re-scope the app-lock
overlay to a pure UI gate; add the config re-merge forward migration; re-scope
the threat-model docs. The existing lock-state and background-sync tests invert
(skip-while-locked → runs-while-locked).

## Depends on / Supersedes

- Serves `docs/specs/007-app-lock/`; redefines App Lock as a UI privacy gate.
- **Supersedes** the master-key-wipe substrate of R029 (authoritative re-lock) and
  R058 (resume timeout): the wipe is retired. R058's _idea_ — an opt-in timeout
  before re-prompting — can re-migrate to the UI gate (re-show the biometric UI
  after a timeout), but it then times a UI re-prompt, not a key wipe.
- **Unblocks** R056 (zero-step autofill) and R061 (reliable background sync).
- The config re-merge retires the rationale of the display/behavior split; that
  split migration stays as history (registry is permanent).
