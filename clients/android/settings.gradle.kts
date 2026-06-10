// MotoView Android client — Slice 8 scaffold + Slice 11 :app/Play Publisher.
//
// SCAFFOLD ONLY — NOT BUILT ON THIS MACHINE (no Android SDK/NDK/ANDROID_HOME,
// no cargo-ndk). This wires:
//   :motoview — the Compose-renderer LIBRARY (links the Rust core .so)
//   :app      — the shippable APPLICATION (App Bundle + Gradle Play Publisher)
// A real Android dev runs `./gradlew :app:bundleRelease` (or
// `:app:publishReleaseBundle` to push to the Play internal track) once they have
// the SDK/NDK and have run the cargo-ndk step in CARGO_NDK.md to drop
// libmotoview_client.so into motoview/src/main/jniLibs/<abi>/.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "motoview-android"
include(":motoview")
include(":app")
