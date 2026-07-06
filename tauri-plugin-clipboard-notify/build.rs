// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

const COMMANDS: &[&str] = &[
    "are_notifications_enabled",
    "request_notifications_permission",
    "post_clipboard_notification",
    "dismiss_clipboard_notification",
    "consume_manual_clear_flag",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
