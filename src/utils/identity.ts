// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Identity type classification — mirrors Rust's `identity::IdentityType`.
 */
export type IdentityType =
  | "x25519"
  | "ssh_ed25519"
  | "ssh_rsa"
  | "age_encrypted"
  | "post_quantum"
  | "unknown";

/**
 * Classify the type of an age identity from its string content.
 * Non-validating — prefix-based detection only.
 */
export function classifyIdentity(text: string): IdentityType {
  const trimmed = text.trim();

  if (trimmed.startsWith("AGE-SECRET-KEY-PQ-1")) return "post_quantum";
  if (trimmed.startsWith("AGE-SECRET-KEY-")) return "x25519";
  if (trimmed.startsWith("-----BEGIN AGE ENCRYPTED FILE-----"))
    return "age_encrypted";
  if (trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----"))
    return "ssh_ed25519";
  if (trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----")) return "ssh_rsa";
  return "unknown";
}

/**
 * Check if an identity string looks like a valid identity (not encrypted, not unknown).
 */
export function isValidIdentity(text: string): boolean {
  const type = classifyIdentity(text);
  return type === "x25519" || type === "ssh_ed25519" || type === "ssh_rsa";
}
