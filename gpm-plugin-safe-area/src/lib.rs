// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin exposing Android safe-area insets to the frontend.

use tauri::Runtime;
use tauri::plugin::{Builder, TauriPlugin};

/// Initializes the safe-area plugin.
///
/// On Android, registers the Kotlin `SafeAreaPlugin` class.
/// On desktop, this is a no-op — frontend calls reject gracefully
/// and CSS `var()` fallbacks of `0px` apply.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("safe-area")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            _api.register_android_plugin("xyz.yzx9.gpm", "SafeAreaPlugin")?;
            Ok(())
        })
        .build()
}
