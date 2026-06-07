# just is a command runner, Justfile is very similar to Makefile, but simpler.

default:
  @just --list

# Run Rust integration tests
test:
  cd src-tauri && cargo test --all-features

# Clippy + vue-tsc type check
lint:
  cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
  npx vue-tsc --noEmit

# Format Rust + Vue code
fmt:
  cd src-tauri && cargo fmt
  prettier --write src

# Desktop dev server with hot reload
dev:
  pnpm tauri dev

# Build debug APK
android-build:
  pnpm tauri android build --debug

# Build release APK (unsigned)
android-release:
  pnpm tauri android build

# Android dev server (requires connected device or emulator)
android-dev:
  pnpm tauri android dev

# Install debug APK to connected device
android-install: android-build
  adb install src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
