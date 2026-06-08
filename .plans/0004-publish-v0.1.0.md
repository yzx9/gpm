# 0005: Publish v0.1.0

**Priority:** P1
**Status:** TODO
**Phase:** Now

## What

Publish the first release of gpm as a signed APK on GitHub Releases. The MVP is feature-complete: clone, list, search, decrypt, copy, pull.

## Why

The app has been functional since Phase 3 but never published. Shipping establishes a baseline for user feedback and iterative development.

## Context

CI is already configured (`.github/workflows/release.yml`). The workflow builds per-architecture APKs + universal APK on tag push, signs them, and uploads to GitHub Releases.

### Release checklist

1. **Generate signing keystore:**

   ```bash
   keytool -genkey -v -keystore release.keystore -alias gpm \
     -keyalg RSA -keysize 2048 -validity 10000
   ```

2. **Configure GitHub Secrets:**
   - `KEYSTORE_BASE64` — base64-encoded keystore file
   - `KEYSTORE_PASSWORD` — keystore password
   - `KEY_ALIAS` — key alias (`gpm`)
   - `KEY_PASSWORD` — key password

3. **Push tag:**

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

4. **Verify:** CI builds APKs → signs → creates GitHub Release with assets

5. **Write release notes:** MVP feature summary, supported Android versions (9+), known limitations (identity stored in plaintext, single identity only, HTTPS-only git)

### Known limitations to document in release notes

- Identity stored as plaintext in app-private directory (no Android Keystore yet)
- Single age identity only (no SSH key support, no multi-identity)
- HTTPS-only git remotes (no SSH)
- No app lock / biometric re-auth
- No edit/create/delete operations (read-only by design)
- Desktop bundles not provided (dev mode only via `cargo tauri dev`)

### Key files

- `.github/workflows/release.yml` — Already complete
- `src-tauri/gen/android/app/build.gradle.kts` — Signing config reads from `keystore.properties`

## Effort

~10 minutes (manual steps: keystore generation, GitHub Secrets, tag push)

## Depends on

None — MVP is ready.
