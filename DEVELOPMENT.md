# Development Guide

## Setup

### Clone

```bash
git clone https://github.com/yzx9/gpm.git
cd gpm
```

### Dev environment

We recommend using [Nix](https://nixos.org/download/) for a consistent, reproducible dev environment across all platforms. The provided `flake.nix` sets up everything you need:

- **Rust toolchain** via `fenix` with 4 Android cross-compilation targets (`aarch64`, `armv7`, `x86_64`, `i686`)
- **Android SDK/NDK** via `androidenv` — platforms 28 + 36, NDK, build-tools, cmake
- **JDK 17**
- **Frontend tooling**: Node.js, pnpm
- **Utilities**: just, prettier, etc.

Install [direnv](https://direnv.net/) (with its [shell hook](https://direnv.net/docs/hook.html)) to auto-load the environment when you `cd` into the project:

```bash
direnv allow
```

On first run, Nix builds the dev shell (this may take a few minutes). Once done, `just`, `cargo`, `pnpm`, and the full Android toolchain will be in your PATH.

<details>
<summary><strong>Without Nix (manual setup)</strong></summary>

You'll need to install these yourself:

**Rust**

- [rustup](https://rustup.rs/) with stable toolchain + Android targets: `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android`

**Frontend**

- [Node.js](https://nodejs.org/)
- [pnpm](https://pnpm.io/)

**Android & JDK**

- **JDK 17**: [Adoptium Temurin](https://adoptium.net/) or your preferred distribution; set `JAVA_HOME`
- **Android SDK/NDK** — Android Studio or [command-line tools](https://developer.android.com/studio#command-line-tools-only); set `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `ANDROID_NDK_ROOT`

**Utilities**

- [just](https://github.com/casey/just) — task runner (`cargo install just`)
- [prettier](https://prettier.io/) — code formatter (`pnpm add -g prettier`)

</details>

### Install frontend dependencies

```bash
pnpm install
```

### Verify everything works

```bash
just lint   # Clippy + vue-tsc type check
just test   # All tests (Rust + frontend)
```

## Commands

We use [just](https://github.com/casey/just) as a task runner. The most common commands:

```bash
just test              # Run all tests (backend + frontend)
just test-be           # Run backend (Rust) tests only
just test-fe           # Run frontend unit tests only
just lint              # Clippy -D warnings + vue-tsc --noEmit
just fmt               # rustfmt + prettier
just dev               # Desktop dev server with hot reload
just android-debug     # Build debug APK
just android-release   # Build release APK (signed if keystore.properties exists)
just android-dev       # Android dev server (requires device/emulator)
just android-install   # Build + install debug APK to connected device
just android-install-release # Build + install release APK to connected device
```

If you don't want to use `just`, you can see the individual commands in `justfile` and run them manually.

## Known Issues

### macOS: Vendored OpenSSL cross-compilation fails for Android

**Problem:** When cross-compiling vendored OpenSSL from macOS to Android, rustc fails with:

```
error: failed to add native library .../libssl.a: invalid utf-8 sequence of 1 bytes from index 0
```

Tracked upstream: [rust-lang/rust#131407](https://github.com/rust-lang/rust/issues/131407)

**Root cause:** macOS's system `ar` creates BSD-format archives (header `#1/20` + `__.SYMDEF` symbol table). rustc cannot parse these when cross-compiling to Linux/Android targets — it expects GNU-format archives produced by GNU `ar` or LLVM's `llvm-ar`.

**Fix:** Set `AR`, `TARGET_AR`, and `RANLIB` to the NDK's LLVM tools in `flake.nix` `shellHook`:

```nix
shellHook = ''
  export PATH="${ndkBin}:$PATH"
'' + lib.optionalString pkgs.stdenv.isDarwin ''
  export AR="${ndkBin}/llvm-ar"
  export TARGET_AR="${ndkBin}/llvm-ar"
  export RANLIB="${ndkBin}/llvm-ranlib"
'';
```

Key details:

- `openssl-sys`'s build script checks `TARGET_AR` (not `AR_aarch64_linux_android`), so setting only the target-prefixed env var is insufficient
- All three vars (`AR`, `TARGET_AR`, `RANLIB`) are needed — `RANLIB` rebuilds the symbol table in GNU format
- Gated behind `pkgs.stdenv.isDarwin` — Linux hosts are unaffected because the system `ar` already produces GNU-format archives

**Files involved:** `flake.nix` (shellHook), `src-tauri/Cargo.toml` (`git2` with `vendored-openssl` + `vendored-libgit2`)

## Contributing

Contributions are welcome! We follow standard GitHub flow:

1. Fork the repository
2. Create a feature branch
3. Make your changes — ensure `just lint` and `just test` pass
4. Open a pull request with a clear description of the problem and solution
5. Address review feedback and iterate

This project is licensed under [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0). By contributing, you agree that your contributions will be licensed under the same terms.

## Releasing

Releases are automated via GitHub Actions. Pushing a `v*` tag (e.g. `v0.1.0`) triggers the release workflow, which builds a signed APK and publishes it as a GitHub Release.

### Setup (one-time)

1. **Generate a signing keystore:**

   ```bash
   mkdir -p ~/.keystores
   keytool -genkey -v -keystore ~/.keystores/gpm-upload.jks \
     -keyalg RSA -keysize 2048 -validity 10000 -alias upload
   ```

2. **Create `src-tauri/gen/android/keystore.properties`** (gitignored):

   ```properties
   keyAlias=upload
   password=<your-password>
   storeFile=/path/to/gpm-upload.jks
   ```

3. **Add GitHub Secrets** (Settings → Secrets → Actions):
   - `ANDROID_KEY_ALIAS` — e.g. `upload`
   - `ANDROID_KEY_PASSWORD` — the keystore password
   - `ANDROID_KEYSTORE_BASE64` — `base64 -i ~/.keystores/gpm-upload.jks`

### Create a release

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds the signed APK and creates a GitHub Release with the artifact attached. Tags containing `-rc` or `-beta` are automatically marked as pre-releases.
