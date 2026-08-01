# Consumer ProGuard rules published to the app consuming this library.
#
# The plugin class is registered BY NAME from Rust
# (`register_android_plugin("xyz.yzx9.gpm.screensecure", "ScreenSecurePlugin")`),
# and its `@Command` method is dispatched BY NAME. Static minification cannot
# see those string-based references, so the class, its command method, and the
# `@InvokeArg` holder (deserialized by field name) must be kept explicitly.

# The plugin class + its ctor + every @Command method.
-keep class xyz.yzx9.gpm.screensecure.ScreenSecurePlugin {
  public <init>(...);
  @app.tauri.annotation.Command <methods>;
}

# The @InvokeArg argument holder is deserialized by field name — keep all members.
-keep class xyz.yzx9.gpm.screensecure.SetSecureArgs {
  *;
}
