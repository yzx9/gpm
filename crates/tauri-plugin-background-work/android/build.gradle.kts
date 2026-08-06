plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "xyz.yzx9.gpm.backgroundwork"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }

    testOptions {
        unitTests {
            isIncludeAndroidResources = true
        }
    }
}

dependencies {
    implementation(project(":tauri-android"))

    // WorkManager: periodic work scheduling.
    implementation("androidx.work:work-runtime-ktx:2.9.0")

    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("androidx.test:core:1.6.1")
    testImplementation("junit:junit:4.13.2")
    // Jackson, to test the JSON → @InvokeArg contract (the IPC shape a wrong
    // workerClassName field-name would silently null). Same version as
    // tauri-api's own jackson-databind. Test-only.
    testImplementation("com.fasterxml.jackson.core:jackson-databind:2.15.3")
}
