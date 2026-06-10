//
//  NativeView.swift — the IR -> SwiftUI renderer. NO WebView.
//
//  This is the heart of the native client: a recursive function mapping the
//  portable UI-IR (UINode, produced by the REAL Rust parser) to native SwiftUI
//  views. One MotoView source compiles to an IR forest; this renders that forest
//  as real SwiftUI — VStack/Text/Button/etc — not an HTML string in a WebView.
//
//  Mapping (mirrors the Android Compose `NativeView` composable 1:1):
//    div / section / main / nav / ul / ol / li -> VStack (vertical flow)
//    span / p / h1..h6 / a / label / strong / em / text -> Text
//    button -> Button(action: emit(event)) { ... }
//    input / textarea -> a bordered Text reflection of the value (host binds it)
//    img -> AsyncImage (by src attr)
//    raw -> RawHTMLView fallback container (renders stripped text; a real app can
//           swap in a WKWebView for true HTML — flagged, needs Xcode)
//    keyed elements -> wrapped with .id(key) so SwiftUI preserves identity, the
//                      native equivalent of data-mv-key keyed-region preservation
//
//  IMPLEMENTATION NOTE: every branch is erased to `AnyView` at the boundary and
//  each tag family is a small concrete helper. This keeps the Swift type-checker
//  linear — a single deeply-nested `@ViewBuilder` switch over heterogeneous
//  branches type-checks super-linearly and can stall the compiler for minutes.
//

#if canImport(SwiftUI)
import SwiftUI

/// An event sink: when a `button`/interactive node fires, the renderer calls this
/// with the IR event name and handler id (e.g. ("click","pick")) plus the node's
/// `data-mv-arg*` attributes, so the host can dispatch the event-as-update.
public struct EventContext {
    public let event: String
    public let handler: String
    public let args: [String: String]
    public init(event: String, handler: String, args: [String: String]) {
        self.event = event
        self.handler = handler
        self.args = args
    }
}

/// The renderer's environment: how to emit events. The brand theme (Slice 7) is
/// applied separately via `ThemedForest`/`.accentColor` (see Theme.swift).
public struct RenderEnv {
    public let emit: (EventContext) -> Void
    public init(emit: @escaping (EventContext) -> Void = { _ in }) {
        self.emit = emit
    }
}

/// Block-level tags that flow vertically (-> VStack).
private let blockTags: Set<String> = [
    "div", "section", "main", "nav", "header", "footer", "article",
    "aside", "ul", "ol", "li", "form", "fieldset", "figure"
]

/// Inline/text tags that render as `Text` (their text children concatenated).
private let textTags: Set<String> = [
    "span", "p", "h1", "h2", "h3", "h4", "h5", "h6",
    "a", "label", "strong", "em", "b", "i", "small", "code", "pre", "blockquote"
]

/// Render a single `UINode` to SwiftUI (type-erased). Recursive; keyed elements
/// get `.id(key)`. Returns `AnyView` so the type-checker stays linear.
public func NativeView(_ node: UINode, env: RenderEnv = RenderEnv()) -> AnyView {
    switch node {
    case .text(let s):
        return AnyView(Text(s))
    case .raw(let html):
        return AnyView(RawHTMLView(html: html))
    case .element(let tag, let attrs, let events, let key, let children):
        let view = renderElement(
            tag: tag.lowercased(), attrs: attrs, events: events,
            children: children, env: env
        )
        if let key = key {
            return AnyView(view.id(key))
        }
        return view
    }
}

/// Render a whole forest (top-level nodes) into a vertical stack.
public struct NativeForest: View {
    private let forest: [UINode]
    private let env: RenderEnv
    public init(_ forest: [UINode], env: RenderEnv = RenderEnv()) {
        self.forest = forest
        self.env = env
    }
    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(forest.enumerated()), id: \.offset) { item in
                NativeView(item.element, env: env)
            }
        }
    }
}

// MARK: - element dispatch (each branch is a small, concretely-typed helper)

