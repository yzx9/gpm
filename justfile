# just is a command runner, Justfile is very similar to Makefile, but simpler.

default:
  @just --list

# Run backend (Rust) integration tests
test-be:
  cd src-tauri && cargo test --all-features

# Run frontend unit tests
test-fe:
  pnpm test

# Run all tests (backend + frontend)
test: test-be test-fe

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
android-debug:
  pnpm tauri android build --debug

# Build release APK (signed if keystore.properties exists)
android-release:
  pnpm tauri android build

# Android dev server (requires connected device or emulator)
android-dev:
  pnpm tauri android dev

# Install debug APK to connected device
android-install: android-debug
  adb install src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk

# Install release APK to connected device
android-install-release: android-release
  adb install src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
