# gpm — Android-first age-only gopass password client

A read-only, age-only, gopass-compatible password client for Android (and desktop), built on **Tauri v2 + Rust + Vue 3**.

## Why

There is no Android GUI client that can read age-encrypted gopass/password-store repositories. The Android Password Store app is unmaintained and GPG-only. gopass itself is Go/CLI-only. People resort to running gopass inside Termux on Android.

**gpm fills this gap.**

## Security Model

- **Age-only** — no GPG, no cloud, no analytics, no Autofill
- **Copy password never touches WebView** — decrypts and copies entirely on the Rust side
- **Show password has 30s auto-clear** — with page-leave cleanup in Vue
- **Zeroize-per-decrypt** — identity bytes wiped after every decrypt call
- **Safe error messages** — no secrets in logs, errors, or toasts

## Features

- Clone a gopass age-encrypted password store from a Git URL (HTTPS + PAT or SSH key)
- Decrypt entries encrypted to native x25519 keys (`age1...`) or SSH keys (`ssh-ed25519`, `ssh-rsa`)
- List all `.age` entries with display names
- Search entries by name (frontend filtering, case-insensitive)
- Copy password to clipboard (password never reaches WebView)
- View password with 30-second auto-clear and page-leave cleanup
- View notes metadata
- Pull updates (fast-forward only) from the remote repo
- Generate ed25519 SSH keys on-device, or paste existing keys
- View SSH public key and export private key from settings

## Getting Started

See [DEVELOPMENT.md](./DEVELOPMENT.md) for development environment setup, commands, and known issues.

## License

See [LICENSE](./LICENSE).
