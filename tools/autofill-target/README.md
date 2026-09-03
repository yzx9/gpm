# autofill-target

A minimal standalone target app for manually verifying gpm's autofill
service. An autofill provider cannot fill its own package, so the repo
carries this deterministic target: two native `EditText`s declaring exactly
the username/password autofill hints the gpm MVP detects (no WebView, no
hint-less fields — those are later-phase scope).

Not wired into the gpm build (`just` recipes or the Tauri gradle tree) —
build it when needed.

## Build & install

No gradle wrapper is checked in; use your system Gradle or open the folder
in Android Studio (the AGP/Kotlin versions match `app/src-tauri/gen/android`).

```sh
cd tools/autofill-target
gradle :app:assembleDebug
adb install app/build/outputs/apk/debug/app-debug.apk
```

## Use

1. Enable gpm as the system autofill service (debug build):
   `adb shell settings put secure selected_autofill_service \
xyz.yzx9.gpm.debug/xyz.yzx9.gpm.GpmAutofillService`
2. Cold-start gpm's process (`adb shell am force-stop xyz.yzx9.gpm.debug`),
   open Autofill Target, and tap the **Username** field.
3. Expect one "gpm" row in the OS autofill dropdown → tap → the gpm fill
   activity lists every entry → filter → pick → both fields fill.

Reset afterwards with
`adb shell settings put secure selected_autofill_service null`.
