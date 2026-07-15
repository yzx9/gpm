# Security Model

gpm is a local, read-only password viewer. It clones an age-encrypted gopass
repository to the device, decrypts entries on demand, and copies passwords to
the clipboard. No editing, no GPG, no cloud sync.

## Threat Model

gpm defends against **local opportunistic access** — someone who briefly has
physical access to an unlocked device, or a malicious app that somehow injects
script into the WebView. It does **not** defend against a fully compromised OS
or a determined attacker with root access.

### Auto-lock

gpm auto-locks the in-memory identity — it is wiped on inactivity rather than
left sitting in memory, so secrets are not exposed to anyone who later picks up
an unlocked device. This is a core security property. (The lock modes, timeouts,
and what counts as activity are implementation details; see the code.)

### Local private files

gpm's private files — the age/SSH `identity` and `repo.json` (which carries the
git credentials and the `authenticity` trust set) — are sensitive. What is
protected, and against whom, differs by threat:

**Defended on Android (at-rest encryption).** `identity` and `repo.json` are
encrypted at rest with a master key sealed in the Android Keystore
(hardware-backed, auth-free AES-GCM). An attacker who can _read_ the app's
private storage — a stolen backup, a forensic dump, a non-root malicious app
with storage access — gets ciphertext, not the git credentials or the trust
set. The same authenticated encryption also gives these files **integrity**: a
modified `repo.json` (flipping the verification mode, injecting an attacker
signing key) or a swapped `identity` fails the authentication tag and is
rejected rather than silently accepted.

**Still assumed, not solved by at-rest encryption.** gpm continues to assume
that **no local attacker has write access** to the app's private storage. In
particular:

- A write attacker can still tamper with the cloned `repo/` (the working tree,
  `.git` objects, the recipients file) between operations. The repository
  authenticity feature verifies commit signatures on `pull` (remote→local), not
  local working-tree tampering; defending that would require a sealed snapshot
  over the working tree, which is not implemented.
- A write attacker with an older, pre-encryption backup could roll a file back
  to plaintext; authenticated encryption prevents _forging_ a new ciphertext
  but not a rollback.

On Android the no-write assumption rests on the app sandbox; on desktop there
is no Keystore equivalent, so the files stay plaintext and the assumption rests
on the user account not being compromised.

The at-rest master key lives in app memory for the session. This is no more
sensitive than the git credentials gpm already holds in memory while cloning or
syncing, and is consistent with the non-goal of not defending against a fully
compromised OS or a process running as the app (which could ask the Keystore to
unseal the key regardless). If the Keystore key is lost (app data cleared,
Keystore wiped, factory reset) the encrypted files become unreadable and
re-setup is required; there is no escrow, since any escrow key stored on disk
would defeat the purpose.

### App-launch biometric gate (opt-in)

The optional **App Lock** (Settings → App Lock, RFC 0028) raises the at-rest
defense into a real lock screen. When on, the master key is re-sealed behind a
**biometric-gated** Keystore key (still hardware-backed AES-GCM, but every use
requires a STRONG biometric). The store is then unreadable — on disk _and_ in
memory — until the user authenticates: gpm builds without the master key at
launch, injects it only after the app-unlock biometric prompt, and **wipes it
when the app returns to the foreground** (detected via the WebView's
`visibilitychange`), so a locked app cannot read the store even from a memory
snapshot. One biometric prompt gates the whole store; the identity `UnlockModal`
is suppressed while the app-lock overlay is up so the two never race.

The wipe happens on _resume_ (foreground return), not at the instant of
backgrounding: while the app sits in the background, before the resume fires, the
master key remains in memory until the re-lock runs. This is consistent with the
threat model (a process-running attacker is an explicit non-goal), but the
guarantee rests on the WebView firing `visibilitychange` on resume, which is the
norm on Android but not contractually guaranteed on every OEM build. If that
event ever failed to fire on a resume, the key would stay loaded until the next
one that does. A Kotlin-side `Activity.onResume` hook would make the signal
authoritative; it is tracked as a future hardening (RFC 0029) rather than
shipped here.

This binds at-rest encryption to biometrics for users who opt in, and is a
deliberate departure from the default auth-free master key — adopted only here,
where the user accepts the tradeoff:

