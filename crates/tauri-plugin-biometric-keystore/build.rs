// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

#![allow(missing_docs)]

const COMMANDS: &[&str] = &[
    "is_available",
    "open_security_settings",
    "store",
    "retrieve",
    "delete",
    "has_stored",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
