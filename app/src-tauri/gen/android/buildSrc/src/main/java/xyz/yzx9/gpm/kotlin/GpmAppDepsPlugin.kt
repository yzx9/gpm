import com.android.build.api.dsl.ApplicationExtension
import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.kotlin.dsl.configure

/**
 * Convention plugin (`gpm.app.deps`) that adds `:app`-only AndroidX +
 * Robolectric test dependencies and the Robolectric test options that Tauri's
 * regenerated `app/build.gradle.kts` can't carry. Applied to `:app` from
 * `settings.gradle`'s `gradle.beforeProject` hook (Tauri re-renders
 * `app/build.gradle.kts` on every `tauri android build`, dropping manual edits
 * — R077/D8).
 *
 * Uses `pluginManager.withPlugin("com.android.application")` so it is safe to
 * apply before AGP: the `android` extension and the `implementation`/
 * `testImplementation` configurations are touched only once AGP has applied,
 * regardless of apply order.
 */
open class GpmAppDepsPlugin : Plugin<Project> {
    override fun apply(project: Project) {
        project.pluginManager.withPlugin("com.android.application") {
            project.extensions.configure<ApplicationExtension> {
                testOptions {
                    unitTests {
                        isIncludeAndroidResources = true
                    }
                }
            }
            val deps = project.dependencies
            // WorkManager: the app-owned SyncWorker extends CoroutineWorker.
            deps.add("implementation", "androidx.work:work-runtime-ktx:2.9.0")
            // Robolectric JVM tests for the app source set (HeadlessBootstrap).
            deps.add("testImplementation", "org.robolectric:robolectric:4.14.1")
            deps.add("testImplementation", "androidx.test:core:1.6.1")
            deps.add("testImplementation", "junit:junit:4.13.2")
        }
    }
}
