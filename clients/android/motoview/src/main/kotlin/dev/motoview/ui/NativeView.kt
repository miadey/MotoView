package dev.motoview.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import android.webkit.WebView
import dev.motoview.core.UINode

/*
 * NativeView.kt — the IR -> Jetpack Compose renderer, the Android sibling of the
 * iOS NativeView.swift. NO WebView for normal nodes; only the `Raw` fallback uses
 * an AndroidView(WebView) for literal HTML the IR can't model natively.
 *
 * Mapping (1:1 with the SwiftUI renderer):
 *   div/section/main/nav/ul/ol/li -> Column
 *   span/p/h1..h6/a/label/strong/em/text -> Text (styled per tag)
 *   button -> Button(onClick = emit(event))
 *   raw -> AndroidView { WebView } fallback container
 *   keyed elements -> Compose `key(node.key)` to preserve identity across reorder
 *
 * SCAFFOLD ONLY — NOT BUILT HERE (no Android SDK/NDK). Compiles under a real
 * Android/Gradle toolchain; see CARGO_NDK.md.
 */

class EventContext(val event: String, val handler: String, val args: Map<String, String>)

private val blockTags = setOf(
    "div", "section", "main", "nav", "header", "footer", "article",
    "aside", "ul", "ol", "li", "form", "fieldset", "figure"
)
private val textTags = setOf(
    "span", "p", "h1", "h2", "h3", "h4", "h5", "h6",
    "a", "label", "strong", "em", "b", "i", "small", "code", "pre", "blockquote"
)

@Composable
fun NativeForest(forest: List<UINode>, emit: (EventContext) -> Unit = {}) {
    Column {
        forest.forEach { NativeView(it, emit) }
    }
}

@Composable
fun NativeView(node: UINode, emit: (EventContext) -> Unit = {}) {
    when (node) {
        is UINode.Text -> Text(node.value)
        is UINode.Raw -> RawHtmlView(node.html)
        is UINode.Element -> {
            val tag = node.tag.lowercase()
            // Keyed elements preserve identity across reorders, the native
            // equivalent of data-mv-key keyed-region preservation.
            androidx.compose.runtime.key(node.key ?: tag) {
                renderElement(tag, node, emit)
            }
        }
    }
}

@Composable
private fun renderElement(tag: String, node: UINode.Element, emit: (EventContext) -> Unit) {
    when {
        tag == "button" -> Button(onClick = {
            val handler = node.event("click") ?: node.events.firstOrNull()?.value ?: ""
            val args = node.attrs.filter { it.name.startsWith("data-mv-arg") }
                .associate { it.name to it.value }
            emit(EventContext("click", handler, args))
        }) {
            Column { node.children.forEach { NativeView(it, emit) } }
        }
        tag in textTags -> {
            val weight = when (tag) {
                "h1", "h2", "h3", "h4", "h5", "h6", "strong", "b" -> FontWeight.Bold
                else -> FontWeight.Normal
            }
            Text(flattenText(node.children), fontWeight = weight)
        }
        tag in blockTags -> Column { node.children.forEach { NativeView(it, emit) } }
        else -> Column { node.children.forEach { NativeView(it, emit) } }
    }
}

private fun flattenText(nodes: List<UINode>): String {
    val sb = StringBuilder()
    for (n in nodes) when (n) {
        is UINode.Text -> sb.append(n.value)
        is UINode.Raw -> sb.append(stripTags(n.html))
        is UINode.Element -> sb.append(flattenText(n.children))
    }
    return sb.toString()
}

private fun stripTags(html: String): String {
    val sb = StringBuilder()
    var inTag = false
    for (c in html) when {
        c == '<' -> inTag = true
        c == '>' -> inTag = false
        !inTag -> sb.append(c)
    }
    return sb.toString()
}

/** The `Raw` fallback: a real WebView for literal HTML the IR can't model. */
@Composable
fun RawHtmlView(html: String, modifier: Modifier = Modifier.padding(0.dp)) {
    AndroidView(
        modifier = modifier,
        factory = { ctx -> WebView(ctx) },
        update = { webView ->
            webView.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
        }
    )
}
