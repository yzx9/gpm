// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin toggling Android `FLAG_SECURE` for per-page screen-capture
//! protection. The frontend calls `set_secure(bool)` per route (blocked when
//! the user's master toggle is on and the route is sensitive).

use tauri::Runtime;
use tauri::plugin::{Builder, TauriPlugin};

/// Android package hosting the `ScreenSecurePlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.screensecure";

/// Initializes the screen-secure plugin.
///
/// On Android, registers the Kotlin `ScreenSecurePlugin` that toggles
/// `WindowManager.LayoutParams.FLAG_SECURE` on the host activity's window.
/// On desktop, this is a no-op — the frontend gates calls on the app's
/// `screen_secure_available()` command (which returns `false` off-Android),
/// so no invoke ever reaches a command that does not exist.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("screen-secure")
        .setup(|_app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            api.register_android_plugin(PLUGIN_IDENTIFIER, "ScreenSecurePlugin")?;
            Ok(())
        })
        .build()
}
