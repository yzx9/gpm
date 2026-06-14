# Add git commit signature verification / repo authenticity

**Priority:** P3
**Status:** Designed (awaiting implementation sign-off)
**Phase:** Future (post-MVP, after [0006-multi-identity.md](./0006-multi-identity.md))

## What

Detect a compromised or malicious git remote that feeds **validly encrypted but
wrong** entries by verifying the signature on every commit pulled into the store.

gpm is read-only and age-only — there is no GPG, no editing. So the natural and
_only_ signing mechanism this plan adopts is **SSH-signed commits** (`git commit
-S` with `gpg.format = ssh`), verified against a user-managed set of trusted
signing public keys. GPG-signed commits are explicitly **out of scope** (see
[Non-goals](#non-goals)).

The feature is a **tri-state per-repo setting**:

| Mode    | Behaviour                                                                                              |
| ------- | ------------------------------------------------------------------------------------------------------ |
| Off     | No verification. (Today's behaviour — the default.)                                                    |
| Audit   | Verify on every pull. On mismatch, pop up a warning. Pull always succeeds. Per-commit "ignore" OK.     |
| Enforce | Verify on every pull. A blocking issue aborts the pull (HEAD stays put). BadSignature never ignorable. |

Plus the surrounding UX the user asked for:

1. **Mode selector** — Off / Audit / Enforce (req 1).
2. **Per-commit popup with ignore** — tap a flagged commit → dialog → "ignore
   this issue" records a scoped exception (req 2).
3. **Mismatch popup in Audit mode** — after a pull with issues, a modal lists
   them (req 3).
4. **Signature status indicator** — a persistent badge on the entry-list header
   reflecting the verified / warning / off state of HEAD (req 4).
5. **Git history + signature query** — a new `/history` screen listing recent
   commits with a per-commit signature column and a detail view (req 5).
6. **Signing key management** — Settings screen to add / view / label / remove
   trusted signing public keys, plus "trust this signer" TOFU from the history
   view (req 6).

## Why

The original plan's framing still holds and is the whole reason this exists:

> Outside voice (Codex) identified that there is no provenance check beyond "git
> pull succeeded." A malicious or compromised remote can feed perfectly valid
> encrypted entries that decrypt to wrong data. The user has no way to detect
> this.

age guarantees **confidentiality** (only a holder of the identity can read
entries) but **not authenticity** of the store history. `git pull` succeeding
only proves you received _a_ valid git object graph — not that it came from
someone you trust, or that commits weren't rewritten in transit. For a product
whose pitch is trust, that is the most important gap after the core crypto.

Concretely, an attacker who controls (or has compromised) the remote can:

- Commit a brand-new `aws/root.age` that decrypts fine under your real identity
  but contains a password the attacker also knows.
- Rewrite an existing entry's history.

Both produce age blobs that decrypt without error. The only signal that
something is wrong is that the **commit** carrying the blob is either unsigned
or signed by a key you don't trust. Commit signature verification is the direct
countermeasure.

## Threat model — what this does and does not defeat

**Defeats** (in Enforce; detects in Audit):

- Compromised remote feeding unsigned malicious commits.
- Compromised remote feeding commits signed by an attacker-controlled key.
- Tampering with a signed commit's contents (any edit invalidates the SSH
  signature over the commit object — detected as `BadSignature`).
- Replay of an old commit (fast-forward-only pull + "verify the new range"
  makes a replayed commit a no-op; a _new_ malicious commit is the only path
  left, which falls into the two cases above).

**Does NOT defeat** (out of scope, call out explicitly):

- The **signing key itself** being compromised. If the attacker has the trusted
  signer's private key, their commits verify as genuine. No commit-signing
  scheme solves key compromise; rotation + revocation do, and we provide key
  management to support that.
- A malicious commit made **before the user enabled the feature**. Verification
  is forward-looking from the enable moment. (The `/history` screen lets a user
  audit the pre-existing past if they want; see [Verification
  scope](#verification-scope).)
- Repository host metadata spoofing (TLS-MITM, DNS). Handled by HTTPS + SSH
  transport trust, not by this plan.

## Key design decisions

### Decision 1 — SSH-signed commits only, not GPG

The plan stub left "GPG or SSH" open. **We commit to SSH-only** for v1.

**Why:**

- `ssh-key` is **already a workspace dependency** (gpm uses it for SSH-identity
  key generation, public-key derivation, and SSH git auth). SSH-signature
  _verification_ reuses the same crate — `PublicKey::verify` — so **no new
  crypto dependency** is added and no vendored GPG/OpenPGP stack is pulled in.
  GPG verification would require `sequoia-openpgp` or the `pgp` crate (large,
  and a GPG keychain story gpm otherwise doesn't need).
- gpm is SSH-first already: SSH identities, SSH git remotes, in-app SSH keypair
  generation. SSH signing is the consistent choice.
- SSH-signed commits are first-class in git ≥ 2.34 and on GitHub (signing with
  an uploaded SSH key, and GitHub's own web-flow keys). The ecosystem is mature.
- GPG is a non-goal users can always add later via a parallel verifier; the
  data model below leaves room (statuses distinguish "signed but unknown
  format").

**Confirmed primitives (the load-bearing facts):**

- `git2::Repository::extract_signature(&oid, None) -> Result<(Buf, Buf)>`
  returns `(signature, signed_data)`. SSH signatures live under the same
  `gpgsig` header as GPG; libgit2 returns the header contents verbatim, so we
  parse the armor afterward to tell SSH (`-----BEGIN SSH SIGNATURE-----`) from
  GPG. git2 0.20 exposes this (verified against the crate source).
- `ssh_key::PublicKey::verify("git", signed_data_bytes, &parsed_sshsig)` verifies
  an SSH signature over the commit object data in the `"git"` namespace. The
  signature parses with `"-----BEGIN SSH SIGNATURE-----\n…\n-----END SSH
SIGNATURE-----".parse::<ssh_key::SshSig>()`. (Verified against ssh-key docs.)
  Feature flags: existing `["ed25519", "encryption", "rand_core", "std"]`
  cover ed25519 verification; confirm at implementation time that no extra flag
  is needed for `SshSig`/`verify` (likely none beyond `ed25519` + `std`, which
  implies `alloc`). We **only verify, never sign**, so no signing-side deps.

This is why the effort is "~1 hour CC" rather than days: the hard part (pure-Rust
SSH-sig verification with deps we already have) is solved by two calls.

### Decision 2 — TOFU with explicit consent, not silent trust

"Key distribution" is the open question the stub flagged. gopass has no signing
standard, so we define our own, minimal and explicit:

- **Trusting the first key is explicit.** When the user turns the feature on
  (Off → Audit or Enforce), the current HEAD's signer (if signed) is presented
  for one-tap confirmation: _"This repo's commits are signed by
  `SHA256:abcd…` (comment). Trust this key?"_ This is the trust anchor. If HEAD
  is unsigned at enable time, the user must paste a signing public key
  manually (Settings) before Enforce is meaningful.
- **Subsequent keys are added explicitly** — either by pasting an SSH public key
  in Settings, or by "Trust this signer" from the `/history` detail view.
- **No implicit trust on every new key.** A commit signed by a key not in the
  trusted set is `UntrustedKey` — a warning (Audit) or a block (Enforce), never
  an auto-trust. This is the difference between "audit mode" and "no mode".

Rationale: the user is opting into a security feature; one confirmation at enable
time is appropriate friction and matches gpm's "trust is the product" stance.
Silent TOFU would let the first commit an attacker feeds become a trusted anchor.

**Clarification:** the trusted **signing** key is unrelated to the age/SSH
**decryption** identity. The signing key is whoever maintains the gopass store
(you on desktop, a teammate, CI). The decryption identity unlocks entries. They
are stored, managed, and rotated independently. Conflating them is a common
mistake this plan deliberately avoids.

### Decision 3 — Verify the full new range, not just HEAD

On a pull, verify **every commit in the range `(pre-pull HEAD, post-pull HEAD]`**,
not just the new tip.

**Why not HEAD-only:** an attacker can push a history where an unsigned/malicious
commit sits _behind_ a legitimately signed tip. Fast-forward pulls in the whole
range; checking only the tip misses the buried bad commit. Verifying the range
closes that.

**Why this is affordable:** gopass stores have small, mostly-linear histories.
A typical pull brings a handful of commits; ed25519 verification is microseconds.
No batching or caching needed for v1.

Commits reachable from the range that predate the trusted-key setup are handled
by [Verification scope](#verification-scope).

### Decision 4 — The signing key is a public trust anchor, stored locally

Trusted signing public keys are **public** data (they're literally the
`ssh-ed25519 AAAA… comment` strings you'd put in `authorized_keys`). They carry
no secret, so they don't need Keystore / biometric protection — they live in a
plain config file in the app-private config dir next to `repo.json`. (Contrast
with the decryption identity / passphrase, which are secret and already handled.)

## Verification scope

- **At enable time:** do **not** re-verify existing history. The current state
  is the trust baseline (the user has been using it). Recording HEAD's signer as
  the anchor _is_ the act of trusting the present state. (A user who wants to
  audit the past can open `/history` and spot-check, but we don't block on it.)
- **At every pull (Audit/Enforce):** verify the new range `(old HEAD, new HEAD]`.
  Each commit's status is computed and filtered through the ignore list. In
  Enforce, any remaining blocking issue aborts before checkout.
- **On demand (`/history` screen):** recompute and display the status of the N
  most recent commits (HEAD and its ancestors), so the user can browse and act.

## Signature status model

A single enum describes a commit's verification outcome. It's the vocabulary
used by the indicator, the popups, and the history screen.

```rust
pub enum CommitSigStatus {
    /// Signed and the key is in the trusted set.
    Verified { signer_fp: String },
    /// Signed by a key not in the trusted set (and not GPG/unknown-format).
    UntrustedKey { signer_fp: String },
    /// No `gpgsig` header at all.
    Unsigned,
    /// Header present, SSH armor parsed, but `PublicKey::verify` failed.
    /// This is the tampering signal — treat as the most severe.
    BadSignature,
    /// Signed with a format we don't verify (e.g. GPG/PGP armor). Treat as a
    /// soft warning: "signed, but not with an SSH key gpm can check".
    UnsupportedFormat { kind: String },
    /// Could not classify (corrupt object, read error). Surface as unknown.
    Unknown,
}
```

Severity ordering (drives the indicator colour and Enforce blocking):

`Verified < UnsupportedFormat < Unsigned < UntrustedKey < BadSignature < Unknown-as-fail`

**Ignore policy by status and mode:**

| Status            | Ignorable in Audit  | Blocks in Enforce           |
| ----------------- | ------------------- | --------------------------- |
| Verified          | n/a                 | no                          |
| UnsupportedFormat | yes                 | yes (unless ignored)        |
| Unsigned          | yes                 | yes (unless ignored)        |
| UntrustedKey      | yes                 | yes (unless ignored)        |
| BadSignature      | yes (scary confirm) | **never** ignorable — block |

`BadSignature` is special: it indicates the commit object was altered after
signing. Letting a user dismiss it in Enforce would gut the feature. It can be
ignored in Audit only, behind a deliberately alarming confirmation.

## Data model

New persisted state, kept in a dedicated `signing.json` in the config dir
(separate from `repo.json` so the public-key trust set is clearly non-secret
and independently migratable):

```rust
#[derive(Serialize, Deserialize)]
pub struct AuthenticityConfig {
    pub mode: VerifyMode,
    pub trusted_keys: Vec<TrustedKey>,
    pub ignored: Vec<IgnoredIssue>,
}

pub enum VerifyMode { Off, Audit, Enforce }

pub struct TrustedKey {
    pub public_key: String,   // "ssh-ed25519 AAAA… [comment]"
    pub fingerprint: String,  // "SHA256:<base64>" — the stable identity
    pub label: String,        // user-given name, e.g. "Alice — laptop"
    pub added_at_commit: String, // HEAD hash when trusted (provenance)
}

pub struct IgnoredIssue {
    pub commit: String,        // full hash — scoped, never "all unsigned commits"
    pub status: CommitSigStatus,
    pub ignored_at_commit: String, // when the user dismissed it
}
```

Design points:

- **Ignore is per-commit-hash + per-status**, never global. "Ignore this
  unsigned commit X" is recorded; a _different_ unsigned commit Y still flags.
  This satisfies req 2 ("忽略这个问题" = this specific issue) without quietly
  disabling the feature.
- **Fingerprint is the identity.** Two `TrustedKey` entries with the same
  fingerprint are duplicates; the UI dedupes on fingerprint. Key rotation = add
  the new fingerprint, optionally remove the old.
- **`added_at_commit` / `ignored_at_commit`** give provenance ("you trusted this
  key when HEAD was abc1234"), useful if a user later wonders why a key is
  trusted. (Timestamps deliberately avoided in the stored model to match the
  repo's hash-addressed mental model; the frontend can show commit hashes.)
- **Backward compat:** absence of `signing.json` ⇒ `VerifyMode::Off` and empty
  lists (today's behaviour). No migration needed.

## Layering (mirrors the biometric plan's discipline)

Two rules, consistent with how biometric was layered:

1. **The crypto lives in `rustpass/`; the policy + UI live in `src-tauri/` +
   frontend.** A new `rustpass/src/signing.rs` module owns the pure logic:
   extracting a signature (`git2`), classifying it, verifying an SSH signature
   (`ssh-key`), and walking a commit range to produce statuses. The `Store`
   facade gets methods like `head_signature_status()`, `verify_range(from, to)`,
   and `authenticity_config()` / `set_authenticity_config()`. `rustpass` knows
   the _status_ of a commit and how to _persist_ a trust set; it does **not**
   know what "Enforce blocks a pull" means as UI.
2. **The blocking decision lives in `Store::sync`.** Enforce "abort pull on
   blocking issue" is a store-level invariant (the working tree must not advance
   past an unverified commit), so it belongs with the pull logic — testable in
   `rustpass` without Tauri. The frontend only reacts to an enriched
   `SyncResult` (see below); it never decides whether to block.

This keeps `rustpass` the testable core and avoids leaking popup/UI concepts
into the library — exactly the split the biometric plan enforced.

## Verification + pull flow (the trickiest part)

The current `git::pull_repo` fetches with `refs/heads/*:refs/heads/*` (updating
local branches in place) **then** checks out HEAD. That ordering cannot block:
by the time we could verify, the branch already moved. Enforce needs
**verify-before-checkout**. Proposed restructure:

1. **Fetch into a temp ref** (e.g. `refs/gpm/pending/<branch>`), not in place.
   This brings all new commit **objects** into the object store without moving
   the working branch — so `extract_signature` works on the fetched tip and its
   ancestors.
2. **Compute the new range** `(current HEAD, fetched tip]` via
   `repo.graph_descendant_of` + a `revwalk` between the two OIDs.
3. **Verify each commit** in the range, filter through the ignore list.
4. **Branch on mode:**
   - **Off:** fast-forward the branch to the fetched tip and check out
     (today's behaviour).
   - **Audit:** fast-forward + check out regardless; return the issue list in
     `SyncResult` so the frontend can pop the warning modal.
   - **Enforce:** if any **non-ignored blocking** issue remains, **do not move
     the branch and do not check out** — return an error with the issue list.
     HEAD and the working tree stay on the last verified state. The user must
     then trust a key, demote to Audit, or remove the offending remote.
     Otherwise: fast-forward + check out.

`SyncResult` is extended:

```rust
pub struct SyncResult {
    pub changed: bool,
    pub head: String,
    pub authenticity: AuthenticityResult, // new
}

pub struct AuthenticityResult {
    pub mode: VerifyMode,
    pub new_commits: Vec<CommitSigInfo>,   // hash, short msg, author, date, status
    pub open_issues: Vec<CommitSigInfo>,   // subset that are non-Verified and not ignored
    pub blocked: bool,                      // true only when Enforce refused checkout
}

pub struct CommitSigInfo {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,        // ISO 8601 from git2::Time
    pub subject: String,     // first line of message
    pub status: CommitSigStatus,
    pub signer_fp: Option<String>,
}
```

**Clone flow:** cloning is the trust baseline — no prior HEAD to diff against.
At clone time we do not verify history; the first _pull_ after enable is where
verification begins. (If the user enables the feature during setup, the first
explicit-trust confirmation uses the just-cloned HEAD's signer.)

**Edge — first pull after enable with no trusted key yet:** if Enforce is on but
no trusted key exists, every signed commit is `UntrustedKey` ⇒ blocks. The
enable flow must ensure a key is recorded (from HEAD's signer or manual paste)
before Enforce is selectable; the UI enforces this.

## UI design (reqs 1–6)

All additions slot into existing screens; no new global navigation paradigm.

### Settings → new "Repository Authenticity" card (req 1, 6)

```
┌─ Repository Authenticity ─────────────────────┐
│ Verification:  ( ) Off   (•) Audit   ( ) Enforce │
│                                                  │
│ Trusted signing keys (2)                         │
│  • SHA256:abcd…  "Alice — laptop"   [Remove]     │
│  • SHA256:ef01…  "CI bot"           [Remove]     │
│  [+ Add a signing public key…]                   │
│                                                  │
│ [View commit history & signatures →]             │
└──────────────────────────────────────────────────┘
```

- **Mode selector** is a 3-way radio (Off / Audit / Enforce). Switching to Audit
  or Enforce when no trusted key exists shows the explicit-trust confirm dialog
  using HEAD's current signer (or blocks Enforce and asks for a pasted key if
  HEAD is unsigned).
- **Add a signing public key** opens a paste box (`ssh-ed25519 AAAA…`); on save,
  derive the fingerprint (`ssh-key`), dedupe, store with a user label.
- **Remove** drops a trusted key (rotation / revocation). Removing the last key
  while in Enforce forces a mode downgrade to Audit (Enforce with zero trusted
  keys would block everything).
- **View commit history** links to the new `/history` screen.

This is the existing `settings-card` pattern (see `SettingsPage.vue`); the SSH
key card there is the closest analog.

### Entry list header → status indicator (req 4)

Next to the existing "Pull" button on `EntryListPage.vue`, a compact badge whose
colour + glyph reflects the _current_ authenticity state (cached after the last
pull / on mount). Tapping it opens `/history`.

| State                          | Badge   | Meaning                               |
| ------------------------------ | ------- | ------------------------------------- |
| Off / no `signing.json`        | ⚪      | Verification disabled                 |
| HEAD Verified                  | ✓ green | HEAD signed by a trusted key          |
| HEAD/last pull has open issues | ⚠ amber | Audit caught something; tap to review |
| Enforce blocked the last pull  | ⛔ red  | Refused to advance; action needed     |
| No data yet (pre-first-pull)   | — grey  | Not checked                           |

The badge reads from a new `get_authenticity_state` command (cheap, cached),
not by re-verifying on every render.

### Pull mismatch popup — Audit mode (req 3)

After a pull in Audit mode that produced `open_issues`, show a modal (not a
toast — this is a decision point):

```
┌─ Signature check ──────────────────────────────┐
│ Pulled 4 commits; 1 has a signature issue:     │
│                                                  │
│  ⚠ abc1234  "update aws/root"  Unsigned         │
│                                                  │
│ [Review in history]  [Ignore this commit]       │
└──────────────────────────────────────────────────┘
```

- **Review in history** → `/history`.
- **Ignore this commit** → records an `IgnoredIssue` (scoped), dismisses.
- Pull still succeeded (Audit never blocks); the modal is informational.
- In **Enforce**, an analogous modal appears only when the pull was _blocked_,
  explaining why HEAD didn't advance and offering: trust the signer / switch to
  Audit / cancel.

### Per-commit popup with ignore (req 2)

In `/history` (and from the Audit modal), tapping a flagged commit opens a detail
sheet:

```
┌─ abc1234 ──────────────────────────────────────┐
│ update aws/root                                  │
│ Alice <alice@example.com> · 2026-06-12           │
│                                                  │
│ Signature: ⚠ Signed by SHA256:9988… (untrusted) │
│                                                  │
│ [Trust this signer]   [Ignore this issue]        │
│ [Copy hash]                                      │
└──────────────────────────────────────────────────┘
```

- **Trust this signer** → adds the signer's key to the trusted set (TOFU
  convenience), re-evaluates.
- **Ignore this issue** → records the scoped `IgnoredIssue`. In Audit this stops
  the nag; in Enforce it downgrades _this commit only_ to a warning (except
  `BadSignature`, which is never ignorable in Enforce — the button is hidden /
  disabled with an explanation).
- Tapping a **Verified** commit shows the green ✓ and the signer label, no
  ignore action.

### Git history screen (req 5)

New route `/history` (lazy-loaded; not in the main flow):

```
┌─ Commit history ───────────────────────────────┐
│ ← Back                                           │
│                                                  │
│ ✓ def5678  add gmail          Alice · 2h ago    │
│ ✓ abc1234  update aws/root    Alice · yesterday  │
│ ⚠ 9ab00ff  rotate keys        CI bot · 3d ago    │ ← UntrustedKey
│ — 77beebb  init               Alice · 2w ago     │ ← Unsigned
│ ...                                              │
└──────────────────────────────────────────────────┘
```

- Each row: status glyph + short hash + subject + author + relative time.
- Tap a row → the per-commit detail sheet above.
- A "Re-check" button recomputes statuses (after adding/trusting a key).
- Lists the most recent N commits (e.g. 50) via `git2` revwalk from HEAD; "load
  more" optional, deferred.

## New Tauri commands (summary)

Mirroring the existing command style in `src-tauri/src/lib.rs`:

```text
get_authenticity_state() -> AuthenticityState          // for the indicator badge
set_verification_mode(Mode) -> ()                      // req 1 mode selector
get_authenticity_config() -> AuthenticityConfigView     // settings card (no secrets)
add_trusted_key(public_key, label) -> TrustedKeyView    // req 6
remove_trusted_key(fingerprint) -> ()                   // req 6
trust_head_signer(label) -> TrustedKeyView              // TOFU from settings/history
ignore_commit_issue(hash) -> ()                         // req 2
list_commit_signatures(limit, after?) -> Vec<CommitSigInfoView>  // req 5
get_commit_signature(hash) -> CommitSigDetailView       // req 2/5 detail
```

`pull_repo`'s existing signature is unchanged; its `SyncResult` gains the
`authenticity` field, which the frontend already destructures.

`rustpass/src/signing.rs` (new) exposes the pure operations:
`extract_signature(repo, oid)`, `classify(signature)`, `verify_ssh(...)`,
`status_of_commit(...)`, `verify_range(repo, from, to, trusted, ignored)`. The
`Store` gains thin async wrappers (with `spawn_blocking` for git2/verify work,
matching how crypto/git already run off the async thread).

## Task breakdown

Phased so each slice is independently shippable and testable. Each rustpass
slice ships with in-module + integration tests (matching the repo convention).

- **A1 — `rustpass::signing` core (no policy).** Signature extraction,
  classification, SSH verify, single-commit status. Unit tests with a fixture
  SSH-signed commit (generate one in-test with `ssh-key` + craft a commit via
  `git2::commit_signed`). Confirms the feature-flag question empirically.
- **A2 — `Store::verify_range` + authenticity persistence.** `signing.json`
  load/save (atomic, like `save_identity_raw`), `verify_range(from,to)`,
  `head_signature_status()`, ignore filtering. Integration tests.
- **A3 — Verify-before-checkout in `git::pull_repo`.** Temp-ref fetch, range
  verify, mode-driven branch/checkout. The riskiest slice; test the Enforce
  abort path carefully (assert HEAD and worktree unchanged on block).
- **B1 — Tauri commands + `SyncResult.authenticity`.** Wire A1–A3 to IPC types;
  add the commands listed above.
- **B2 — Settings "Repository Authenticity" card.** Mode selector, trusted-key
  add/remove, paste box, fingerprint display.
- **B3 — Entry-list indicator badge + `get_authenticity_state`.**
- **B4 — Audit-mode mismatch modal + Enforce-block modal.**
- **B5 — `/history` screen + per-commit detail sheet + ignore/trust actions.**
- **C1 — Docs.** Update `docs/SECURITY.md` (new "Repository authenticity" section
  - move this from a Known Limitation to a Mitigation), `CHANGELOG.md`
    (user-facing), and `CLAUDE.md` architecture section.

Suggested order: A1 → A2 → B2 (lets you exercise the core end-to-end in Audit
without the risky pull change) → A3 → B1 → B3 → B4 → B5 → C1. A3 can land last
among backend slices if we want the verify logic field-tested before making it
block pulls.

## Risks and open questions

1. **`git::pull_repo` restructure (A3) is the main risk.** Switching from
   in-place refspec fetch to temp-ref-then-conditional-ff changes a load-bearing,
   already-tested path. Mitigation: keep the Off-mode path byte-for-byte
   equivalent to today (same fetch + checkout, no temp ref) so the common case
   is untouched; only Audit/Enforce take the new path. Full regression of the
   existing `git.rs` tests + new Enforce-abort tests.
2. **ssh-key feature flags.** Confirm `SshSig` parse + `PublicKey::verify` work
   with the current feature set (expected: yes for ed25519). If a flag is
   needed, add it to the workspace dep — no new crate.
3. **First-run UX in Enforce.** Enforce with zero trusted keys blocks everything.
   The UI must prevent selecting Enforce until a key is recorded (from HEAD's
   signer or a paste). Otherwise the app appears "broken" on pull.
4. **Mixed signing.** A repo might have some commits SSH-signed and some GPG or
   unsigned (e.g. an unsigned merge, or a pre-signing-policy era). In Enforce
   that blocks. The escape hatches are Audit + per-commit ignore; but document
   clearly that Enforce assumes a consistently SSH-signed history. Consider an
   "ignore all unsigned commits at-or-before X" convenience (deferred — the
   per-commit ignore is enough for v1).
5. **`gpgsig` for SSH is stored under the same header as GPG.** Classification by
   armor prefix (`-----BEGIN SSH SIGNATURE-----` vs `-----BEGIN PGP SIGNATURE-----`)
   is reliable; add a test for both armor shapes so a GPG-signed repo classifies
   as `UnsupportedFormat`, not `BadSignature`.
6. **TOFU trust of HEAD at enable time** assumes the _current_ HEAD isn't already
   an attacker commit. This is the irreducible "first-use" assumption; the
   explicit-confirm dialog is the mitigation, and the `/history` audit path is
   the escape hatch for a paranoid user. Worth calling out in `SECURITY.md`.

## Non-goals

- **GPG/PGP-signed commits.** SSH-only for v1 (see Decision 1). The status enum
  reserves `UnsupportedFormat` so GPG commits are visibly "signed but not
  verifiable by gpm" rather than silently `Unsigned`.
- **Signing commits from gpm.** gpm is read-only; signing happens on the desktop
  gopass side. gpm only _verifies_.
- **Trusted-key sync across devices.** The trusted set is per-device local state
  (like the decryption identity). Multi-device sync is a separate problem.
- **A remote-host pinning / TOFU-on-transport layer.** That's transport trust
  (HTTPS/SSH), orthogonal to commit-signature authenticity. Out of scope here.

## Depends on

None strictly. Nicely complements [0006-multi-identity.md](./0006-multi-identity.md)
(both are "trust the store" features) but ships independently. P3 priority: ship
after core stability; the Audit mode alone (no pull-blocking) is low-risk enough
to land early if desired.

## Effort

~2–3 days (human) / ~1 hour (CC) for design + implementation, per the stub's
estimate — and that estimate holds **because** the verification primitives are
already dependencies. The risk-weighted slice is A3 (pull restructure); the rest
is mechanical plumbing on confirmed APIs.
