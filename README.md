# gpm — Android-first gopass-compatible password client

A gopass-compatible password client for Android (and desktop), built on **Tauri v2 + Rust + Vue 3**.
It opens existing **age** and **GPG/OpenPGP** gopass repositories; new stores it creates are age-only.

## Highlights

- **Drop-in gopass compatibility.** Clone an existing gopass or password-store repository and it just works — gpm reads and writes the standard on-disk format, age or GPG. If something doesn't interoperate with `gopass`, [it's a bug](https://github.com/yzx9/gpm/issues).
- **age and GPG/OpenPGP, both first-class.** Existing Android clients are GPG-only; gpm also opens age-encrypted gopass stores — age is gopass's modern alternative to GPG. Full list, read, write, and sync for both backends (native x25519, SSH, age-plugin recipients; and GPG/OpenPGP). New stores are age-only; existing GPG stores are opened as-is. No system `gpg` required.
- **A modern Android client for gopass.** Tauri v2 + Rust — biometric unlock, on-device encryption via the Android Keystore, and a native feel. No Termux, no CLI.
- **Private by design.** No cloud, no analytics, no accounts. Secrets sync over **git to a repo you control**. A password is decrypted and copied entirely on the Rust side and never reaches the WebView, and decrypted material is wiped after every use.
- **Fully open source.** Every line is public and auditable — no proprietary components, no black-box crypto. Dual-licensed under **MIT or Apache-2.0** (your choice): use it, audit it, fork it, self-host it.

## Security Model

- **Copy password never touches WebView** — decrypts and copies entirely on the Rust side
- **Show password auto-clear** — with page-leave cleanup
- **Zeroize-per-decrypt** — identity bytes wiped after every decrypt call
- **At-rest encryption** — on Android, the repo config and identity are sealed with AES-256-GCM under a Keystore key
- **Safe error messages** — no secrets in logs, errors, or toasts

See [SECURITY.md](docs/SECURITY.md) for the full threat model.

## Features

- Clone a gopass store from a Git URL (HTTPS + PAT or SSH key) — age or GPG/OpenPGP
- List and search entries (fuzzy, case-insensitive) with display names
- Copy a password (never reaches the WebView) or reveal it with auto-clear
- Create and edit secrets with gopass-compatible templates
- Browse a secret's past revisions and recover an old value
- Sync over git (fast-forward only), with optional auto-sync on every save and a per-entry keep-yours / keep-theirs resolve on collision
- Generate ed25519 SSH keys on-device, or import existing age / SSH / GPG keys
- Android: biometric unlock, screen-capture protection, background sync

## Contribution

Any help in the form of descriptive and friendly [issues](https://github.com/yzx9/gpm/issues) or
comprehensive pull requests are welcome!

Please check out [DEVELOPMENT.md](DEVELOPMENT.md) for guidelines.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
gpm by you, as defined in the Apache-2.0 license, shall be dual licensed under the terms of the
[License](#license) section below, without any additional terms or conditions.

Thanks goes to these wonderful people:

[![Contributors](https://contrib.rocks/image?repo=yzx9/gpm)](https://github.com/yzx9/gpm/graphs/contributors)

## License

This project is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
