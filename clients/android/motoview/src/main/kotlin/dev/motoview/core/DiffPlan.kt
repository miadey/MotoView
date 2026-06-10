package dev.motoview.core

import org.json.JSONObject

/*
 * DiffPlan.kt — the Kotlin image of the keyed-diff Plan/Op vocabulary
 * (mirrors diff.rs and the iOS DiffPlan.swift). The Rust core computes the plan;
 * the HostBridge executes the ops.
 */

sealed class DiffOp {
    data class Replace(val key: String, val html: String) : DiffOp()
    data class Remove(val key: String) : DiffOp()
    data class Insert(val html: String, val after: String?) : DiffOp()
    data class Move(val key: String, val after: String?) : DiffOp()

    companion object {
        fun fromJson(obj: JSONObject): DiffOp {
            val after = if (obj.isNull("after")) null else obj.optString("after", null)
            return when (obj.getString("op")) {
                "replace" -> Replace(obj.getString("key"), obj.getString("html"))
                "remove" -> Remove(obj.getString("key"))
                "insert" -> Insert(obj.getString("html"), after)
                "move" -> Move(obj.getString("key"), after)
                else -> throw MotoViewError("unknown op '${obj.getString("op")}'")
            }
        }
    }
}

sealed class DiffPlan {
    object Full : DiffPlan()
    data class Patch(val ops: List<DiffOp>) : DiffPlan()

    companion object {
        fun fromJson(obj: JSONObject): DiffPlan = when (obj.getString("plan")) {
            "full" -> Full
            "patch" -> {
                val arr = obj.optJSONArray("ops")
                val ops = if (arr == null) emptyList()
                else (0 until arr.length()).map { DiffOp.fromJson(arr.getJSONObject(it)) }
                Patch(ops)
            }
            else -> throw MotoViewError("unknown plan '${obj.getString("plan")}'")
        }
    }
}
