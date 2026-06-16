# Consumer ProGuard rules published to the app consuming this library.
#
# The plugin class is registered BY NAME from Rust
# (`register_android_plugin("xyz.yzx9.gpm.biometrickeystore", "KeystorePlugin")`),
# and its `@Command` methods are dispatched BY NAME
# (`run_mobile_plugin("store", ...)`). Static minification cannot see those
# string-based references, so the class and its command methods must be kept
# explicitly. `androidx.biometric` ships its own consumer rules and needs none
# here.

# The plugin class + its ctor + every @Command method.
-keep class xyz.yzx9.gpm.biometrickeystore.KeystorePlugin {
  public <init>(...);
  @app.tauri.annotation.Command <methods>;
}

# The @InvokeArg argument holder is deserialized by field name — keep all members.
-keep class xyz.yzx9.gpm.biometrickeystore.StoreArgs {
  *;
}
