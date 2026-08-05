// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

#![allow(missing_docs)]

const COMMANDS: &[&str] = &[
    "is_available",
    "retrieve",
    "store",
    "delete",
    // Biometric-gated (app-lock) path — the auth-free commands above cover the
    // permanent master key; these cover the biometric vault/legacy slots.
    "is_biometric_available",
    "has_stored_biometric",
    "store_biometric",
    "retrieve_biometric",
    "delete_biometric",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
