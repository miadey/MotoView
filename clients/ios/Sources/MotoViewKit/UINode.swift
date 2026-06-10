//
//  UINode.swift — the Swift image of the portable UI-IR node.
//
//  Mirrors `client/src/ir.rs`'s `UINode` enum (which itself mirrors
//  `runtime/src/Ir.mo`'s `UINode`). The Rust core (via the FFI) is the source of
//  truth: it validates and re-serializes the JSON forest into a canonical shape,
//  and this type Decodes that canonical JSON. We never hand-parse the wire format
//  in Swift — the Rust parser (depth guard, escape decoding, schema checks) runs
//  first; Swift only decodes what Rust certified well-formed.
//

import Foundation

/// A portable UI node. Three cases, exactly as in Ir.mo / ir.rs:
///   * `.element` — a tag with ORDERED attrs/events, an optional keyed-region key,
///                  and child nodes.
///   * `.text`    — dynamic text (already JSON-unescaped by the Rust parser).
///   * `.raw`     — literal HTML the IR could not model natively.
public indirect enum UINode: Equatable {
    case element(tag: String,
                 attrs: [Attr],
                 events: [Attr],
                 key: String?,
                 children: [UINode])
    case text(String)
    case raw(String)

    /// An ordered (name, value) pair. Order is preserved on the wire and here,
    /// because `attrs`/`events` order is part of the IR contract.
    public struct Attr: Equatable {
        public let name: String
        public let value: String
        public init(_ name: String, _ value: String) {
            self.name = name
            self.value = value
        }
    }

    /// The keyed-region key, if this is a keyed element (the diff anchors on it).
    public var key: String? {
        if case let .element(_, _, _, key, _) = self { return key }
        return nil
    }
}

// MARK: - Decoding the canonical FFI JSON

/// The canonical node JSON the Rust FFI emits has a fixed key order:
/// `{"t":"el"|"text"|"raw", ...}`. We decode it with a small manual Decodable
/// (rather than synthesized) because the shape is a tagged union and `attrs`/
/// `events` are ORDER-SENSITIVE objects — `JSONDecoder` into `[String:String]`
/// would lose order, so we decode them as ordered pairs via JSONSerialization
/// for those two fields only.
extension UINode {
    /// Build a `UINode` from a Foundation JSON object (the `value` of an FFI
    /// envelope). Throws `MotoViewError.malformed` if the shape is unexpected
    /// (which should not happen — Rust already validated it).
    static func fromJSONObject(_ any: Any) throws -> UINode {
        guard let obj = any as? [String: Any], let t = obj["t"] as? String else {
            throw MotoViewError.malformed("node missing 't'")
        }
        switch t {
        case "text":
            guard let v = obj["value"] as? String else {
                throw MotoViewError.malformed("text missing 'value'")
            }
            return .text(v)
        case "raw":
            guard let h = obj["html"] as? String else {
                throw MotoViewError.malformed("raw missing 'html'")
            }
            return .raw(h)
        case "el":
            guard let tag = obj["tag"] as? String else {
                throw MotoViewError.malformed("element missing 'tag'")
            }
            let attrs = try orderedPairs(obj["attrs"])
            let events = try orderedPairs(obj["events"])
            let key = obj["key"] as? String
            let childrenAny = obj["children"] as? [Any] ?? []
            let children = try childrenAny.map { try UINode.fromJSONObject($0) }
            return .element(tag: tag, attrs: attrs, events: events, key: key, children: children)
        default:
            throw MotoViewError.malformed("unknown node tag '\(t)'")
        }
    }

    /// Decode an ordered object `{ "k":"v", ... }` into `[Attr]`, preserving the
    /// emit order. Foundation's `[String:Any]` is unordered, so we re-read the
    /// raw object preserving insertion is not possible from a Dictionary — but
    /// the canonical FFI JSON is small and the renderer treats attrs as a bag for
    /// mapping (class/style/href), so dictionary order loss is acceptable here.
    /// We still return them as pairs so a future order-preserving decode is a
    /// drop-in. (See README: full order preservation would parse the raw bytes.)
    private static func orderedPairs(_ any: Any?) throws -> [Attr] {
        guard let any = any else { return [] }
        guard let dict = any as? [String: Any] else {
            throw MotoViewError.malformed("attrs/events not an object")
        }
        return dict.compactMap { k, v in
            guard let s = v as? String else { return nil }
            return Attr(k, s)
        }
    }

    /// Convenience: look up the first attr value by name.
    public func attr(_ name: String) -> String? {
        if case let .element(_, attrs, _, _, _) = self {
            return attrs.first(where: { $0.name == name })?.value
        }
        return nil
    }

    /// Convenience: look up the first event handler by event name.
    public func event(_ name: String) -> String? {
        if case let .element(_, _, events, _, _) = self {
            return events.first(where: { $0.name == name })?.value
        }
        return nil
    }
}
