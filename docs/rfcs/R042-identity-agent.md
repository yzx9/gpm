# Identity-state agent (deferred)

**Priority:** P2
**Status:** Blocked
**Phase:** Future

## What

A dedicated in-process component that owns the unlocked-identity cache and the
unlock/lock lifecycle, separating identity state from both the crypto backend
(which stays stateless) and the store facade. Today that state lives on the
facade; this RFC records the decision to eventually extract it into its own
component — and the decision, in the same breath, to **defer** that extraction
until a second consumer forces the component's shape.

## Why

Two pressures push identity state toward its own home:

1. **gopass alignment.** gopass splits its crypto backend (stateless) from its
   agent (holds unlocked keys), and gpm's crypto backend is converging on the
   same stateless shape as the second backend arrives. Identity state then
   reads more naturally as a dedicated concern than as facade fields.

2. **Hardware keys need a session cache.** A hardware key (an OpenPGP card, the
   age-plugin-yubikey path) charges a PIN or a touch per operation. Without
   caching the unlocked session for a TTL, every decrypt re-prompts — unusable.
   An agent is the natural owner: software keys cache decrypted key material,
   hardware keys cache the card session, one component, one policy. This is the
   load-bearing reason the component exists at all; without it, the facade cache
   suffices.

## Context

The question surfaced while reshaping the crypto trait so the backend owns
recipient resolution and the encrypt/decrypt pipeline (the change that lets a
future GPG backend resolve fingerprint recipients through its own keyring rather
than receiving age-shaped recipient strings). The first-cut plan extracted an
identity agent in that same step. On closer analysis, extracting it inside a
phase whose whole point is zero behavior change carries real costs that a
deferral avoids:

- **The auto-lock policy is not the backend's to own.** The Immediate / Idle /
  Never wipe policy, and the deferred wipe that lets a keep-mine resolve reuse
  an unlocked identity without a second unlock, are decided in the app layer —
  the backend crate has no concept of lock modes. An agent that "owns" locking
  would misrepresent where the policy lives; the agent can only own the cache
  and expose a lock primitive the app layer still has to call.

- **The unlock-status signal has semantics the cache must respect.** A plaintext
  identity (the default, no-passphrase case) reports "not unlocked" forever,
  because nothing was ever unlocked — the user enters no passphrase. An agent
  that caches a plaintext identity on first use would flip that signal to
  "unlocked," which is wrong, and which the app's lock UI depends on. Distingu
  ishing "cached because plaintext" from "cached because decrypted" is solvable
  but is exactly the kind of behavior-change debate a zero-change phase should
  not absorb.

- **The component's shape is unknown until a hardware key forces it.** Is the
  cache a single slot or per-key? Does it carry a TTL? Is it biometric-gated?
  Does it hold a card-session handle rather than key bytes? Each answer changes
  the abstraction. Designing those answers blind — ahead of the one consumer
  that needs them — risks the same "trait shape guessed from a single
  implementation" failure the crypto-trait reshape exists to correct.

The stateless-backend pipeline reshape lands without any of those debates by
leaving the cache on the facade. The agent earns its keep when a hardware key
arrives and the cache shape is concrete; extracting it then is one focused
structural change, not a behavior-change fight bolted onto a refactor. This is
the "make the change easy, then make the easy change" sequencing, and the same
"do not design the abstraction ahead of the second backend" principle the
crypto-trait work already follows.

**Threat-model note.** Until the agent arrives, identity handling is unchanged:
the plaintext identity is re-read per operation in Immediate mode and held for
the auto-lock window in Idle/Never, exactly as the encrypted-identity cache is
today. The agent extraction itself is not a security change; the hardware-key
session-cache policy it will carry _is_ one, and belongs to whatever RFC
delivers that path.

## Alternatives considered

1. **Extract the agent now, alongside the pipeline reshape.** Rejected: the
   three costs above — mis-located policy, the unlock-status semantics break,
   and a blind shape guess — each add risk to a phase whose contract is zero
   behavior change. Reassess when a hardware-key consumer lands.

2. **A separate agent daemon process (the gpg-agent / gopass-agent model).**
   Rejected: gpm is a single-process app on Android and desktop; a daemon adds
   IPC and lifecycle cost to serve no second client. When the agent arrives, it
   is an in-process component, not a daemon.

## Effort

Medium when it arrives — one new component plus the move of the cache and the
unlock/lock lifecycle off the facade, and a real threat-model update for the
session-cache policy. Trivial now: this RFC is the deferred-work record.

## Depends on / Supersedes

Relates to `0036-gpg-crypto-backend` (whose trait reshape raised the question),
`0030-age-plugin-yubikey` (the first consumer that will force the session
cache), and `0009-gpg-signature-verification` (unrelated, but shares the rpgp
seam). Supersedes an earlier same-day call to put the identity cache on the
stateful backend: the backend instead stays stateless and the cache remains on
the facade until this RFC unblocks.
