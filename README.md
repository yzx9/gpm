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

- Clone a gopass age-encrypted password store from a Git URL (HTTPS + PAT)
- List all `.age` entries with display names
- Search entries by name (frontend filtering, case-insensitive)
- Copy password to clipboard (password never reaches WebView)
- View password with 30-second auto-clear and page-leave cleanup
- View notes metadata
- Pull updates (fast-forward only) from the remote repo

## Tech Stack

| Layer           | Technology                                                           |
| --------------- | -------------------------------------------------------------------- |
| App framework   | Tauri v2                                                             |
| Backend         | Rust (age, git2, zeroize, walkdir)                                   |
| Frontend        | Vue 3 + TypeScript + Vite                                            |
| Crypto          | [age](https://github.com/str4d/rage) (Rust reference implementation) |
| Clipboard       | tauri-plugin-clipboard-manager                                       |
| Package manager | pnpm                                                                 |

## Getting Started

### Prerequisites

- [nix](https://nixos.org/) (or manually: Rust, Node.js 22, pnpm)
- [direnv](https://direnv.net/) (optional, for nix shell)

### Install dependencies

```bash
# With nix + direnv
direnv allow

# Install frontend dependencies
pnpm install
```

### Development

```bash
# Desktop dev mode
pnpm tauri dev

# Run Rust tests (14 tests)
cargo test --manifest-path src-tauri/Cargo.toml
```

### Android (Phase 3)

```bash
pnpm tauri android init
pnpm tauri android dev
```

## Project Structure

```
gpm/
├── src/                          # Vue 3 frontend
│   ├── main.ts                   # Router + app entry
│   ├── App.vue                   # Root component
│   ├── types.ts                  # Tauri IPC type definitions
│   └── pages/
│       ├── SetupPage.vue         # Git URL + PAT + identity → clone
│       ├── EntryListPage.vue     # List, search, copy, pull
│       └── EntryDetailPage.vue   # Show password (30s auto-clear)
├── src-tauri/                    # Rust backend
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── src/
│   │   ├── lib.rs                # Tauri commands + app entry
│   │   ├── crypto.rs             # Age decryption
│   │   ├── store.rs              # Directory walking, content parsing
│   │   ├── git.rs                # Clone + pull (ff-only)
│   │   ├── secure_storage.rs     # Identity + config storage
│   │   └── error.rs              # Safe error types
│   └── tests/
│       └── fixtures.rs           # 14 integration tests
├── flake.nix                     # Nix dev shell
└── package.json                  # Frontend dependencies
```

## Rust Command API

| Command         | Description                              | Secrets cross IPC?  |
| --------------- | ---------------------------------------- | ------------------- |
| `setup`         | Clone repo + save identity + config      | No                  |
| `list_entries`  | Walk repo, return `.age` entries         | No                  |
| `pull_repo`     | Fetch + fast-forward merge               | No                  |
| `copy_password` | Decrypt → clipboard → zeroize            | **No** (primary op) |
| `show_password` | Decrypt → return to Vue (30s auto-clear) | Yes (secondary op)  |
| `is_configured` | Check if setup is complete               | No                  |
| `get_config`    | Return repo URL + path                   | No                  |
| `reset_config`  | Clear all local data                     | No                  |

## License

See [LICENSE](./LICENSE).
