# Background sync under App Lock via auth-free git credentials

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Background sync (R061) is functionally dead whenever App Lock is on: the worker
can only retrieve the **auth-free** master key, which is absent under App Lock
(the master key migrates to the biometric-gated alias), so every scheduled tick
skips. This RFC unblocks it **without weakening the identity**: seal `repo.json`
(git remote + credentials + authenticity trust set) under a dedicated
**auth-free** key the background worker can retrieve at any time, while
`identity` and its passphrase slot stay sealed under the **biometric-gated**
master key. The worker decrypts only the git credential it needs to pull; it
never touches the age identity.

Serves `docs/specs/005-git-storage/` (background sync) and preserves
`docs/specs/007-app-lock/` (the identity stays behind the gate).

## Why

Under App Lock the master key sits behind a per-use STRONG biometric
(`setUserAuthenticationRequired(true)` + `setUserAuthenticationParameters(0,
AUTH_BIOMETRIC_STRONG)`) that a headless WorkManager job cannot satisfy, so the
sealed `repo.json` — including the git credential sync needs — is unreadable and
the tick skips (issue #21). Foreground sync after unlock still works, but the
heavy-autofill user who rarely opens the app never converges.

Pull-only sync needs only the git transport credential — PAT **or** SSH key,
both fields in `repo.json` (`RepoConfig::to_git_auth`). It moves encrypted blobs
and never decrypts secret contents, so it has no need for the age `identity`.
That separability is the whole point: make the git credential auth-free
(unblocking sync) and leave the identity — the crown-jewel key that decrypts
every secret — behind the biometric, exactly where the threat model wants it.

The broader "decouple at-rest from App Lock" idea (the retired R063) proposed
making the single shared master key auth-free, which would also make the
identity auth-free. That sacrifice is justified only by zero-step (prompt-free)
autofill, which we are **not** pursuing: per-fill biometric via the OS Autofill
Framework's dataset-auth is the acceptable autofill UX for users who already
biometric-unlock routinely, and it does not require the identity to leave the
gate. Making the identity auth-free population-wide to save one biometric tap at
fill time is a bad trade against the security model, so this RFC deliberately
does not do it.

## Context

**The split, concretely.** Today one master key seals `repo.json`, `identity`,
and `app_id_pass`. After this RFC:

1. `repo.json` is always sealed under a dedicated **auth-free** Keystore key
   (hardware-backed AES/GCM, `setUserAuthenticationRequired(false)` — the same
   shape as today's auth-free master key) and **never migrates on the App Lock
   toggle**. The background worker retrieves this key directly — it already
   knows how, via `MasterKeyAccess.loadAuthFree` — decrypts `repo.json`, reads
   the git credential, and pull-syncs. No prompt.
2. `identity` and `app_id_pass` keep the master key that follows the App Lock
   toggle (auth-free when App Lock is off, biometric-gated when on), exactly as
   today. Copy / show / create still require the per-use biometric via the gate;
   the "locked app holds no identity key" property is preserved.

**Migration.** A forward migration re-seals the existing `repo.json` under the
new auth-free key: one last biometric-gated read of the current master key to
recover the plaintext `repo.json`, then re-seal. It therefore runs post-unlock
(it needs the gated key once) and is a one-time cost on the next unlock after
upgrade; background sync under App Lock starts working from the tick after that
migration completes. The migration registry is permanent; old migrations stay as
history.

**Threat-model impact — narrow by design.**

- _Read / dump / forensics attacker_: **unchanged.** Both keys are
  hardware-backed and absent from any data dump; AEAD integrity on `repo.json`
  and `identity` stays.
- _Physical-access / shoulder-surfer_: **unchanged.** UI gate and per-secret
  reveal flow untouched.
- _Process / memory attacker_: the git credential (PAT / SSH transport key)
  becomes process-retrievable while the app runs, locked or not — but **that is
  already the status quo for App-Lock-off users**, whose master key is auth-free
  so `repo.json` is already process-readable. This RFC extends that existing,
  accepted exposure to App-Lock-on users' git credentials, and **no further**:
  the age identity — the key that actually decrypts secrets — stays
  biometric-gated and is not process-retrievable. The credential exposed is
  repo-scoped and rotatable (re-enter the token / rotate the SSH key); the crown
  jewel is not exposed.

**Honest caveat — same-material keys.** If a user configures the _same_ SSH
private key as both the git transport key and their age identity (age-plugin-ssh
style), the two files hold identical material and the split protects nothing for
that user. On Android this configuration is unavailable (age-plugin-ssh is a
desktop-only subprocess plugin) and the Android age identity is a native age key
distinct from the git SSH key, so the split is real exactly where App Lock +
background sync matter. Documented as a residual.

## Residual risks (what we accept)

- **Wider git-credential residency under App Lock.** The PAT / SSH transport
  key is now process-retrievable while App Lock is on (it wasn't before).
  Bounded: repo-scoped, rotatable, and identical to what App-Lock-off already
  assumes. A cheap mitigation is to hold the credential in memory only for the
  sync operation and re-fetch per tick.
- **Same-material SSH key (above).** Desktop-only configuration where
  git-transport key == age identity; the split is illusory there. Not reachable
  on Android.
- **Two keys to manage.** Enabling / disabling App Lock now migrates `identity`
  between biometric-gated and auth-free stores, while `repo.json` always lives
  under the auth-free key. More moving parts than one shared key; the migration
  carries its own failure/recovery path (re-setup on a lost auth-free key,
  mirroring today's lost-master-key recovery).

## Alternatives considered

- **Full decoupling — make the single shared master key auth-free (retired
  R063).** Rejected: it makes the age identity auth-free population-wide,
  removing the "locked app holds no identity key" property for every user. That
  is justified only by zero-step (prompt-free) autofill, which we are not
  pursuing; per-fill biometric covers autofill without it. Trades a crown-jewel
  protection for one UX tap.
- **Keep the crypto gate; forgo background sync under App Lock (status quo).**
  Rejected: background sync is functionally dead for every App-Lock-on user (the
  realistic majority for a password manager), so the feature ships but does not
  deliver. Foreground sync converges only when the user opens and unlocks —
  exactly the heavy-autofill user who never does.
- **Cache the master key so the background worker can use it.** Rejected as
  security theater: a key cached for unattended background use is, by
  definition, process-retrievable without a user. It keeps the illusion of a
  gate without its property — and it would expose the identity, not just the git
  credential.
- **Make the identity auth-free but not the git credential.** Rejected:
  backwards — background sync needs the git credential, not the identity, so
  this unblocks nothing while still sacrificing the identity.

## Effort

~M (human) / ~S (CC). Add a dedicated auth-free key for `repo.json`, rewire
`Config` / `Seal` to seal `repo_config` under it while `identity` /
`app_id_pass` keep the App-Lock-toggle master key, add the one-time forward
migration (post-unlock re-seal), point `MasterKeyAccess` at the new key, and
re-scope `007-app-lock/security.md` to state that the **identity** (not the
master key wholesale) is what stays gated. The background-sync tests invert from
skip-while-AppLock to runs-while-AppLock; issue #21's interim UI gate is then
removed (sync works under App Lock, so the setting no longer misleads).

## Depends on / Supersedes

- Builds on the shipped background sync (R061), whose AppLock-off path already
  runs; this lifts the AppLock-on gap.
- **Retires R063** (decouple at-rest from App Lock). R063's full decoupling made
  the identity auth-free to serve zero-step autofill, which we are not pursuing.
  This RFC takes only the part R063 identified correctly — auth-free git
  credentials for sync — and leaves the identity gated.
- Serves `docs/specs/005-git-storage/` and preserves `docs/specs/007-app-lock/`'s
  identity-protection promise.
