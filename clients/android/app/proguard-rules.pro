# MotoView Android — ProGuard/R8 rules (Slice 11 scaffold).
#
# Keep the JNI bridge symbols the Rust core resolves by name. The Kotlin
# `external fun` declarations in dev.motoview.core.MotoViewNative must NOT be
# renamed, or the dynamic JNI lookup against libmotoview_client.so fails.
-keep class dev.motoview.core.** { *; }

# Play Integrity API uses reflection on its request/response models.
-keep class com.google.android.play.core.integrity.** { *; }
