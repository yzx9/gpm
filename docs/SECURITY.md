# Security Model

gpm is a local, read-only password viewer. It clones an age-encrypted gopass
repository to the device, decrypts entries on demand, and copies passwords to
the clipboard. No editing, no GPG, no cloud sync.

## Threat Model

gpm defends against **local opportunistic access** — someone who briefly has
physical access to an unlocked device, or a malicious app that somehow injects
script into the WebView. It does **not** defend against a fully compromised OS
or a determined attacker with root access.

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
