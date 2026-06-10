//
//  NativeWidgetKind.swift — the PURE, testable image of NativeView's dispatch.
//
//  `NativeView(_:)` (NativeView.swift) maps a UINode to a SwiftUI `AnyView`. An
//  `AnyView` is opaque, so it cannot be asserted on without a running SwiftUI
//  environment (a simulator / device). This file exposes the SAME per-node
//  decision as a plain enum so the mapping can be unit-tested headless — exactly
//  the brain/hands split the renderer already uses: the WHICH-WIDGET decision is
//  data, the SwiftUI draw is the dumb hands.
//
//  `nativeWidgetKind(_:)` MIRRORS `renderElement` in NativeView.swift 1:1:
//    button                              -> .button
//    img                                 -> .image
//    input / textarea                    -> .input
//    span/p/h1..h6/a/label/strong/em/... -> .text   (textTags)
//    div/section/main/nav/ul/li/form/... -> .stack   (blockTags)  )- both flow
//    any other / unknown element         -> .stack   (fallback)   )  children
//    .text(_)                            -> .text
//    .raw(_)                             -> .rawFallback (RawHTMLView)
//
//  This is NOT a second renderer — NativeView remains the single source of the
//  actual SwiftUI views; this enum is the assertable shadow of its dispatch.
//

import Foundation

/// The native SwiftUI widget family a `UINode` renders to, mirroring
/// `NativeView(_:)`. Used by tests/smoke to prove the IR -> SwiftUI mapping
/// without a running SwiftUI environment.
public enum NativeWidgetKind: Equatable {
    /// `Button(action:)` — a `<button>`.
    case button
    /// `AsyncImage` — an `<img>`.
    case image
    /// A bordered value/placeholder reflection — `<input>` / `<textarea>`.
    case input
    /// A SwiftUI `Text` (its text children flattened) — inline/heading tags and
    /// a bare `.text` node.
    case text
    /// A `VStack` flowing its children — block tags and the unknown fallback.
    case stack
    /// The `RawHTMLView` fallback — a `.raw` node.
    case rawFallback
}

/// Block-level tags that flow vertically (-> VStack). Mirrors `blockTags` in
/// NativeView.swift exactly.
private let nativeBlockTags: Set<String> = [
    "div", "section", "main", "nav", "header", "footer", "article",
    "aside", "ul", "ol", "li", "form", "fieldset", "figure"
]

/// Inline/text tags that render as `Text`. Mirrors `textTags` in NativeView.swift.
private let nativeTextTags: Set<String> = [
    "span", "p", "h1", "h2", "h3", "h4", "h5", "h6",
    "a", "label", "strong", "em", "b", "i", "small", "code", "pre", "blockquote"
]

/// Classify a `UINode` into the native widget family `NativeView` would render
/// it as. Pure; no SwiftUI required.
public func nativeWidgetKind(_ node: UINode) -> NativeWidgetKind {
    switch node {
    case .text:
        return .text
    case .raw:
        return .rawFallback
    case let .element(tag, _, _, _, _):
        let t = tag.lowercased()
        if t == "button" { return .button }
        if t == "img" { return .image }
        if t == "input" || t == "textarea" { return .input }
        if nativeTextTags.contains(t) { return .text }
        // block tags + unknown both flow children vertically (VStack).
        return .stack
    }
}

/// Best-effort visible text of a node: concatenates `.text` / stripped `.raw`
/// descendants. Mirrors `flattenText` in NativeView.swift (which the `Text`
/// branch uses to label inline elements), so a test can assert the label a deal
/// title / heading would draw.
public func nativeFlattenedText(_ node: UINode) -> String {
    var out = ""
    func walk(_ n: UINode) {
        switch n {
        case let .text(s): out += s
        case let .raw(h): out += nativeStripTags(h)
        case let .element(_, _, _, _, children):
            for c in children { walk(c) }
        }
    }
    walk(node)
    return out.trimmingCharacters(in: .whitespacesAndNewlines)
}

/// Small tag-stripper matching `stripTags` in NativeView.swift.
private func nativeStripTags(_ html: String) -> String {
    var out = ""
    var inTag = false
    for ch in html {
        if ch == "<" { inTag = true }
        else if ch == ">" { inTag = false }
        else if !inTag { out.append(ch) }
    }
    return out
}
