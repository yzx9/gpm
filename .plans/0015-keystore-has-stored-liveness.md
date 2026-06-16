# Keystore `has_stored` liveness check (UX)

**Priority:** P3 (UX polish; security unchanged — self-healing already works)
**Scope:** `tauri-plugin-biometric-keystore` (Kotlin). Optional frontend touch.
**Status:** Not started.

## Context

`has_stored()` is the **single** "is biometric enabled?" signal (no flag file;
`is_biometric_unlock_enabled` just proxies it — `src-tauri/src/biometric.rs:70`). Today it only
checks prefs:

```kotlin
fun has_stored(invoke: Invoke) {
    val ret = JSObject()
    ret.put("stored", readCipherData() != null)   // KeystorePlugin.kt:201-205
    invoke.resolve(ret)
}
```

`readCipherData()` reads the `ct`/`iv` strings from `SharedPreferences("gpm_keystore")`. It does
**not** verify the AES key still exists or that a STRONG biometric is still enrolled. So "enabled" =
"ciphertext blob in prefs", regardless of whether the key is live. (codex F5 from
`.plans/0002-keystore-biometric.md`.)

### Why it affects UX

The key is generated with `setUserAuthenticationRequired(true)` + `setInvalidatedByBiometricEnrollment(true)`
(`KeystorePlugin.kt:111,114`, the latter is the default). So **enrolling a new fingerprint/face
permanently destroys the key** — any later `cipher.init` throws `KeyPermanentlyInvalidatedException`.
There is no "add a new fingerprint to an existing key" API: Android treats re-enrollment as possible
tampering (the device may have been unattended) and revokes keys created under the old, trusted set.
gpm keeps the secure default rather than weakening it with `setInvalidatedByBiometricEnrollment(false)`.

Because `has_stored` doesn't see the invalidation, after a fingerprint change:

- `has_stored` = `true` → `isBiometricUnlockEnabled()` = `true`.
- The unlock overlay auto-prompts on every cold launch (`UnlockModal.vue:106`, gated on
  `enabled && available`).
- `retrieve` inits the decrypt cipher → `KeyPermanentlyInvalidatedException` →
  `BIOMETRIC_KEY_INVALIDATED` → frontend disables + reveals the form.

It **self-heals on the next `retrieve`**, but the user sees one doomed fingerprint prompt per launch
until they re-enable. The passphrase fallback always works, so this is UX, not security.

## Design

In `has_stored`, also verify the key is live **and** a STRONG biometric is enrolled — so "enabled"
reflects reality and the app skips the doomed auto-prompt:

```kotlin
@Command
fun has_stored(invoke: Invoke) {
    val keyLive = try {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        ks.getEntry(KEY_ALIAS, null) != null
    } catch (e: Exception) { false }
    val enrolled = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
        BiometricManager.from(activity).canAuthenticate(strongAuthenticator) ==
            BiometricManager.BIOMETRIC_SUCCESS
    val ret = JSObject()
    ret.put("stored", readCipherData() != null && keyLive && enrolled)
    invoke.resolve(ret)
}
```

Both checks are non-prompting: `getEntry(alias, null)` with a null protection parameter does not
require user auth, and `canAuthenticate` does not show a prompt. Stays API-30-gated (matches the
key).

### UX wrinkle — to decide

With the check, a cold launch after invalidation goes **straight to the passphrase form with no
notice** (vs. today's doomed-prompt-then-notice). The Settings biometric card correctly shows
biometric as re-enableable either way. Two options:

- **(a) Minimal (recommended):** just the liveness check. The frontend already degrades gracefully
  to the passphrase form. The overlay's existing `BIOMETRIC_KEY_INVALIDATED` notice still fires if
  the user taps the (now hidden) biometric button or if availability later flips. Cleanest; this
  plan.
- **(b) Optional enhancement:** surface a one-time "Biometric was reset — re-enable in Settings"
  notice when `isBiometricUnlockEnabled()` flips false. Hard to distinguish "never enabled" from
  "invalidated" without remembering prior state — would need a last-known-enabled flag in prefs.
  Defer unless wanted.

This plan implements (a).

## Implementation

- **`tauri-plugin-biometric-keystore/android/src/main/java/KeystorePlugin.kt`** — extend
  `has_stored` as above (reuse the existing `ANDROID_KEYSTORE`, `KEY_ALIAS`, `strongAuthenticator`
  already defined in the file). No change to `store`/`retrieve`/`delete`.
- No Rust or frontend change for option (a): `is_biometric_unlock_enabled` already returns the
  boolean unchanged; the overlay already handles `enabled=false` (passphrase form only).

## Tests

Kotlin has no in-tree unit-test harness for the Android plugin (it's verified on-device). Fold this
into the still-pending **0002 on-device verification**: the `store→enroll-new-fingerprint→has_stored`
scenario should now report `false` (today it reports `true` until `retrieve` self-heals). Also:
remove all fingerprints post-store → `has_stored` false; re-enable → round-trip works.

## Verification

- Emulator, API 30+ with a fingerprint sensor (`adb -e emu finger add/touch`):
  enable biometric → `has_stored` true; enroll a new fingerprint → `has_stored` **false** (no
  doomed auto-prompt; cold launch lands on the passphrase form); re-enable → round-trip.
- Desktop sanity unchanged (`has_stored` is Android-only; desktop stays false).

## Risks

- **`getEntry` semantics** — confirm `KeyStore.getEntry(alias, null)` returns non-null for a live
  biometric-gated key without prompting (it should — the protection param gates auth-on-use, not
  read). Verify on emulator in Phase 1.
- **Notice regression** — option (a) loses the explanatory notice on cold-launch invalidation;
  acceptable (Settings card is self-explanatory), flagged above.

## NOT in scope

- The one-time invalidation notice (option b) — deferred.
- SSH-key caching / passphrase-lifetime — **0013** / **0014**.
