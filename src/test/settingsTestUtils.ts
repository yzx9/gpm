// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared fixtures for the settings sub-page tests (`SettingsPage` hub +
 * `SettingsGeneralPage` / `SettingsLockingPage` / `SettingsIdentityPage` /
 * `SettingsRepositoryPage`). Each test file owns its own `vi.mock` boilerplate
 * (vitest hoists `vi.mock` above imports, so the mock factories cannot reference
 * imported helpers); this module supplies only the data those per-file mocks
 * layer on top of, plus the per-command override/install helpers.
 *
 * The invoke-mock posture mirrors the original monolithic `SettingsPage.test.ts`:
 * `defaults` is the order-independent success return per command, `overrides`
 * holds per-test `{ value }` or `{ reject }` entries, and `installInvokeMock`
 * wires the layered resolver onto the auto-mocked `invoke`.
 */

/** SSH-auth repo config (the URL + auth-type display tests key off this). */
export const sshConfig = {
  url: "git@github.com:user/repo.git",
  pat: null,
  ssh_key:
    "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n-----END OPENSSH PRIVATE KEY-----",
  ssh_passphrase: null,
  local_path: "/tmp/repo",
};

/** HTTPS + PAT repo config. */
export const httpsConfig = {
  url: "https://github.com/user/repo.git",
  pat: "ghp_token123",
  ssh_key: null,
  ssh_passphrase: null,
  local_path: "/tmp/repo",
};

/**
 * Default successful return values per command (order-independent). The
 * AppConfig behavior prefs are omitted so they take their serde defaults
 * (immediate / default clears / autosync on / app-lock off) — the same posture
 * as a fresh install.
 */
export const baseDefaults: Record<string, unknown> = {
  get_config: httpsConfig,
  get_app_config: { secure_screen: true },
  // App-launch biometric gate reads Keystore truth (Path B), not the flag.
  get_app_lock_state: { enabled: false, locked: false },
  get_auth_state: {
    configured: true,
    encrypted: false,
    unlocked: false,
    identity_type: "x25519",
  },
  is_biometric_available: false,
  is_biometric_unlock_enabled: false,
  get_authenticity_config: {
    mode: "off",
    trusted_keys: [],
    trusted_gpg_keys: [],
    ignored: [],
  },
  get_gpg_key_parse_warnings: [],
  get_commit_identity_default: { name: "gpm", email: "gpm@local" },
  get_ssh_public_key: { public_key: "ssh-ed25519 default" },
  export_ssh_private_key: { private_key: "default-private" },
};

/** Per-command override: a value to resolve, or `{ reject: payload }`. */
export type Overrides = Record<string, { value?: unknown; reject?: unknown }>;

/** Set a successful return value for `cmd` (cleared per-test by the caller). */
export function when(overrides: Overrides, cmd: string, value: unknown): void {
  overrides[cmd] = { value };
}

/** Make `cmd` reject with `payload`. */
export function reject(
  overrides: Overrides,
  cmd: string,
  payload: unknown,
): void {
  overrides[cmd] = { reject: payload };
}

/** Clear every override key (used in `beforeEach`). */
export function resetOverrides(overrides: Overrides): void {
  for (const k of Object.keys(overrides)) delete overrides[k];
}