private func renderElement(tag: String,
                           attrs: [UINode.Attr],
                           events: [UINode.Attr],
                           children: [UINode],
                           env: RenderEnv) -> AnyView {
    if tag == "button" {
        return AnyView(ButtonNode(attrs: attrs, events: events, children: children, env: env))
    }
    if tag == "img" {
        return AnyView(ImageNode(src: attrs.first(where: { $0.name == "src" })?.value))
    }
    if tag == "input" || tag == "textarea" {
        let value = attrs.first(where: { $0.name == "value" })?.value ?? ""
        let placeholder = attrs.first(where: { $0.name == "placeholder" })?.value ?? ""
        return AnyView(InputNode(value: value, placeholder: placeholder))
    }
    if textTags.contains(tag) {
        return AnyView(TextNode(tag: tag, text: flattenText(children)))
    }
    // block tags + unknown fallback both flow children vertically.
    return AnyView(ChildStack(children: children, env: env))
}

// MARK: - concrete node views

private struct ChildStack: View {
    let children: [UINode]
    let env: RenderEnv
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(children.enumerated()), id: \.offset) { item in
                NativeView(item.element, env: env)
            }
        }
    }
}

private struct ButtonNode: View {
    let attrs: [UINode.Attr]
    let events: [UINode.Attr]
    let children: [UINode]
    let env: RenderEnv
    var body: some View {
        Button(action: fire) {
            ChildStack(children: children, env: env)
        }
    }
    private func fire() {
        let handler = events.first(where: { $0.name == "click" })?.value
            ?? events.first?.value ?? ""
        var args: [String: String] = [:]
        for a in attrs where a.name.hasPrefix("data-mv-arg") {
            args[a.name] = a.value
        }
        env.emit(EventContext(event: "click", handler: handler, args: args))
    }
}

private struct ImageNode: View {
    let src: String?
    var body: some View {
        if let src = src, let url = URL(string: src) {
            AsyncImage(url: url) { image in
                image.resizable().scaledToFit()
            } placeholder: {
                Color.gray.opacity(0.1)
            }
        } else {
            EmptyView()
        }
    }
}

private struct InputNode: View {
    let value: String
    let placeholder: String
    // A read-only reflection of the input's value. A real app binds this to host
    // state via the HostBridge (the two-way binding lives in the app shell).
    var body: some View {
        Text(value.isEmpty ? placeholder : value)
            .foregroundColor(value.isEmpty ? .secondary : .primary)
            .padding(6)
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.3)))
    }
}

private struct TextNode: View {
    let tag: String
    let text: String
    var body: some View {
        styled(Text(text))
    }
    private func styled(_ t: Text) -> Text {
        switch tag {
        case "h1": return t.font(.largeTitle).bold()
        case "h2": return t.font(.title).bold()
        case "h3": return t.font(.title2).bold()
        case "h4", "h5", "h6": return t.font(.headline)
        case "strong", "b": return t.bold()
        case "em", "i": return t.italic()
        case "code", "pre": return t.font(.system(.body, design: .monospaced))
        default: return t
        }
    }
}

/// The `.raw` fallback container. For now it renders the raw HTML as plain
/// stripped text. A production app swaps a WKWebView in here for true HTML
/// rendering — that needs UIKit/WebKit and a real app target (flagged in README;
/// not buildable on this Command-Line-Tools-only machine).
public struct RawHTMLView: View {
    public let html: String
    public init(html: String) { self.html = html }
    public var body: some View {
        Text(stripTags(html))
            .font(.system(.body, design: .default))
            .foregroundColor(.secondary)
    }
}

// MARK: - text flattening

/// Flatten a subtree's `.text`/`.raw` descendants into a single string for inline
/// text elements (so `<p>Hello <b>world</b></p>` becomes "Hello world").
private func flattenText(_ nodes: [UINode]) -> String {
    var out = ""
    for n in nodes {
        switch n {
        case .text(let s): out += s
        case .raw(let h): out += stripTags(h)
        case .element(_, _, _, _, let children): out += flattenText(children)
        }
    }
    return out
}

/// Very small tag-stripper for raw HTML flattened into a Text run.
private func stripTags(_ html: String) -> String {
    var out = ""
    var inTag = false
    for ch in html {
        if ch == "<" { inTag = true }
        else if ch == ">" { inTag = false }
        else if !inTag { out.append(ch) }
    }
    return out
}

#endif
