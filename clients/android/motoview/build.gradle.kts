// :motoview — the MotoView Android client library (Compose renderer + Rust core).
//
// SCAFFOLD ONLY — NOT BUILT HERE. Requires the Android SDK/NDK and the Rust .so
// produced by cargo-ndk (see ../CARGO_NDK.md). The `.so` goes in
// src/main/jniLibs/<abi>/libmotoview_client.so; Gradle packages it into the AAR.

plugins {
    id("com.android.library") version "8.5.0"
    id("org.jetbrains.kotlin.android") version "2.0.0"
}

android {
    namespace = "dev.motoview"
    compileSdk = 34

    defaultConfig {
        minSdk = 26 // arm64 + modern WebView; matches the Rust aarch64 target
        ndk {
            // The Rust core ships arm64 first; add others as cargo-ndk emits them.
            abiFilters += listOf("arm64-v8a")
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

    // libmotoview_client.so is delivered under src/main/jniLibs/<abi>/ by
    // cargo-ndk (CARGO_NDK.md). Gradle auto-packages jniLibs into the AAR.
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.06.00")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.core:core-ktx:1.13.1")

    testImplementation("junit:junit:4.13.2")
}
