package dev.motoview.core

import org.json.JSONArray
import org.json.JSONObject

/*
 * UINode.kt — the Kotlin image of the portable UI-IR node (mirrors ir.rs /
 * Ir.mo and the iOS UINode.swift). Decoded from the canonical JSON the Rust FFI
 * emits; never hand-parsed in Kotlin.
 */

data class Attr(val name: String, val value: String)

sealed class UINode {
    data class Element(
        val tag: String,
        val attrs: List<Attr>,
        val events: List<Attr>,
        val key: String?,
        val children: List<UINode>
    ) : UINode()

    data class Text(val value: String) : UINode()
    data class Raw(val html: String) : UINode()

    val key: String?
        get() = (this as? Element)?.key

    fun attr(name: String): String? =
        (this as? Element)?.attrs?.firstOrNull { it.name == name }?.value

    fun event(name: String): String? =
        (this as? Element)?.events?.firstOrNull { it.name == name }?.value

    companion object {
        fun fromJson(obj: JSONObject): UINode = when (obj.getString("t")) {
            "text" -> Text(obj.getString("value"))
            "raw" -> Raw(obj.getString("html"))
            "el" -> Element(
                tag = obj.getString("tag"),
                attrs = pairs(obj.optJSONObject("attrs")),
                events = pairs(obj.optJSONObject("events")),
                key = if (obj.has("key")) obj.getString("key") else null,
                children = children(obj.optJSONArray("children"))
            )
            else -> throw MotoViewError("unknown node tag '${obj.getString("t")}'")
        }

        private fun pairs(o: JSONObject?): List<Attr> {
            if (o == null) return emptyList()
            return o.keys().asSequence().map { Attr(it, o.getString(it)) }.toList()
        }

        private fun children(arr: JSONArray?): List<UINode> {
            if (arr == null) return emptyList()
            return (0 until arr.length()).map { fromJson(arr.getJSONObject(it)) }
        }
    }
}
