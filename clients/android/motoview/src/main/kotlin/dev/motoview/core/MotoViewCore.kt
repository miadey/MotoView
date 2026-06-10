package dev.motoview.core

import org.json.JSONArray
import org.json.JSONObject

/*
 * MotoViewCore.kt — the Kotlin facade over the Rust core, the Android sibling of
 * the iOS MotoViewCore.swift.
 *
 * SCAFFOLD ONLY — NOT BUILT ON THIS MACHINE. There is no Android NDK /
 * ANDROID_HOME / cargo-ndk here, so the `.so` is not produced and this module is
 * not compiled. See clients/android/CARGO_NDK.md for the exact build steps.
 *
 * Binding strategy: the Rust crate exposes a flat C ABI (mv_ffi_*), not JNI-named
 * symbols. Two supported options, both documented in CARGO_NDK.md:
 *   (A) UniFFI — add the uniffi proc-macro to client/, `cargo run --bin
 *       uniffi-bindgen generate ... --language kotlin`, and the generated Kotlin
 *       replaces these `external` declarations with a typed binding. (Preferred
 *       once uniffi-bindgen is on PATH.)
 *   (B) A tiny JNI shim (jni_bridge.c, ~30 lines) that re-exports each mv_ffi_*
 *       as a `Java_dev_motoview_core_MotoViewNative_*` symbol, built into the
 *       same `libmotoview_client.so` by cargo-ndk. The `external fun`s below
 *       match that shim's names.
 *
 * Either way the surface is identical to iOS: parse a JSON IR forest into a
 * UINode tree, diff two forests, render to HTML, and verify a response against
 * the pinned NNS root key — all decided in Rust.
 */

/** JNI entry points implemented by the Rust core's JNI shim (see CARGO_NDK.md). */
internal object MotoViewNative {
    init {
        // Loads libmotoview_client.so from src/main/jniLibs/<abi>/ (placed there
        // by cargo-ndk). Throws UnsatisfiedLinkError if the .so is absent — which
        // it is on this machine, by design (scaffold not built here).
        System.loadLibrary("motoview_client")
    }

    external fun parseForest(json: String): String
    external fun parseNode(json: String): String
    external fun renderForest(json: String): String
    external fun irDiff(oldJson: String, newJson: String): String
    external fun verifyResponse(
        cert: ByteArray,
        canister: ByteArray,
        path: ByteArray,
        body: ByteArray,
        nowNsHi: Long,
        nowNsLo: Long,
        maxOffsetNs: Long
    ): String
}

class MotoViewError(message: String) : Exception(message)

/** Stateless facade — every call is a pure function over the Rust FFI. */
object MotoViewCore {

    private fun envelopeValue(raw: String): Any {
        val obj = JSONObject(raw)
        if (obj.optBoolean("ok", false)) {
            return obj.get("value")
        }
        throw MotoViewError(obj.optString("error", "unknown"))
    }

    fun parseForest(json: String): List<UINode> {
        val value = envelopeValue(MotoViewNative.parseForest(json)) as JSONArray
        return (0 until value.length()).map { UINode.fromJson(value.getJSONObject(it)) }
    }

    fun parseNode(json: String): UINode {
        val value = envelopeValue(MotoViewNative.parseNode(json)) as JSONObject
        return UINode.fromJson(value)
    }

    fun renderForest(json: String): String =
        envelopeValue(MotoViewNative.renderForest(json)) as String

    fun irDiff(oldJson: String, newJson: String): DiffPlan {
        val value = envelopeValue(MotoViewNative.irDiff(oldJson, newJson)) as JSONObject
        return DiffPlan.fromJson(value)
    }

    /**
     * Verify an IC response against the PINNED NNS root key (mainnet). Returns the
     * certified /time (ns) as a String (u128 in Rust). Throws on failure (fails
     * closed). `path` segments are length-prefixed [u32_le len][bytes]... here.
     */
    fun verifyResponse(
        cert: ByteArray,
        canister: ByteArray,
        path: List<ByteArray>,
        body: ByteArray,
        nowNsHi: Long = 0,
        nowNsLo: Long,
        maxOffsetNs: Long = 0
    ): String {
        val pathBuf = encodePath(path)
        val value = envelopeValue(
            MotoViewNative.verifyResponse(cert, canister, pathBuf, body, nowNsHi, nowNsLo, maxOffsetNs)
        ) as JSONObject
        return value.getString("time_ns")
    }

    private fun encodePath(path: List<ByteArray>): ByteArray {
        val out = ArrayList<Byte>()
        for (seg in path) {
            val n = seg.size
            out.add((n and 0xFF).toByte())
            out.add(((n shr 8) and 0xFF).toByte())
            out.add(((n shr 16) and 0xFF).toByte())
            out.add(((n shr 24) and 0xFF).toByte())
            seg.forEach { out.add(it) }
        }
        return out.toByteArray()
    }
}
