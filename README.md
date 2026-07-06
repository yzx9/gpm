# gpm — Android-first age-only gopass password client

A read-only, age-only, gopass-compatible password client for Android (and desktop), built on
**Tauri v2 + Rust + Vue 3**.

## Why

There is no Android GUI client that can read age-encrypted gopass/password-store repositories. The
Android Password Store app is unmaintained and GPG-only. gopass itself is Go/CLI-only. People
resort to running gopass inside Termux on Android.

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
- Search entries by name (fuzzy, case-insensitive)
- Copy password to clipboard (password never reaches WebView)
- View password with auto-clear and page-leave cleanup
- View notes metadata
- Create and edit secrets with gopass-compatible templates
- Pull and push updates over git (fast-forward only), with optional auto-sync on every save
- Generate ed25519 SSH keys on-device, or paste existing keys
- View SSH public key and export private key from settings

## Contribution

Any help in the form of descriptive and friendly [issues](https://github.com/yzx9/gpm/issues) or
comprehensive pull requests are welcome!

Please check out [DEVELOPMENT.md](DEVELOPMENT.md) for guidelines.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
gpm by you, as defined in the [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) license,
without any additional terms or conditions.

Thanks goes to these wonderful people:

[![Contributors](https://contrib.rocks/image?repo=yzx9/gpm)](https://github.com/yzx9/gpm/graphs/contributors)

## LICENSE

This work is licensed under a <a rel="license" href="https://www.apache.org/licenses/">Apache-2.0</a>.

Copyright (c) 2026, Zexin Yuan