- **Enrollment does not brick.** The biometric-gated master key is _not_
  invalidated by enrolling a new fingerprint or face, so adding a finger never
  locks you out of your store.
- **Removing all biometrics does.** If every enrolled biometric is removed, the
  key is invalidated and the store becomes unreadable — re-setup (re-clone,
  re-enter the git token) is the only recovery. There is no escrow; an escrow
  key on disk would defeat the lock. This is the accepted residual risk of the
  opt-in feature.
- **One prompt, not two.** A second toggle, _Identity Auto-Unlock_ (off by
  default, and separate from the Auto-Lock timing presets), seals the identity
  passphrase under the master key. When it is on, a successful app-unlock also
  unlocks the identity session with no second prompt; when off, the identity
  keeps its existing per-operation/session behavior.

The gate re-challenges on every return to the foreground (cold start and warm
resume alike). On desktop there is no Keystore equivalent, so App Lock is
unavailable and the files stay plaintext (the existing asymmetry).

## Two Password Operation Paths

### `copy_password` — primary operation (no IPC exposure)

The password is decrypted in Rust, written directly to the system clipboard,
and **never crosses the IPC boundary** to the WebView. Only a metadata response
(`CopyResult { success, entry_name, cleared_after_secs }`) is returned to
JavaScript.

The clipboard is automatically cleared after 30 seconds via a Tokio background
task.

### `show_password` — secondary operation (intentional IPC exposure)

The password is decrypted in Rust and returned to the WebView as
`SensitiveContent { password, notes }` for display. **This is the inherent
cost of rendering text on screen** — if you must display it, it must exist in
the DOM.

Mitigations:

- 30-second auto-clear timer
- Cleanup on navigation (`popstate`), component unmount (`onBeforeUnmount`),
  and manual dismiss
- Password is never logged or persisted to storage

### `copy_totp` — 2FA code path (no IPC exposure of the seed or code)

When an entry stores a TOTP seed (a `totp:` line or an `otpauth://` link in its
notes — the format gopass uses), gpm extracts the seed, computes the current
one-time code in Rust, and writes the code directly to the clipboard. **On this
path neither the seed nor the code crosses the IPC boundary** — only a small
result (`TotpCopyResult { copied, entry_name, cleared_after_secs }`) returns to
the UI. This is **strictly safer than using _Show_ for the same goal**: the copy
path never exposes the password, notes, or seed to the WebView, and it rides the
same clipboard auto-clear and sticky notification as `copy_password`.

