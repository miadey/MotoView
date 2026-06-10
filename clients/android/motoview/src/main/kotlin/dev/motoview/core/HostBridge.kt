package dev.motoview.core

/*
 * HostBridge.kt — the native equivalent of the brain's ~12 host_* ABI, the
 * Android sibling of the iOS HostBridge.swift. A Compose app conforms to this (or
 * uses StateHostBridge) and the renderer / a future native brain drives it. No
 * decision logic lives here — every decision was made in Rust.
 */

interface HostBridge {
    // DOM/tree application (keyed-op vocabulary)
    fun applyTree(target: String, root: List<UINode>)
    fun replaceKeyed(target: String, key: String, node: UINode)
    fun insertKeyed(target: String, node: UINode, after: String?)
    fun removeKeyed(target: String, key: String)
    fun moveKeyed(target: String, key: String, after: String?)

    // side effects
    fun effect(kind: String, target: String, value: String)
    fun navigate(url: String)
    fun setTitle(title: String)

    // network / timing / diagnostics
    fun fetch(reqId: Int, method: String, url: String, body: String, completion: (Int, String) -> Unit)
    fun setTimer(timerId: Int, ms: Double, fire: () -> Unit)
    fun now(): Double
    fun log(message: String)
}

/** A reference, in-memory HostBridge for tests / a shell starting point. */
class StateHostBridge : HostBridge {
    data class Record(val kind: String, val detail: String)

    val records = mutableListOf<Record>()
    var title: String = ""
        private set
    var route: String = "/"
        private set

    private fun rec(kind: String, detail: String) { records.add(Record(kind, detail)) }

    override fun applyTree(target: String, root: List<UINode>) = rec("applyTree", "$target <- ${root.size} node(s)")
    override fun replaceKeyed(target: String, key: String, node: UINode) = rec("replaceKeyed", "$target[$key]")
    override fun insertKeyed(target: String, node: UINode, after: String?) = rec("insertKeyed", "$target after=${after ?: "<start>"}")
    override fun removeKeyed(target: String, key: String) = rec("removeKeyed", "$target[$key]")
    override fun moveKeyed(target: String, key: String, after: String?) = rec("moveKeyed", "$target[$key] after=${after ?: "<start>"}")
    override fun effect(kind: String, target: String, value: String) = rec("effect", "$kind $target=$value")
    override fun navigate(url: String) { route = url; rec("navigate", url) }
    override fun setTitle(title: String) { this.title = title; rec("setTitle", title) }
    override fun fetch(reqId: Int, method: String, url: String, body: String, completion: (Int, String) -> Unit) {
        rec("fetch", "$method $url"); completion(0, "")
    }
    override fun setTimer(timerId: Int, ms: Double, fire: () -> Unit) = rec("setTimer", "#$timerId +${ms}ms")
    override fun now(): Double = System.currentTimeMillis().toDouble()
    override fun log(message: String) = rec("log", message)
}
