# just is a command runner, Justfile is very similar to Makefile, but simpler.

default:
  @just --list

# Run backend (Rust) integration tests
test-be:
  cargo test --all-features

# Run frontend unit tests
test-fe:
  pnpm test

# Run all tests (backend + frontend)
test: test-be test-fe

# Clippy + vue-tsc type check
lint:
  cargo clippy --all-targets --all-features -- -D warnings
  npx vue-tsc --noEmit

# Format Rust + Vue code
fmt:
  cargo fmt
  prettier --write src

# Desktop dev server with hot reload
dev:
  pnpm tauri dev

# Build debug APK (optional: specify arch e.g. aarch64, armv7, i686, x86_64; builds universal if omitted)
android-debug target='':
  @if [ -z "{{ target }}" ]; then \
    pnpm tauri android build --debug; \
  else \
    pnpm tauri android build --debug -t "{{ target }}"; \
  fi

# Build release APKs (per-arch + universal, signed if keystore.properties exists)
android-release:
  pnpm tauri android build --apk --split-per-abi
  pnpm tauri android build --apk

# Android dev server (requires connected device or emulator)
android-dev:
  pnpm tauri android dev

# Install debug APK to connected device (auto-detects arch, or specify e.g. aarch64)
android-install target='':
  @if [ -z "{{ target }}" ]; then \
    ABI=$$(adb shell getprop ro.product.cpu.abi 2>/dev/null); \
    case "$$ABI" in \
      arm64-v8a)   T="aarch64" ;; \
      armeabi-v7a) T="armv7" ;; \
      x86_64)      T="x86_64" ;; \
      x86)         T="i686" ;; \
      *)           T="aarch64" ;; \
    esac; \
    echo "Detected ABI: $${ABI:-unknown}, target: $$T"; \
    pnpm tauri android build --debug -t "$$T"; \
  else \
    pnpm tauri android build --debug -t "{{ target }}"; \
  fi
  adb install src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk

# Install release APK to connected device (auto-detect arch)
android-install-release: android-release
  @ARCH=$$(adb shell getprop ro.product.cpu.abi 2>/dev/null); \
  case "$$ARCH" in \
    arm64-v8a)   APK="arm64/release/app-arm64-release.apk" ;; \
    armeabi-v7a) APK="arm/release/app-arm-release.apk" ;; \
    x86_64)      APK="x86_64/release/app-x86_64-release.apk" ;; \
    x86)         APK="x86/release/app-x86-release.apk" ;; \
    *)           APK="universal/release/app-universal-release.apk" ;; \
  esac; \
  echo "Installing for ABI: $${ARCH:-unknown}"; \
  adb install "src-tauri/gen/android/app/build/outputs/apk/$$APK"
