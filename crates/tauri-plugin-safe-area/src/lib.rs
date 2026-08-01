// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin exposing Android safe-area insets to the frontend.

use tauri::Runtime;
use tauri::plugin::{Builder, TauriPlugin};

/// Android package hosting the `SafeAreaPlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.safearea";

/// Initializes the safe-area plugin.
///
/// On Android, registers the Kotlin `SafeAreaPlugin` class.
/// On desktop, this is a no-op — frontend calls reject gracefully
/// and CSS `var()` fallbacks of `0px` apply.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("safe-area")
        .setup(|_app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            api.register_android_plugin(PLUGIN_IDENTIFIER, "SafeAreaPlugin")?;
            Ok(())
        })
        .build()
}
