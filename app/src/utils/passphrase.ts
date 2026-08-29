// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

/**
 * Map a form's optional passphrase to its IPC shape. The passphrase is an
 * opaque secret: it crosses byte-verbatim, never rewritten (a trim would
 * silently re-key a credential whose real passphrase carries surrounding
 * whitespace, making it forever unverifiable). Whitespace-only input counts
 * as "left blank" and maps to `null` — the same blank-as-absent rule the
 * backend's `normalize_ssh_credential` applies on the persistence side.
 */
export function optionalPassphrase(value: string): string | null {
  return value.trim() === "" ? null : value;
}
