# SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
#
# SPDX-License-Identifier: Apache-2.0
#
# R8/ProGuard keep rules for the biometric KeystorePlugin.
#
# The plugin class is registered BY NAME from Rust
# (`register_android_plugin("xyz.yzx9.gpm", "KeystorePlugin")`), and its
# `@Command` methods are dispatched BY NAME (`run_mobile_plugin("store", ...)`).
# Static minification cannot see those string-based references, so the class and
# its command methods must be kept explicitly. `androidx.biometric` ships its
# own consumer rules and needs no keep rules here.

# The plugin class + its no-arg/ctor + every @Command method.
-keep class xyz.yzx9.gpm.KeystorePlugin {
  public <init>(...);
  @app.tauri.annotation.Command <methods>;
}

# The @InvokeArg argument holder is deserialized by field name — keep all members.
-keep class xyz.yzx9.gpm.StoreArgs {
  *;
}
