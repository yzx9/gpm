# Security Model

gpm is a local, read-only password viewer. It clones an age-encrypted gopass
repository to the device, decrypts entries on demand, and copies passwords to
the clipboard. No editing, no GPG, no cloud sync.

## Threat Model

gpm defends against **local opportunistic access** — someone who briefly has
physical access to an unlocked device, or a malicious app that somehow injects
script into the WebView. It does **not** defend against a fully compromised OS
or a determined attacker with root access.

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

## Security Measures

| Measure                 | Detail                                                                       |
| ----------------------- | ---------------------------------------------------------------------------- |
| Zeroizing memory        | Rust `Secret` type wraps `Zeroizing<String>`; content wiped on drop          |
| Safe Debug output       | Custom `Debug` impl shows `[REDACTED]`, never actual secrets                 |
| Clipboard isolation     | `copy_password` keeps password in Rust; JS receives only metadata            |
| Lifecycle cleanup       | Vue refs cleared on timer, navigation, and unmount                           |
| Screen capture block    | Android `FLAG_SECURE` prevents screenshots and screen recording              |
| Error sanitization      | Error messages contain only codes and generic descriptions                   |
| Path traversal guard    | Resolved paths validated to stay within repository; symlink escape detection |
| Content Security Policy | CSP restricts `script-src`, `connect-src` to `self` and IPC only             |
| Commit signature verify | Optional SSH-signed-commit verification on every pull (see below)            |

## Repository authenticity

`age` guarantees **confidentiality** but not **authenticity** of the store
history. A successful `git pull` only proves you received a valid git object
graph — not that it came from someone you trust. An attacker who controls the
remote can feed age blobs that decrypt fine but contain data they also know
(e.g. a new `aws/root.age` with a password they chose).

To close this, gpm offers optional **SSH-signed commit verification** (git ≥
2.34 `gpg.format = ssh`, verified against a user-managed trusted-signing-key
set). It is a tri-state per-repo setting:

- **Off** — no verification (the default).
- **Audit** — verify every pulled commit; warn on a mismatch, always pull.
- **Enforce** — verify every pulled commit; a non-ignored blocking issue
  aborts the pull, leaving HEAD and the working tree on the last verified
  state.

On each pull every commit in the range `(old HEAD, new HEAD]` is verified (not
just the tip — a buried malicious commit behind a signed tip is still caught).
Verification reuses the already-present `ssh-key` crate; **no new crypto
dependency**. GPG-signed commits are out of scope and surface as "signed but
not verifiable by gpm". The trusted-signing-key set is public, non-secret
data; it lives as the `authenticity` field of `repo.json`.

**Defeats** (Enforce; detects in Audit): a compromised remote feeding unsigned
or attacker-signed commits, or tampering with a signed commit's contents (any
edit invalidates the SSH signature → `BadSignature`).

**Does not defeat**: the signing key itself being compromised (rotation/
revocation is the countermeasure); a malicious commit made before the feature
was enabled (verification is forward-looking — use the History screen to audit
the past); transport-level spoofing (handled by HTTPS/SSH transport trust).

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

## Approaches Not Adopted

| Approach                        | Why not                                                                                                                                                                                                                  |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Tauri Isolation Pattern         | Encrypts the **frontend → Rust** IPC direction (protects against malicious frontend calling Rust commands). Does **not** encrypt the Rust → frontend response. CSP is a more direct defense for our threat model.        |
| Custom IPC encryption layer     | Both ends run in the same process — the decryption key would also be in the same process. This is security theater.                                                                                                      |
| Canvas-based password rendering | Would avoid DOM text nodes, but Android accessibility services can OCR rendered content. Extreme complexity for marginal gain.                                                                                           |
| JavaScript memory overwriting   | V8 strings are immutable. `result.password = "\x00".repeat(...)` creates a **new** string and reassigns the reference — the original password string remains on the heap until GC. Doing this would be security theater. |