**What this does _not_ fix:** tapping _Show_ on a seed-bearing entry still sends
the full notes — including the seed line — to the WebView, exactly as before. We
deliberately do **not** redact `totp:`/`otpauth:` lines out of the revealed
notes: the editor loads a secret through the same reveal path, so redaction would
overwrite the seed with a placeholder on save and destroy it (and would diverge
from gopass's `show`). Treat the seed as sensitive on the reveal path too —
prefer _Copy 2FA_. A malformed or unsupported seed (HOTP, Steam Guard, an unknown
algorithm, or a zero period) surfaces as a clear error rather than a silently
wrong code.

## Security Measures

| Measure                  | Detail                                                                                                                                                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Zeroizing memory         | Rust `Secret` type wraps `Zeroizing<String>`; content wiped on drop                                                                                                                                     |
| Safe Debug output        | Custom `Debug` impl shows `[REDACTED]`, never actual secrets                                                                                                                                            |
| Clipboard isolation      | `copy_password` keeps password in Rust; JS receives only metadata                                                                                                                                       |
| Lifecycle cleanup        | Vue refs cleared on timer, navigation, and unmount                                                                                                                                                      |
| Screen capture block     | Per-route Android `FLAG_SECURE` (user toggle); see Screen capture protection                                                                                                                            |
| Error sanitization       | Error messages contain only codes and generic descriptions                                                                                                                                              |
| Path traversal guard     | Resolved paths validated to stay within repository; symlink escape detection                                                                                                                            |
| Content Security Policy  | CSP restricts `script-src`, `connect-src` to `self` and IPC only                                                                                                                                        |
| WebView script integrity | Secrets render as text (Vue-escaped), never as executable HTML (`v-html`/`innerHTML`); the only script in the WebView is the app's own bundle, which is why the frontend may supply native-prompt text. |
| Commit signature verify  | Optional SSH-signed-commit verification on every pull (see below)                                                                                                                                       |

## Screen capture protection

On Android, gpm sets `WindowManager.FLAG_SECURE` on the activity window for
**secret-bearing pages** — setup, create, generate, entry detail, and settings
(settings renders the SSH private key on export). `FLAG_SECURE` blanks
screenshots, screen recording, and the Recents/task-switcher thumbnail for that
window. The entry list (entry _names_ only) and history (commit signatures)
carry no secret content and stay capturable.

This is **per-route**, gated by a user **master toggle** (Settings, default on):
`secure = toggle && route.secret-bearing`. With the toggle off, no page is
secured. The flag is applied in the navigation guard _before_ the target page
paints, and `MainActivity` sets it on at boot as a safe default until the
frontend reconciles — so a secret-bearing page is never shown unprotected. A
guard that cannot confirm the flag on a secret-bearing route aborts the
navigation and toasts, rather than render it unprotected.

Caveats: `FLAG_SECURE` is Android-only (desktop has no equivalent); it is
bypassable on rooted devices (e.g. Magisk "Disable Flag Secure"); and the
non-secret list/history pages are capturable by design. The toggle is a
device-level preference stored in `app.json` and intentionally survives a repo
reset. Component-level granularity (securing just a reveal action on an
otherwise-capturable page) is deferred — see RFC 0028.

## Repository authenticity

`age` guarantees **confidentiality** but not **authenticity** of the store
history. A successful `git pull` only proves you received a valid git object
graph — not that it came from someone you trust. An attacker who controls the
remote can feed age blobs that decrypt fine but contain data they also know
(e.g. a new `aws/root.age` with a password they chose).

To close this, gpm offers optional **commit signature verification** for both
formats git supports — **SSH-signed** commits (git ≥ 2.34 `gpg.format = ssh`,
verified with the already-present `ssh-key` crate) and **GPG/OpenPGP-signed**
commits (verified with the pure-Rust `rpgp` crate, the same dependency class
as `age`/`ssh-key` — no C, `ring`, or `openssl`). Both are checked against a
user-managed trusted-signing-key set. It is a tri-state per-repo setting:

- **Off** — no verification (the default).
- **Audit** — verify every pulled commit; warn on a mismatch, always pull.
- **Enforce** — verify every pulled commit; a non-ignored blocking issue
  aborts the pull, leaving HEAD and the working tree on the last verified
  state.

On each pull every commit in the range `(old HEAD, new HEAD]` is verified (not
just the tip — a buried malicious commit behind a signed tip is still caught).
The trusted-signing-key set is public, non-secret data; it lives as the
`authenticity` field of `repo.json`.

**Trust is set membership, not web-of-trust.** gpm does a simple "is this key
in the trusted set?" check — it ignores GPG owner-trust, certification levels,
and keyserver lookups entirely, so no new network trust vector is introduced.
You add a trusted signer by pasting its public key (or importing a `.asc`
file). For GPG the trusted identity is the primary-key fingerprint, and a
subkey signature verifies against the trusted primary via its binding
signature.

**Expiry and revocation are NOT enforced.** A GPG key past its expiry date, or
one carrying a revocation signature, still verifies in this phase (revocation
is not even parsed for policy). Treat the trust set as "the keys I have
chosen to trust", not "keys currently valid by GPG's own rules."

**SSH-sig and GPG make different guarantees about an untrusted signer.** An
SSH signature embeds the signer's public key, so gpm always verifies the
cryptography and only the trust decision remains — an untrusted SSH signer
surfaces as `UntrustedKey` (crypto-verified, just not in your trust set). A
GPG signature carries only the issuer fingerprint, never the key, so when the
signer is not trusted gpm has no key to check against and performs **no
cryptographic verification** — that surfaces as a distinct
`UnverifiedSignature` status, a weaker statement than `UntrustedKey`. The
difference is visible to the user, not hidden behind one label.

**Defeats** (Enforce; detects in Audit): a compromised remote feeding unsigned
or attacker-signed commits, or tampering with a signed commit's contents (any
edit invalidates the signature → `BadSignature`).

