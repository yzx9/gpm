// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

const COMMANDS: &[&str] = &["is_available", "store", "retrieve", "delete", "has_stored"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
