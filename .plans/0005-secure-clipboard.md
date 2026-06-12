# Custom SecureClipboard (ByteArray-based)

**Priority:** P3
**Status:** TODO
**Phase:** Future

## What

Replace `tauri-plugin-clipboard-manager` with a custom Kotlin clipboard plugin that uses `ByteArray` throughout, eliminating JVM `String` retention in the clipboard path.

## Why

The current `tauri-plugin-clipboard-manager` likely uses `String` internally on the Kotlin side. JVM `String` is immutable — once created, it cannot be zeroed and lingers on the heap until garbage collection. GC does not overwrite memory. This means the password exists as an unzeroable JVM `String` for an unbounded time.

A custom plugin using `ByteArray` can zero the array immediately after passing it to `ClipboardManager`, closing this memory retention gap.

## Context

### Current clipboard flow

```
Rust: decrypt → tauri-plugin-clipboard-manager → Kotlin (String) → Android ClipboardManager
```

The plugin bridge serialization + Kotlin `String` both retain the password.

### Proposed clipboard flow

```kotlin
class SecureClipboardPlugin : Plugin() {
    @Command
    fun copyWithTtl(invoker: Invoker, content: ByteArray, ttlSeconds: Int) {
        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val clip = ClipData.newPlainText("password", String(content, Charsets.UTF_8))
        clipboard.setPrimaryClip(clip)

        content.fill(0) // Zero ByteArray immediately

        Handler(Looper.getMainLooper()).postDelayed({
            val clearClip = ClipData.newPlainText("cleared", "")
            clipboard.setPrimaryClip(clearClip)
        }, ttlSeconds * 1000L)

        invoker.resolve(null)
    }
}
```

### Key files

- New: `src-tauri/gen/android/app/src/main/java/xyz/yzx9/gpm/SecureClipboardPlugin.kt`
- `src-tauri/src/lib.rs` — Replace clipboard plugin registration
- `src-tauri/Cargo.toml` — Replace `tauri-plugin-clipboard-manager` with custom plugin

### Considerations

- This only affects Android. Desktop continues using the standard clipboard plugin.
- The `ClipData.newPlainText` call still creates a temporary `String`, but it's scoped to the call and eligible for GC immediately. The `ByteArray` zeroing is the meaningful improvement.
- Consider combining with the auto-clear timer (currently 30s via `tokio::time::sleep` in Rust). Moving the timer to Kotlin avoids the IPC round-trip for clearing.

## Effort

~0.5 day (human) / ~15 min (CC)

## Depends on

None — independent. Can be combined with 0004-keystore-biometric.md if both touch the Kotlin plugin layer.
