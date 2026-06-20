// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use serde::Serialize;

/// A password store entry (no secret data).
///
/// Aligned with gopass's listed entries. Each entry corresponds to a `.age`
/// file discovered in the store directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// Relative path from repo root (e.g., `"cloud/aws/root.age"`).
    pub path: String,
    /// Display name (e.g., `"aws/root"`) — extension stripped, forward slashes.
    pub name: String,
}
