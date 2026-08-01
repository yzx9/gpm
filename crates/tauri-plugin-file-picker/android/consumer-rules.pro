# Consumer ProGuard rules published to the app consuming this library.
# The plugin class is registered by name from Rust and its @Command methods are
# dispatched by name; tauri-android already ships keep rules covering classes
# annotated with @TauriPlugin / @Command, so no per-plugin rules are needed
# here (this plugin has no @InvokeArg argument holders).
