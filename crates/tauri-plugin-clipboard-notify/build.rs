// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(missing_docs)]

const COMMANDS: &[&str] = &[
    "are_notifications_enabled",
    "request_notifications_permission",
    "open_app_notification_settings",
    "post_clipboard_notification",
    "dismiss_clipboard_notification",
    "consume_manual_clear_flag",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
