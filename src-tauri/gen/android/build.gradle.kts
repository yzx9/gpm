buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:8.11.0")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:1.9.25")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

tasks.register("clean").configure {
    delete("build")
}

// Aggregated JVM unit-test gate for the local Android plugins. Fans out across
// every local plugin subproject (matched by `tauri-plugin-` name + projectDir
// under this repo root, which excludes the upstream `tauri-android` and
// `tauri-plugin-clipboard-manager` whose projectDir lives in the cargo registry).
// The plugin subprojects are `include`d only via the generated, gitignored
// `tauri.settings.gradle` (settings.gradle does `apply from:` it); when that file
// is absent Gradle fails at configuration, and `just test-plugin`/CI guard the
// file before invoking Gradle. The doFirst fails loud if, despite the file being
// present, zero local plugins matched (a stale/partial generation) — a silent
// no-op pass is the exact failure mode this gate exists to prevent.
tasks.register("testPlugins") {
    // Local plugins: name starts with `tauri-plugin-` and the module lives in this
    // repo — NOT pulled from the cargo registry like the upstream
    // `tauri-plugin-clipboard-manager` and `tauri-android`. (rootDir is the gradle
    // root under gen/android, but the plugin modules live outside it at
    // ../../tauri-plugin-*/android, so a projectDir-under-rootDir check would
    // match nothing — exclude by cargo-registry location instead.)
    val matched = subprojects.filter { p ->
        p.name.startsWith("tauri-plugin-") &&
            !p.projectDir.canonicalPath.contains("/.cargo/registry/")
    }
    matched.forEach { p -> dependsOn("${p.path}:testDebugUnitTest") }
    doFirst {
        if (matched.isEmpty()) {
            throw GradleException(
                "testPlugins: no local plugin subprojects configured. " +
                    "tauri.settings.gradle is missing or stale — run a `tauri android build/dev` to regenerate it."
            )
        }
    }
}

