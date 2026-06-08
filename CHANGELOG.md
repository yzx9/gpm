# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [v0.1.0] - 2026-06-08

In this initial release, we have implement a read-only age-only gopass password client for Android, built with Tauri v2 + Rust + Vue 3.

### Added

- Clone age-encrypted gopass repositories via HTTPS + PAT
- List and search password entries
- Decrypt and copy passwords to clipboard
- Show password with 30-second auto-clear and lifecycle cleanup
- Pull-to-refresh to sync with remote repository
- Android APK signing and per-architecture release builds

[Unreleased]: https://github.com/yzx9/gpm/compare/v0.1.0...HEAD
[v0.1.0]: https://github.com/yzx9/gpm/releases/tag/v0.1.0
