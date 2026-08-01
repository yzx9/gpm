# Consumer ProGuard rules published to the app consuming this library.
#
# The plugin class is registered BY NAME from Rust
# (`register_android_plugin("xyz.yzx9.gpm.clipboardnotify", "ClipboardNotifyPlugin")`),
# and its `@Command` methods are dispatched BY NAME. Static minification cannot
# see those string-based references, so the class, its command methods, the
# `@InvokeArg` holders (deserialized by field name), and the BroadcastReceiver
# (referenced by class when registered dynamically) must be kept explicitly.

# The plugin class + its ctor + every @Command method.
-keep class xyz.yzx9.gpm.clipboardnotify.ClipboardNotifyPlugin {
  public <init>(...);
  @app.tauri.annotation.Command <methods>;
}

# @InvokeArg holders are deserialized by field name — keep all members.
-keep class xyz.yzx9.gpm.clipboardnotify.PostClipboardNotificationArgs {
  *;
}

# The BroadcastReceiver is registered dynamically and referenced by class.
-keep class xyz.yzx9.gpm.clipboardnotify.ClipboardClearReceiver {
  public <init>(...);
  *;
}