**Does not defeat**: the signing key itself being compromised (rotation/
revocation is the countermeasure — and see above, gpm does not yet honor
revocation); a malicious commit made before the feature was enabled
(verification is forward-looking — use the History screen to audit the past);
transport-level spoofing (handled by HTTPS/SSH transport trust).

**Irreducible first-use assumption:** trusting the current HEAD's signer at
enable time assumes that HEAD isn't already an attacker commit. The explicit
confirm step is the mitigation; the History screen is the escape hatch for a
paranoid user.

## Known Limitations

### Encrypted SSH private keys as age identities

gpm accepts SSH private keys (`ssh-ed25519`, `ssh-rsa`) as age identities for decryption, but does **not** support passphrase-encrypted SSH keys. Users with encrypted keys must provide an unencrypted key or convert their key. This is a deliberate scope limitation — passphrase support may be added in a future release.

### JavaScript memory persistence

Setting `password.value = null` clears the Vue ref but does **not** zero the
underlying V8 string. JavaScript strings are immutable — even overwriting the
IPC response object (`result.password = ...`) only changes the reference, not
the original heap memory. The plaintext may persist until garbage collection.

This is a fundamental limitation of the WebView runtime, not a bug. There is no
reliable way to deterministically zero JavaScript string memory.

### `show_password` plaintext in IPC

The `SensitiveContent` response crosses the Rust → WebView IPC boundary as
plaintext JSON. This is **by design**: the password must be displayed. Tauri v2's
IPC is process-local (`ipc:` / `http://ipc.localhost` custom protocol, or the
Android JNI bridge). It does not traverse any network socket.

### Android accessibility services

When the password is displayed, it exists as a text node in the DOM. Android
accessibility services can read it. This is inherent to displaying text in a
WebView — there is no reliable way to show text on screen while hiding it from
accessibility services.

### `select-all` on password display

The password display element uses `select-all` CSS to allow users to manually
select and copy the password. On mobile, this may interact with the system
clipboard in unexpected ways. The primary copy mechanism should be the
"Copy Password" button, which avoids this entirely.

### TOTP seed storage is not true two-factor authentication

gpm can store a TOTP seed in an entry and copy the current code — convenient and
gopass-compatible — but this is **two-step verification, not a true second
factor.** TOTP's protection rests on the seed being a separate thing you _have_.
When the password and the seed live in the same gpm vault on the same device, a
single compromise of that device or vault exposes both at once, collapsing two
factors into one.

What it **still defends against**: an attacker who only has your password — for
example from another site's breach used in credential stuffing — still cannot
generate the code, because the seed stays on your device. So storing the seed in
gpm retains real value against the most common account-takeover vector.

What it **does not defend against**: anyone who gains access to your unlocked
gpm gets both the password and the ability to produce codes.

**Recommendation.** For high-value accounts — and especially for the email or
account that protects your gpm vault and its git remote — keep the TOTP seed on a
**separate device or a hardware security key**, not in gpm. For routine accounts,
storing the code in gpm is a reasonable convenience tradeoff. **Never** store the
TOTP seed for the account that protects access to your vault (your git host or
recovery email) inside that same vault — losing that one device must not lock you
out of everything.

gpm also enforces a stricter seed floor than gopass: a TOTP secret must be at
least 128 bits (16 bytes), or gpm refuses to parse it. gopass accepts shorter
seeds, so a seed gopass honors may need lengthening before gpm will generate its
code.

## Approaches Not Adopted

| Approach                        | Why not                                                                                                                                                                                                                  |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Tauri Isolation Pattern         | Encrypts the **frontend → Rust** IPC direction (protects against malicious frontend calling Rust commands). Does **not** encrypt the Rust → frontend response. CSP is a more direct defense for our threat model.        |
| Custom IPC encryption layer     | Both ends run in the same process — the decryption key would also be in the same process. This is security theater.                                                                                                      |
| Canvas-based password rendering | Would avoid DOM text nodes, but Android accessibility services can OCR rendered content. Extreme complexity for marginal gain.                                                                                           |
| JavaScript memory overwriting   | V8 strings are immutable. `result.password = "\x00".repeat(...)` creates a **new** string and reassigns the reference — the original password string remains on the heap until GC. Doing this would be security theater. |
