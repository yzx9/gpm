# Keystore + biometric unlock

**Priority:** P2
**Status:** In Progress (plan revised via `/plan-eng-review` + codex outside voice)
**Phase:** Post-MVP (v1.1)

## What

Store the identity **passphrase** in the Android Keystore with hardware-backed
encryption, and use biometric authentication (fingerprint/face) to retrieve it.
Users with a passphrase-protected identity can unlock gpm with biometrics
instead of typing the passphrase on every launch.

Applies to **both passphrase-having identity types**:

| #   | Identity type                      | Biometric applies?  |
| --- | ---------------------------------- | ------------------- |
| 1   | age (x25519), no passphrase        | ❌ no unlock needed |
| 2   | age (x25519), passphrase-encrypted | ✅                  |
| 3   | SSH, no passphrase                 | ❌ no unlock needed |
| 4   | SSH, passphrase-protected          | ✅                  |

The first and third types never need unlocking, so biometric is moot. This plan
covers types 2 and 4.

**This does not change how gpm/gopass encrypts anything.** It only adds a
second way to _obtain_ the existing passphrase (KeyStore + biometric). The
retrieved passphrase is fed to the existing `Store::unlock` path exactly like a
typed passphrase.

**Unlock UX:** biometric lands on the **existing `/unlock` page** — no route or
modal changes in this plan. `UnlockPage.vue` gains a "Unlock with biometric"
button and auto-prompts on mount when biometric is enabled and available. The
modal-overlay refactor (stay-on-current-page on auto-lock) is a separate,
deferred plan: [0009-unlock-modal.md](./0009-unlock-modal.md).

Android-only, **API 30+ (Android 11+)** for biometric. Devices below API 30 and
desktop see passphrase-only (biometric reports unavailable). iOS deferred.

## Why

A passphrase-protected identity requires typing the passphrase on every app
launch — the single highest-frequency friction point for daily use. Biometric
lets users skip password entry while maintaining real security: the passphrase
is protected by hardware-backed, biometric-gated storage.

