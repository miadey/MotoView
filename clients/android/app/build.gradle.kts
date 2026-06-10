// :app — the shippable MotoView Android APPLICATION module (Slice 11).
//
// SCAFFOLD ONLY — NOT BUILT HERE (no Android SDK/NDK on the build machine).
// This module turns the :motoview library (Compose renderer + Rust core) into a
// store-uploadable App Bundle and carries the Gradle Play Publisher config that
// pushes `bundleRelease` to the Play internal track.
//
// ----------------------------------------------------------------------------
// EXTERNAL PREREQUISITES (user must supply — none exist on the build machine):
//   * Android SDK + NDK + ANDROID_HOME, and the Rust .so produced by cargo-ndk
//     (see ../CARGO_NDK.md) under :motoview/src/main/jniLibs/<abi>/.
//   * A Google Play Console account ($25 one-time) with this applicationId
//     already registered (the FIRST bundle must be uploaded MANUALLY once; the
//     plugin can only update an existing app listing).
//   * A Play service-account JSON with the "Release to testing tracks" /
//     "Release apps to production" permission, provided to CI as the secret
//     PLAY_SERVICE_ACCOUNT_JSON (written to a file the plugin reads).
//   * An upload keystore (.jks) for release signing, provided to CI as base64
//     (ANDROID_KEYSTORE_BASE64) plus ANDROID_KEYSTORE_PASSWORD /
//     ANDROID_KEY_ALIAS / ANDROID_KEY_PASSWORD. NO keystore is committed.
//   * Play Integrity API enabled for this app (see ../PLAY_INTEGRITY.md).
// ----------------------------------------------------------------------------

plugins {
    id("com.android.application") version "8.5.0"
    id("org.jetbrains.kotlin.android") version "2.0.0"
    // Gradle Play Publisher — `publishBundle` / `publishReleaseBundle` tasks.
    id("com.github.triplet.play") version "3.10.1"
}

android {
    namespace = "dev.motoview.app"
    compileSdk = 34

    defaultConfig {
        applicationId = "dev.motoview.app"
        minSdk = 26
        targetSdk = 34
        versionCode = (System.getenv("ANDROID_VERSION_CODE") ?: "1").toInt()
        versionName = System.getenv("ANDROID_VERSION_NAME") ?: "0.1.0"
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    // Release signing pulled ENTIRELY from the environment so NO keystore or
    // password is committed. CI decodes ANDROID_KEYSTORE_BASE64 to upload.jks.
    signingConfigs {
        create("release") {
            val ksPath = System.getenv("ANDROID_KEYSTORE_PATH")
            if (ksPath != null) {
                storeFile = file(ksPath)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            // Only attach the signing config when a keystore was supplied, so a
            // bare `assembleRelease` (unsigned) still configures on a dev box.
            if (System.getenv("ANDROID_KEYSTORE_PATH") != null) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    buildFeatures {
        compose = true
    }
    composeOptions {
        kotlinCompilerExtensionVersion = "1.5.14"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

// ----------------------------------------------------------------------------
// Gradle Play Publisher config.
//   * serviceAccountCredentials: the JSON key file CI writes from the
//     PLAY_SERVICE_ACCOUNT_JSON secret (path via PLAY_JSON_FILE).
//   * track "internal": ship `bundleRelease` to the internal testing track;
//     promote to alpha/beta/production from the console or a separate lane.
//   * defaultToAppBundles: upload an .aab (required by Play), not an .apk.
// Run: ./gradlew :app:publishReleaseBundle
// ----------------------------------------------------------------------------
play {
    val playJson = System.getenv("PLAY_JSON_FILE")
    if (playJson != null) {
        serviceAccountCredentials.set(file(playJson))
    }
    track.set(System.getenv("PLAY_TRACK") ?: "internal")
    defaultToAppBundles.set(true)
    // "completed" makes the upload live on its track; use "draft" for a manual
    // review gate in the console.
    releaseStatus.set(
        com.github.triplet.gradle.androidpublisher.ReleaseStatus.COMPLETED
    )
}

dependencies {
    implementation(project(":motoview"))

    val composeBom = platform("androidx.compose:compose-bom:2024.06.00")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.9.0")
    implementation("androidx.core:core-ktx:1.13.1")

    // Play Integrity API — server-verified device/app attestation (stub wiring
    // documented in ../PLAY_INTEGRITY.md). Pulled here so the app can request a
    // token; the verdict is verified server-side (canister/backend), not on box.
    implementation("com.google.android.play:integrity:1.4.0")

    testImplementation("junit:junit:4.13.2")
}
