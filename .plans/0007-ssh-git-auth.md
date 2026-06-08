# 0007: Add SSH authentication for git operations

**Priority:** P3
**Status:** TODO
**Phase:** Future

## What

Support SSH key authentication for `git clone` and `git pull`, in addition to the current HTTPS + PAT method. Users would provide an SSH private key (or agent socket) instead of a PAT.

## Why

Many gopass repositories are hosted on SSH-only git servers, or users prefer SSH key auth over PATs. The current HTTPS-only restriction blocks these users from the setup flow entirely.

## Context

The current git implementation in `rustpass/src/git.rs` uses `git2` with HTTPS credential callbacks. SSH support requires `libssh2` (git2's SSH backend), which needs to be cross-compiled for Android NDK targets.

### Implementation approach

1. **Enable libssh2 in git2:** The `git2` crate supports SSH via libssh2 when the `ssh` feature is enabled. On most systems this requires libssh2 to be installed.

2. **NDK cross-compilation challenge:** libssh2 must be compiled for 4 Android targets (aarch64, armv7, x86_64, i686). This is the same approach used for the existing vendored OpenSSL setup in the Nix flake. Add libssh2 to `flake.nix` as a vendored dependency with NDK toolchain configuration.

3. **SSH key storage:** Accept SSH private key alongside (or instead of) PAT in the setup flow. Store in app-private storage (same as age identity). Consider Android Keystore integration (see 0008).

4. **SSH agent support (optional):** Forward SSH agent socket from host. Complex on Android, likely post-launch.

5. **Update SetupPage:** Allow user to choose between HTTPS (PAT) and SSH (private key) authentication.

### Key files

- `rustpass/src/git.rs` — Add SSH credential callback alongside HTTPS PAT callback
- `rustpass/src/config.rs` — Store SSH key alongside PAT
- `src/SetupPage.vue` — Add auth method toggle (HTTPS vs SSH)
- `flake.nix` — Add libssh2 as vendored dependency for Android cross-compilation
- `rustpass/Cargo.toml` — Enable `ssh` feature on git2 if needed

### Risks

- **NDK build complexity:** libssh2 cross-compilation is known to be finicky. The existing OpenSSL vendoring approach provides a template but debugging may take significant time.
- **git2 SSH support maturity:** git2's libssh2 backend works but may have edge cases with newer SSH key formats (e.g., sk-ssh-ed25519@openssh.com for hardware keys).

## Effort

~2-3 days (human) / ~1 hour (CC) — mostly NDK cross-compilation debugging

## Depends on

None — independent of other plans. Can be combined with 0008 (Android Keystore) if SSH key storage needs hardware protection.