**Key principle:** biometric produces the passphrase and runs through the
_same_ `Store::unlock()` + `reset_lock_timer()` path as the password UI —
whatever the password flow does, biometric mirrors. The retrieved passphrase
flows **Kotlin → Rust → `store.unlock`** and never enters the WebView/JS
(preserving gpm's "secrets never reach the WebView" model).

## Layering (the central design decision)

Two rules, agreed in review:

1. **Biometric lives entirely in the Tauri app layer (`src-tauri/`).** The core
   library `rustpass/` learns nothing about biometric — no `BiometricConfig`, no
   `Biometric*` error codes. Biometric is a platform/app concern (Android
   Keystore + a UI flag), not a password-store concern. The only `rustpass/`
   changes in this plan are (a) a **prerequisite bug fix** to its own
   `is_unlocked()` state tracking and (b) the shared `unlock_and_arm` helper
   extraction (see Implementation). Neither is biometric logic.

2. **The Store owns the locked/unlocked state.** The state is the Store's two
   caches (`cached_identity`, `cached_passphrase`); the app layer does **not**
   duplicate it into an `authenticated` flag. The `/unlock` page asks the Store
   (via `get_auth_state` + the existing `identity-locked` event) whether it is
   locked — it does not hold a second source of truth. (The frontend keeps a
   reactive `locked` ref for rendering, but that is the frontend's _view of_ the
   Store, mirrored from `get_auth_state` + events, not an independent state.)

### Why the Store holds the state, and the prerequisite fix

`Store` already has `lock()` / `unlock()` and tracks state via its caches:

- `cached_identity` — the decrypted age identity (x25519 plaintext key). Populated
  by `unlock()` for age-encrypted identities. Decryption of an age entry uses
  only this (the passphrase is no longer needed).
- `cached_passphrase` — the raw passphrase string. **Required for SSH
  identities**: age re-decrypts the SSH key with the passphrase on _every_ entry
  decryption (`crypto.rs` SSH branch, `enc.decrypt(passphrase)` at `crypto.rs:99`),
  so there is no "decrypted identity" to cache for SSH — the cached passphrase
  _is_ the unlock state.

`is_unlocked()` (`rustpass/src/store.rs:124`) currently checks only
`cached_identity`:

```rust
// CURRENT (buggy for SSH):
pub fn is_unlocked(&self) -> bool {
    self.cached_identity.read().is_ok_and(|g| g.is_some())
}
```

This was correct before `48f5d7c` (when SSH keys were age-encrypted and thus
populated `cached_identity`). After that refactor stopped age-encrypting SSH
keys, SSH `unlock()` only fills `cached_passphrase`, so `is_unlocked()` stays
`false` forever — and the lock UI (`encrypted && !unlocked`) traps an SSH
identity on the unlock screen **whether or not biometric exists**. This is a
pre-existing regression, independent of this feature.

**Prerequisite fix** (small, lands first, can be its own commit/PR):

```rust
pub fn is_unlocked(&self) -> bool {
    self.cached_identity.read().is_ok_and(|g| g.is_some())
        || self.cached_passphrase.read().is_ok_and(|g| g.is_some())
}
```

Result: age → `cached_identity` set → `true`; SSH → `cached_passphrase` set →
`true` (fixed); plaintext → `unlock()` is never called in practice → stays
`false` (harmless; the lock condition is `encrypted && !unlocked` and plaintext
has `encrypted=false`).

Caveat: the existing test `unlock_caches_passphrase_for_plaintext_identity`
asserts `!is_unlocked()` after calling `unlock()` on a plaintext identity; that
assertion now flips (or `unlock()` stops caching the passphrase for plaintext).
Either is fine — handle it during implementation. **A new regression test is
mandatory** (see Test review): encrypted SSH identity → `unlock()` →
`is_unlocked()` is `true`.

### "Enabled" signal — no flag file

There is **no separate `biometric.json` flag**. "Is biometric enabled?" is
defined as **"a passphrase is stored in the Keystore"** (`plugin.has_stored()` —
a non-prompting read of the stored ciphertext state). This collapses the
flag/Keystore desync into one source of truth: the stored ciphertext itself.

Note (codex Finding 5, accepted as bounded): `has_stored()` only sees the
ciphertext in prefs; it does not verify the Keystore key is still live or that a
biometric is still enrolled. So "enabled" can read true while the key is dead,
until the next `retrieve` attempt detects `KeyPermanentlyInvalidatedException`
and self-heals (delete + reveal form). This lazy self-healing is acceptable; an
optional sharpen (also check `keyStore.getEntry(alias, null)` and
`BiometricManager.canAuthenticate(STRONG)` in `has_stored`) is filed as a TODO.

## Build vs. buy (why custom, not a third-party plugin)

The third-party landscape was evaluated for "store a secret in Android Keystore,
retrieve it biometric-gated, from Rust":

| Plugin                               | Encrypted storage | Biometric-gated                                                      | Verdict                                                                                                                   |
| ------------------------------------ | ----------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `tauri-plugin-biometric` (official)  | ❌ prompt-only    | prompt-only                                                          | Doesn't store secrets                                                                                                     |
| `tauri-plugin-stronghold` (official) | ✅                | ❌ no biometric, no Keystore HW backing; overkill for one passphrase | Wrong fit                                                                                                                 |
| `tauri-plugin-keystore` (impierce)   | ✅                | ✅ correct `BiometricPrompt` CryptoObject                            | **Closest** — but alpha (`v2.1.0-alpha.1`, default branch `alpha`), ~4 stars, last real work Feb 2025, single contributor |
| `tauri-plugin-biometry` (community)  | ✅                | ✅                                                                   | Feature-complete but individual-maintainer, not battle-tested                                                             |

**Decision: build custom.** Depending on an alpha, single-contributor crate for
the _security-critical secret-storage path_ of a product whose value prop is
trust is the wrong trade. None of the official options do biometric-gated
storage.

**But don't write the Kotlin crypto blind:** study and adapt
`impierce/tauri-plugin-keystore`'s proven `BiometricPrompt` CryptoObject +
AndroidKeyStore GCM implementation as the reference for `KeystorePlugin.kt`.
This captures the benefit of a reviewed pattern without taking on the alpha
dependency. (License: Apache-2.0 — compatible; attribution retained.)

## Architecture

```
Daily path (high frequency, the whole point):
  App launch / auto-lock fires  →  router pushes /unlock (existing behavior)
    UnlockPage onMounted: isBiometricUnlockEnabled() && isBiometricAvailable()
      → invoke("biometric_unlock")
          → app command: plugin.retrieve()        [run_mobile_plugin_ASYNC, backend-only]
              → BiometricPrompt (CryptoObject DECRYPT) → cipher.doFinal → passphrase bytes
          → unlock_and_arm(state, app, &pw)        [shared helper, DRY]
          → router.push("/entries")
    else: passphrase form shown; "Unlock with biometric" button if available

Enable path (typed passphrase, then ONE biometric prompt to seal it — see D2):
  SettingsPage → user types passphrase → invoke("enable_biometric_unlock", {passphrase})
    → validate first:
        age   → store.unlock(pw)            (WrongPassphrase rejects)
        SSH   → ssh::Identity::from_buffer(buf, Some(pw)).decrypt()  (D4: rejects typos)
    → plugin.store(pw)   [BiometricPrompt CryptoObject ENCRYPT → AES/GCM → prefs]

Disable / passphrase-change / invalidation → plugin.delete()  (clears stored passphrase)
```

The plugin is a **backend primitive only** — the frontend never calls
`plugin:keystore|*` directly. All access is through five app commands in
`src-tauri/src/lib.rs`, keeping the secret-returning operations off the WebView.

### Security chain

```
/unlock page (foreground) → biometric prompt
  → Android Keystore unlocks (hardware-backed, biometric-gated via CryptoObject)
  → Passphrase retrieved (ByteArray, zeroed after use)
  → run_mobile_plugin_async returns to Rust (never JS), wrapped in Zeroizing<String>
  → Store::unlock(passphrase)  → identity cache populated, auto-lock timer armed
```

### The plugin pattern (backend-callable, not frontend-only; and async)

`gpm-plugin-safe-area/` is a **frontend-only** plugin (Rust `init()` just
registers the Kotlin class; the frontend calls `plugin:safe-area|*` directly).
The Keystore plugin needs the **opposite** shape: Rust app code calling Kotlin
via the `run_mobile_plugin` pattern on a `PluginHandle` (cf.
`tauri-plugin-haptics`). This is the canonical Tauri v2 pattern (confirmed
against the official Mobile Plugin docs). So "mirror safe-area" covers only the
crate _scaffolding_ (`Cargo.toml`, `build.rs`, permissions, registration); the
`src/lib.rs` Rust shim is a different pattern and is validated in Phase 3 before
any Kotlin crypto is written.

**Hard requirement (codex Finding 7):** `retrieve()` and `store()` hold the
`Invoke` across a multi-second user prompt. They **MUST** use
`run_mobile_plugin_async` (the async variant), not the blocking
`run_mobile_plugin`. Wrapping the prompt on the blocking helper pins a Rust
worker thread for the whole biometric interaction. `KeystoreExt::retrieve()` and
`::store()` are therefore `async` and the app commands `.await` them (matching
the `app.keystore()?.retrieve().await` shape). The fast ops (`is_available`,
`has_stored`, `delete`) may use either.

Watch-item: blocking work in plugin callbacks can freeze the Android UI
(tauri#14694). `retrieve`/`store` are inherently async (user prompt, resolves
from callbacks); the AES-GCM op itself is fast. Neither blocks the main thread.

## Implementation

### Prerequisite — `rustpass/` (lands first)

- **`store.rs`** — fix `is_unlocked()` as shown above (`cached_identity ||
cached_passphrase`). Add a **regression test**: encrypted SSH identity →
  `unlock()` → `is_unlocked()` is `true`. Update the affected
  `unlock_caches_passphrase_for_plaintext_identity` test. `just test`.

No other `rustpass/` change in this feature except the shared helper below.

### Shared helper — `src-tauri/src/lib.rs` (DRY, D5)

Extract `fn unlock_and_arm(state, app, pw) -> Result<(), Error>` that does
`state.store.unlock(&pw).await?; reset_lock_timer(&state, &app);`. Both the
existing `unlock` command and the new `biometric_unlock` command call it, so the
central "unlock + arm timer" contract lives in one place. This is also the
natural unit-testable seam for the stale-passphrase path.

### New plugin crate `gpm-plugin-keystore/` (mirror safe-area scaffolding only)

- **`Cargo.toml`** — copy from safe-area; `name = "gpm-plugin-keystore"`,
  `links = "gpm-plugin-keystore"`. Deps `tauri = "2"`; build-dep
  `tauri-plugin = { version = "2", features = ["build"] }`.
- **`build.rs`** — `const COMMANDS: &[&str] = &["is_available", "store", "retrieve", "delete", "has_stored"];`
- **`src/lib.rs`** — the Rust shim. Exposes a `KeystoreExt` extension trait on
  `Manager<R>` so app commands do `app.keystore()?.retrieve().await`. Methods:
  `is_available()`, `store(pw)` (**async**), `retrieve()` (**async**),
  `delete()`, `has_stored()`. Mobile uses `run_mobile_plugin_async` for
  `store`/`retrieve` (Finding 7); desktop returns `None`/unavailable (`keystore()`
  returns `None` because the plugin is not registered there). Registration:
  `Builder::new("keystore").setup(|_app,_api| { #[cfg(target_os="android")] _api.register_android_plugin("xyz.yzx9.gpm","KeystorePlugin")?; Ok(()) }).build()`.
  Exact handle/capability wiring confirmed against Tauri 2.x in Phase 3.
- **`permissions/default.toml`** + **`permissions/schemas/schema.json`** — copy
  verbatim from safe-area. Frontend capability is intentionally **not** added to
  `mobile.json` — ops are backend-only (so the permission scaffold is largely
  vestigial, but harmless and keeps the crate shape consistent).

### `src-tauri/gen/android/app/src/main/java/xyz/yzx9/gpm/KeystorePlugin.kt`

(NEW, git-tracked, manually maintained — highest-risk file)

`@TauriPlugin class KeystorePlugin(activity: Activity) : Plugin(activity)`
mirroring `SafeAreaPlugin.kt`'s imports/shape, with the Keystore/GCM/
BiometricPrompt crypto **adapted from `impierce/tauri-plugin-keystore`**. The
`@Command`/`Invoke` mechanism is symmetric — the same handler serves a
`run_mobile_plugin_async` call from Rust.

Storage: AES/GCM key in `AndroidKeyStore`, alias `gpm_passphrase`; ciphertext +
IV as two base64 strings in `SharedPreferences("gpm_keystore", MODE_PRIVATE)`.
`@Command` methods: `is_available`, `store`, `retrieve`, `delete`, `has_stored`.

`MainActivity : TauriActivity()` → `WryActivity` (`WryActivity.kt:51`) →
`AppCompatActivity` → `FragmentActivity`, so the `BiometricPrompt` cast
`(activity as? FragmentActivity)` succeeds. **No `MainActivity` change needed.**
Verified against the actual generated chain.

**Key flags (D3 — API 30+ only, no version guards):** `KeyGenParameterSpec.Builder`
with `setUserAuthenticationRequired(true)` and
`setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG)` (API 30+; this is why
biometric is gated at API 30+). `setInvalidatedByBiometricEnrollment(true)` (the
default; desired — new fingerprint invalidates the key). Because biometric is
API-30-only, there is **no `setUserAuthenticationValidityDurationSeconds`
fallback** and no FingerprintManager backport path.

**Load-bearing: BOTH `store` and `retrieve` show a BiometricPrompt (D2).** A key
with `setUserAuthenticationRequired(true)` + timeout 0 requires a
CryptoObject-bound biometric auth for _every_ use (encrypt and decrypt); calling
`cipher.doFinal` without a preceding prompt throws `UserNotAuthenticatedException`
(verified against the Android
`KeyGenParameterSpec.Builder#setUserAuthenticationParameters` docs, and
independently confirmed by the codex outside voice). The enable flow is
therefore: validate passphrase → **one biometric prompt (CryptoObject ENCRYPT)**
→ `cipher.doFinal(pw)` → write ct+iv prefs → `pw.fill(0)`. (The plan's earlier
"no prompt at store-time — the typed passphrase is the auth" was wrong and is
removed: the typed passphrase is gpm's secret, unrelated to Keystore user-auth.)

**`retrieve` must NOT resolve synchronously** (holds the `Invoke` across the
prompt, resolves only from callbacks):

```kotlin
@Command
fun retrieve(invoke: Invoke) {
    val fa = (activity as? FragmentActivity)
        ?: run { invoke.reject("not FragmentActivity", "BIOMETRIC_UNAVAILABLE"); return }
    // load ciphertext+IV from prefs; init Cipher in DECRYPT mode with GCMParameterSpec(128, iv)
    // catch → invoke.reject(..., "BIOMETRIC_KEY_INVALIDATED")   // covers KeyPermanentlyInvalidatedException
    val cipher: Cipher = /* DECRYPT_MODE init */

    val prompt = BiometricPrompt(fa, ContextCompat.getMainExecutor(activity),
        object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(r: BiometricPrompt.AuthenticationResult) {
                try {
                    val plain = cipher.doFinal(ciphertext)
                    invoke.resolve(JSObject().put("passphrase", String(plain, UTF_8)))
                    plain.fill(0)                       // ByteArray hygiene
                } catch (e: Exception) { invoke.reject(safe(e), "BIOMETRIC_FAILED") }
            }
            override fun onAuthenticationError(code: Int, err: CharSequence) {
                val c = if (code in setOf(ERROR_USER_CANCELED, ERROR_NEGATIVE_BUTTON, ERROR_CANCELED))
                            "BIOMETRIC_CANCELLED" else "BIOMETRIC_FAILED"
                invoke.reject(err.toString(), c)
            }
            // onAuthenticationFailed (wrong finger, non-terminal) → do nothing; prompt stays open
        })
    prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
}
```

- `store`: symmetric — generate key, `Cipher` ENCRYPT_MODE, `BiometricPrompt`
  (CryptoObject ENCRYPT) → `cipher.doFinal(pw)` → write ct+iv prefs → `pw.fill(0)`.
- `is_available`: `Build.VERSION.SDK_INT >= R &&
BiometricManager.from(activity).canAuthenticate(BIOMETRIC_STRONG) == BIOMETRIC_SUCCESS`
  (A3 — must match the key's `BIOMETRIC_STRONG`, else `retrieve` fails at prompt
  time). Returns false below API 30 and on desktop.
- `has_stored`: returns whether ciphertext exists in prefs (no prompt).
- `delete`: `keyStore.deleteEntry(ALIAS)` + clear prefs.
- `safe(e)`: return `e.javaClass.simpleName` only — never leak the passphrase or
  crypto stack.
- `PromptInfo`: `setNegativeButtonText("Use passphrase")` so the dialog's cancel
  maps to `BIOMETRIC_CANCELLED` → page reveals the passphrase form.

### Kotlin-side rule

Always use `ByteArray`, never `String`, for secrets _within_ the crypto flow.
JVM `String` is immutable and cannot be zeroed; `ByteArray` can be zeroed
in-place with `.fill(0)`. Caveat (codex Finding 2, acknowledged): the passphrase
unavoidably becomes a JVM `String` at `invoke.resolve(... String(plain ...))` and
crosses the Kotlin→Rust hop as a JSON string (Tauri's `Invoke` transport). It is
wrapped in `Zeroizing<String>` on the Rust side and lives only briefly. The
"never String" rule governs the in-crypto-flow handling; the IPC String is
inherent to Tauri's transport and unavoidable.

### Backend (`src-tauri/src/lib.rs`) — five app commands + invalidation

Errors are a **`src-tauri`-local** type that serializes to `{code, message}`
(mirroring the `BIOMETRIC_*` strings) **with a `From<rustpass::Error>`** so
`store.unlock`'s `WrongPassphrase` maps to a code the frontend can match.
`rustpass::ErrorCode` is not touched.

- **`is_biometric_unlock_enabled`** → `plugin.has_stored()`; `false` on desktop.
- **`is_biometric_available`** → `plugin.is_available()`; `false` on desktop.
- **`enable_biometric_unlock(passphrase, app)`** → validate first:
  - age → `store.unlock(&pw)` (returns `WrongPassphrase` if wrong);
  - SSH → `ssh::Identity::from_buffer(buf, Some(pw))` → if `Encrypted`,
    `.decrypt()` (D4: rejects a wrong SSH passphrase at enable, instead of
    silently sealing it).

  On validation success → `plugin.store(pw)` (which shows the D2 biometric
  prompt). Needs `app: AppHandle`.

- **`biometric_unlock(app)`** → `plugin.retrieve()` → `unlock_and_arm(state, app,
&pw)` (D5). If `unlock_and_arm` returns `WrongPassphrase` (age case: stored
  passphrase is stale), call `plugin.delete()` and surface a "stored passphrase
  invalid — re-enable" outcome so the page reveals the form.
- **`disable_biometric_unlock`** → best-effort `plugin.delete()`.
- **Passphrase-change invalidation**: in `set_passphrase` / `change_passphrase`
  (x25519 only — SSH has no gpm passphrase-change path), after success call
  `plugin.delete()` (stored passphrase is now stale). SSH-passphrase biometric
  is invalidated only by key-invalidation (new fingerprint →
  `BIOMETRIC_KEY_INVALIDATED`), since gpm cannot change an SSH passphrase.
- Register `.plugin(gpm_plugin_keystore::init())` alongside safe-area; add the
  commands to `generate_handler!`. Messages carry no secret.

### Frontend — `UnlockPage.vue` (biometric on the existing page; no modal)

- **`src/biometric.ts`** (NEW) — thin wrappers over the five app commands only
  — **no `plugin:keystore|` strings**. `isBiometricAvailable`/
  `isBiometricUnlockEnabled` catch → `false` (desktop / below API 30).
- **`src/pages/UnlockPage.vue`** (modify, not replace — there is no
  `UnlockPage.test.ts` today, so nothing to delete):
  - `onMounted`: if `isBiometricUnlockEnabled() && isBiometricAvailable()` →
    auto-call `biometricUnlock()`; on success `router.push({ name: "entries" })`.
  - Always render the existing passphrase form; add an "Unlock with biometric"
    button (shown when `isBiometricAvailable()`), and a "Use passphrase instead"
    affordance after auto-prompt.
  - `BIOMETRIC_CANCELLED` → reveal/keep the passphrase form silently.
  - `BIOMETRIC_KEY_INVALIDATED` → info notice ("Biometric was reset — re-enable
    in Settings") + call `disableBiometricUnlock()` (delete the dead ciphertext
    so it stops auto-prompting) + reveal form. **No 3-strike lock** — biometric
    failure never blocks the password fallback.
  - Keep the existing `identity-locked` → `/unlock` redirect in `main.ts`
    (unchanged): on auto-lock, the user lands on `/unlock`, which auto-prompts
    biometric. (The "stay on current page" overlay is deferred to 0009.)
- **`src/pages/SettingsPage.vue`** — "Biometric Unlock" card, gated on
  `isIdentityEncrypted` (covers both age-passphrase and SSH-passphrase). If
  `!isBiometricAvailable()` → "not available on this device." Enable: dedicated
  passphrase input → `enableBiometricUnlock(pw)` (note: this now triggers the D2
  biometric prompt to seal). Disable: `disableBiometricUnlock()`. After
  `set/changePassphrase` succeeds, refresh the card (the stored passphrase was
  deleted by the invalidation hook).
- **`src/types.ts`** — add `BiometricStatus { available: boolean }` if needed.

### Other wiring

- **`Cargo.toml`** (workspace) — add `"gpm-plugin-keystore"` to `members`.
- **`src-tauri/Cargo.toml`** — `gpm-plugin-keystore = { path = "../gpm-plugin-keystore" }`.
- **`src-tauri/gen/android/app/build.gradle.kts`** —
  `implementation("androidx.biometric:biometric:1.1.0")` (appcompat already
  brings `FragmentActivity` via `WryActivity`).
- **`CHANGELOG.md`** — under `[Unreleased] → Added`: biometric unlock for
  passphrase-protected identities (age and SSH) on Android 11+, stored in Android
  Keystore (hardware-backed, biometric-gated); passphrase changes invalidate and
  require re-enabling; desktop and Android <11 stay passphrase-only.

## Edge cases

- **Passphrase change** → `plugin.delete()` → next launch offers no biometric →
  user re-enables. (Authoritative signal is the stored ciphertext's existence;
  self-healing.)
- **Biometric re-enrollment** (new fingerprint) → Android invalidates the key →
  `KeyPermanentlyInvalidatedException` → `BIOMETRIC_KEY_INVALIDATED` → notice +
  `disableBiometricUnlock()` + reveal form.
- **Stale stored passphrase (age)** → `biometric_unlock` → `store.unlock`
  returns `WrongPassphrase` → `plugin.delete()` + reveal form + re-enable notice.
- **Stale stored passphrase (SSH)** → `store.unlock` doesn't validate SSH, so it
  succeeds; the wrong passphrase surfaces as `DECRYPT_FAILED` on the first entry
  access. Mitigated by D4 (validated at enable), so this only happens if the SSH
  key itself changed out-of-band. Rare. Acceptable degradation.
- **Device below API 30 / desktop** → plugin unregistered or `is_available`
  false → `isBiometricAvailable()` returns `false` → `/unlock` shows passphrase
  form only. No Keystore code path.
- **Activity recreation** — MainActivity has `configChanges` set (no recreation
  on rotation); BiometricPrompt survives. (Request-id hardening deferred.)

## Implementation phases

0. **Rewrite this plan doc** (this step) + create [0009-unlock-modal.md](./0009-unlock-modal.md).
1. **`rustpass` prerequisite** — fix `is_unlocked()` for SSH + add the regression
   test + update the plaintext test. `just test`. Can be its own commit/PR —
   fixes the SSH-unlock regression for password _and_ biometric.
2. **Plugin skeleton** — `gpm-plugin-keystore` crate + `KeystoreExt` +
   `run_mobile_plugin_async` wiring + `is_available` (API 30+ STRONG). Verify
   availability on emulator; `just test` green on desktop.
3. **Keystore store/retrieve** — full `BiometricPrompt` CryptoObject flow for
   BOTH `store` (ENCRYPT, with prompt per D2) and `retrieve` (DECRYPT) + `has_stored`
   - `delete`. All emulator scenarios.
4. **Biometric integration** — `unlock_and_arm` helper (D5), 5 app commands
   (local `Biometric` error type with `From<rustpass::Error>`), `biometric.ts`,
   `UnlockPage.vue` biometric button + auto-prompt + cancel/invalidation handling,
   Settings card (gated on `isIdentityEncrypted`) with D2 prompt + D4 SSH
   validation, passphrase-change invalidation, stale-passphrase handling. `just test`.
5. **Edge cases + polish** — invalidation notice UX, release ProGuard/R8 check
   (`just android-install-release`), CHANGELOG. (No API-version fallback — API 30+ only.)

## Verification

- **Rust (`just test`)**: `is_unlocked()` now true for SSH after unlock (new
  **regression test**); updated plaintext test; the `unlock_and_arm` stale path
  via the helper.
- **Frontend (`pnpm test`)** — D6: `src/biometric.test.ts` (NEW — wrappers,
  desktop/`<API30` catch→false), `src/pages/UnlockPage.test.ts` (NEW — auto-prompt
  when enabled+available, cancel reveals form, invalidation notice + disable;
  mock `@tauri-apps/api/core` `invoke`), and biometric-card cases in the existing
  `SettingsPage.test.ts`; `vue-tsc --noEmit` on `biometric.ts`/`types.ts`.
- **Android device/emulator (`just android-debug` / `just android-dev`)**: AVD
  (API 30+) with fingerprint sensor; `adb -e emu finger add/touch`. Test:
  `is_available` true; `has_stored` false→true round `store` (with prompt);
  `store→retrieve` round-trip (prompt → success); cancel → `BIOMETRIC_CANCELLED`;
  wrong finger (prompt stays open) → correct finger → success; delete fingerprint
  post-store → `BIOMETRIC_KEY_INVALIDATED`; full enable→kill→relaunch→auto-prompt→
  entries→auto-lock (shorten `DEFAULT_LOCK_TIMEOUT_SECS` in debug)→`/unlock`
  re-prompt; passphrase change deletes biometric; **both age-passphrase and
  SSH-passphrase** identities unlock via biometric; enable with a WRONG SSH
  passphrase is rejected (D4); `adb shell run-as xyz.yzx9.gpm ...` shows
  `gpm_keystore.xml` (no `biometric.json`).
- **Desktop sanity (`just dev`)**: `isBiometricUnlockEnabled()` false;
  `isBiometricAvailable()` false; `/unlock` shows passphrase form only; no crash;
  no biometric UI.

## Risks

- **Tauri v2 Rust→mobile-plugin handle API** — confirmed canonical
  (`run_mobile_plugin_async` + ext trait; cf. haptics, official docs). Exact
  handle/capability wiring validated in Phase 2 (skeleton phase validates before
  any Kotlin crypto is written). **Must use the async variant** for
  `retrieve`/`store` (Finding 7).
- **Double `Invoke` resolution** — mitigated by resolving only from terminal
  callbacks; `onAuthenticationFailed` (non-terminal) does nothing.
- **`UserNotAuthenticatedException` at store time** — resolved by D2 (prompt on
  store too). Independently confirmed by codex + Android docs.
- **Passphrase in JS at _enable_ time** — unavoidable (user-typed, same as
  today's `unlock`). Only the high-frequency _retrieve_ path is backend-only.
- **ProGuard/R8** (release) — safe-area works in release; keep-rules likely
  suffice, but verify on a release build (Phase 5).

## Effort

~3 days (human) / ~1.5 hours (CC) — down from the prior ~3-4 day estimate now
that the modal refactor is deferred to 0009.

## Depends on

Passphrase-encrypted identities — already shipped (v0.3.0). The identity
groundwork landed in `48f5d7c` (on `main`). Branch `feat/biometric-unlock`.

## What already exists (reuse)

- `Store::unlock()` + `reset_lock_timer()` — biometric mirrors them exactly; no
  new unlock API needed.
- `Store::lock()` / the two caches — the lock state; the only rustpass change is
  making `is_unlocked()` consult both caches.
- `gpm-plugin-safe-area/` — crate scaffolding (Cargo.toml, build.rs,
  permissions, `register_android_plugin`) is copied; only `src/lib.rs` differs
  (it needs the backend-callable `run_mobile_plugin_async` pattern).
- `SafeAreaPlugin.kt` — the `@TauriPlugin`/`@Command`/`Invoke` Kotlin shape.
- The `identity-locked` event + `get_auth_state` + the `/unlock` route — the page
  is driven by these, no new event or route needed.
- `tauri-plugin-haptics` (external) — reference for the `run_mobile_plugin` +
  ext-trait pattern.

## NOT in scope

- **Modal overlay refactor** — deferred to [0009-unlock-modal.md](./0009-unlock-modal.md).
  Biometric ships on the existing `/unlock` route in this plan.
- **iOS biometric** — deferred (different Keystore/LAKeychain API).
- **Multi-identity** — single identity only (plan 0006); one passphrase stored.
- **Biometric below API 30** — gated at API 30+ (D3); no FingerprintManager
  backport, no validity-duration fallback.
- **Caching the decrypted SSH identity** — `crypto.rs` currently re-decrypts the
  SSH key per entry using `cached_passphrase`; caching the decrypted key (like
  x25519) is a separate optimisation, not needed for biometric.
- **Discarding the passphrase for x25519 after unlock** — currently cached
  redundantly (`store.rs:170`); a "use once then drop" cleanup is filed as a TODO
  (D8: keep as TODO, not in-scope here).
- **Fixing the broader `is_unlocked()` semantics / plaintext caching** beyond
  the minimal SSH fix — out of scope; only the targeted fix lands.
- **Lock-timer race (generation check)** — pre-existing (codex Finding 6); filed
  as a TODO.
- **`has_stored` key/enrollment liveness check** — optional sharpen (codex
  Finding 5); filed as a TODO.
- **Encrypted-SSH passphrase _change_** — gpm can't change an SSH passphrase
  (SSH keys rely on native protection); SSH biometric invalidation is by
  key-invalidation only.

## Failure modes

For each new codepath, a realistic production failure and its handling:

| Codepath           | Failure                           | Test?           | Error handling?                                     | User sees          |
| ------------------ | --------------------------------- | --------------- | --------------------------------------------------- | ------------------ |
| `retrieve`         | Key invalidated (new fingerprint) | emulator ✓      | `BIOMETRIC_KEY_INVALIDATED` → delete + form         | notice + form      |
| `retrieve`         | User cancels                      | emulator ✓      | `BIOMETRIC_CANCELLED` → form                        | form silently      |
| `retrieve`         | Double-resolve race               | unit (hard)     | resolve only from terminal callbacks                | n/a                |
| `store`            | No auth prompt → `UserNotAuth`    | n/a (D2 fix)    | D2: prompt on store removes this                    | prompt shown       |
| `biometric_unlock` | Stale age passphrase              | unit (helper) ✓ | `WrongPassphrase` → delete + form                   | re-enable notice   |
| `biometric_unlock` | Stale SSH passphrase              | emulator        | D4 validated at enable; else `DECRYPT_FAILED` later | entry error        |
| `store`            | AES init fails                    | emulator ✓      | `BIOMETRIC_FAILED`                                  | "not available"    |
| enable             | Wrong SSH passphrase              | unit ✓ (D4)     | ssh decrypt fails → reject before store             | "wrong passphrase" |

No critical gap (every failure has error handling or is prevented); the
double-resolve race is the one with no direct test but is structurally guarded.

## TODOs (proposed)

- **Cache the decrypted SSH identity** (perf): `crypto.rs` re-decrypts the SSH
  key on every entry access; cache it after first unlock like x25519. _Why:_ cuts
  per-decrypt cost; _Depends on:_ the `is_unlocked()` fix (so the cache is
  recognised).
- **Drop the redundant x25519 passphrase cache** (secret lifetime, codex F2):
  `store.rs:170` caches the passphrase unconditionally; for x25519 it's unused
  after unlock. _Why:_ smaller secret surface in memory on the exact feature that
  retrieves the passphrase. Kept as a TODO here (D8); mind the test that encodes
  current behaviour.
- **Lock-timer generation check** (correctness, codex F6): `reset_lock_timer`
  aborts the prior task, but abort is not a generation check — a timer that
  already woke can still `store.lock()` + emit `identity-locked` right after a
  fresh unlock. _Why:_ closes a real (tiny-window) race; biometric makes it more
  visible by auto-prompting again. _Fix:_ monotonic session token captured by the
  timer, checked before locking. Pre-existing, not biometric-introduced.
- **`has_stored` liveness check** (sharpness, codex F5): also verify
  `keyStore.getEntry(alias, null)` and `BiometricManager.canAuthenticate(STRONG)`
  so "enabled" reflects key + enrollment, not just prefs. _Why:_ removes the
  stale-"enabled"-until-first-retrieve window.
- **Auto-lock biometric-prompt cadence** (UX): auto-prompting biometric on every
  5-min auto-lock may be aggressive if the user stepped away (prompt times out).
  _Why:_ consider prompting on user-initiated re-show vs every lock. Defer until
  real-world use feedback. (Mostly mooted once 0009's modal lands.)

## Worktree parallelization

With the modal deferred to 0009, this plan is largely sequential after Phase 1:

| Step                  | Modules touched                                                                  | Depends on   |
| --------------------- | -------------------------------------------------------------------------------- | ------------ |
| Phase 1 (rustpass)    | `rustpass/src/store.rs`                                                          | —            |
| Phase 2-3 (plugin)    | `gpm-plugin-keystore/`, `KeystorePlugin.kt`, `build.gradle.kts`                  | Phase 1      |
| Phase 4 (integration) | `src-tauri/src/lib.rs`, `src/biometric.ts`, `UnlockPage.vue`, `SettingsPage.vue` | Phases 1 + 3 |
| Phase 5 (polish)      | `KeystorePlugin.kt`, proguard rules, `CHANGELOG.md`                              | Phase 4      |

- **Lane A**: Phases 2-3 (plugin crate + Kotlin crypto) — Android-only, no
  frontend dependency.
- **Lane B**: nothing meaningful parallels A in this plan (the frontend
  integration in Phase 4 touches `src-tauri/src/lib.rs`, which Lane A's crate
  registration also touches → sequential).
- **Execution**: Phase 1 first; then Lane A (Phases 2-3) in a worktree; then
  Phase 4-5 sequentially in the main worktree (it depends on both the rustpass
  fix and the plugin).
- **Parallelization opportunity is limited** in this plan. The bigger split
  (modal ↔ biometric) moved to 0009.

## Implementation Tasks

- [ ] **T1 (P1, human: ~2h / CC: ~15min)** — `rustpass/src/store.rs` — Fix
      `is_unlocked()` to consult `cached_passphrase` (SSH unlock recognition);
      add the SSH-after-unlock regression test; update the plaintext test.
  - Surfaced by: Architecture — the latent SSH-unlock regression (commit
    `48f5d7c`) that blocks SSH biometric and SSH password unlock alike.
  - Files: `rustpass/src/store.rs`
  - Verify: `just test`.
- [ ] **T2 (P1, human: ~4h / CC: ~20min)** — `gpm-plugin-keystore/`,
      `KeystorePlugin.kt` — New backend-callable plugin crate + Kotlin Keystore/
      BiometricPrompt with **prompts on both store (D2) and retrieve**, API 30+
      STRONG keys (D3), async variants (F7).
  - Surfaced by: Build-vs-buy + plugin-pattern + D2/D3/F7.
  - Files: `gpm-plugin-keystore/*`,
    `src-tauri/gen/android/.../KeystorePlugin.kt`, `build.gradle.kts`,
    workspace `Cargo.toml`, `src-tauri/Cargo.toml`.
  - Verify: emulator round-trip (Phase 2-3 checklist).
- [ ] **T3 (P1, human: ~3h / CC: ~15min)** — `src-tauri/src/lib.rs`, `src/` —
      `unlock_and_arm` helper (D5), 5 biometric commands (local error type +
      `From<rustpass::Error>`), `biometric.ts`, `UnlockPage.vue` biometric UI,
      Settings card with **enable-time prompt (D2) + SSH validation (D4)**,
      passphrase-change invalidation, stale handling.
  - Surfaced by: Layering + D4/D5 + Architecture.
  - Files: `src-tauri/src/lib.rs`, `src/biometric.ts` (new), `UnlockPage.vue`,
    `SettingsPage.vue`, `src/types.ts`.
  - Verify: `just test`; emulator full enable→relaunch→unlock→auto-lock flow.
- [ ] **T4 (P2, human: ~2h / CC: ~10min)** — D6 vitest: `biometric.test.ts`,
      `UnlockPage.test.ts` (new), `SettingsPage.test.ts` card cases (mock
      `invoke`).
  - Surfaced by: Test review (D6).
  - Files: `src/biometric.test.ts`, `src/pages/UnlockPage.test.ts`,
    `src/pages/SettingsPage.test.ts`.
  - Verify: `pnpm test`.
- [ ] **T5 (P2, human: ~1h / CC: ~5min)** — release ProGuard/R8 check + CHANGELOG.
  - Surfaced by: Risks.
  - Files: proguard rules, `CHANGELOG.md`.
  - Verify: `just android-install-release`.

## GSTACK REVIEW REPORT

| Review        | Trigger               | Why                             | Runs | Status       | Findings                                                                          |
| ------------- | --------------------- | ------------------------------- | ---- | ------------ | --------------------------------------------------------------------------------- |
| CEO Review    | `/plan-ceo-review`    | Scope & strategy                | 0    | —            | —                                                                                 |
| Codex Review  | `/codex review`       | Independent 2nd opinion         | 1    | issues_found | 7 findings: 2 confirmed our A1/A4, 4 new (2 baked-in, 2 TODO), 1 deferred to 0009 |
| Eng Review    | `/plan-eng-review`    | Architecture & tests (required) | 2    | CLEAR (PLAN) | Pass 2: 6 issues raised (A1-A4, C1, D6) + modal split → 0009                      |
| Design Review | `/plan-design-review` | UI/UX gaps                      | 0    | —            | —                                                                                 |
| DX Review     | `/plan-devex-review`  | Developer experience gaps       | 0    | —            | —                                                                                 |

- **CODEX:** confirmed the store-time-prompt contradiction (A1) and SSH-no-validation (A4) via the official Android docs; surfaced F2 (secret lifetime), F4 (modal secret-clearing → 0009 blocker), F6 (lock-timer race → TODO), F7 (async variant → baked-in).
- **CROSS-MODEL:** strong agreement on the two highest-severity items (store-time prompt, SSH validation). No unresolved cross-model tension — codex's F2/F5/F6 captured as TODOs, F4/F7 baked in.
- **UNRESOLVED:** 0
- **VERDICT:** ENG + CODEX CLEARED (PLAN) — ready to implement. Start with T1 (the `is_unlocked()` prerequisite), which unblocks SSH biometric and SSH password unlock alike. Modal overlay deferred to 0009.
