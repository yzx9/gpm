// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

/**
 * Identity type classification — mirrors Rust's `identity::IdentityType`.
 * R085: the union is generated from the Rust enum by `just gen-codegen`.
 */
import type { IdentityType } from "@/api/generated/rustpass";

export type { IdentityType };

/**
 * Classify the type of an age identity from its string content.
 * Non-validating — prefix-based detection only.
 */
export function classifyIdentity(text: string): IdentityType {
  const trimmed = text.trim();

  if (trimmed.startsWith("AGE-SECRET-KEY-PQ-1")) return "post_quantum";
  if (trimmed.startsWith("AGE-PLUGIN-")) return "plugin";
  if (trimmed.startsWith("AGE-SECRET-KEY-")) return "x25519";
  if (trimmed.startsWith("-----BEGIN AGE ENCRYPTED FILE-----"))
    return "age_encrypted";
  if (trimmed.startsWith("-----BEGIN PGP PRIVATE KEY BLOCK-----"))
    return "pgp_secret_key";
  if (trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----"))
    return "ssh_ed25519";
  if (trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----")) return "ssh_rsa";
  if (trimmed.startsWith("-----BEGIN PRIVATE KEY-----")) return "ssh_ed25519";
  return "unknown";
}

/**
 * Check if an identity string looks like a valid identity (not encrypted, not unknown).
 */
export function isValidIdentity(text: string): boolean {
  const type = classifyIdentity(text);
  // Mirrors Rust `validate_identity_format`: accepts native x25519, SSH
  // (ed25519/RSA, incl. PKCS#8 which classifies as ssh_ed25519), and armored
  // PGP secret keys. Rejects age-encrypted, plugin, post-quantum, unknown.
  return (
    type === "x25519" ||
    type === "ssh_ed25519" ||
    type === "ssh_rsa" ||
    type === "pgp_secret_key"
  );
}
