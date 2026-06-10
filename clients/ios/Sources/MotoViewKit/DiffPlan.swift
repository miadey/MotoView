//
//  DiffPlan.swift — the Swift image of the keyed-diff Plan/Op vocabulary.
//
//  Mirrors `client/src/diff.rs`'s `Plan` and `Op`. The Rust core computes the
//  plan (the LIS keyed reconcile); the native HostBridge executes the ops, the
//  same brain/hands split the web client uses (decisions in Rust, mutation in
//  the host).
//

import Foundation

/// A primitive op the HostBridge executes against a keyed target. `after` is the
/// key to position after, or `nil` for the start of the parent.
public enum DiffOp: Equatable {
    case replace(key: String, html: String)
    case remove(key: String)
    case insert(html: String, after: String?)
    case move(key: String, after: String?)
}

/// What to do with a freshly rendered target.
public enum DiffPlan: Equatable {
    /// Replace the whole target (no stable keyed structure to exploit).
    case full
    /// Apply these ops in order.
    case patch([DiffOp])

    static func fromJSONObject(_ any: Any) throws -> DiffPlan {
        guard let obj = any as? [String: Any], let plan = obj["plan"] as? String else {
            throw MotoViewError.malformed("plan missing 'plan'")
        }
        switch plan {
        case "full":
            return .full
        case "patch":
            let opsAny = obj["ops"] as? [Any] ?? []
            let ops = try opsAny.map { try DiffOp.fromJSONObject($0) }
            return .patch(ops)
        default:
            throw MotoViewError.malformed("unknown plan '\(plan)'")
        }
    }
}

extension DiffOp {
    static func fromJSONObject(_ any: Any) throws -> DiffOp {
        guard let obj = any as? [String: Any], let op = obj["op"] as? String else {
            throw MotoViewError.malformed("op missing 'op'")
        }
        func str(_ k: String) throws -> String {
            guard let v = obj[k] as? String else { throw MotoViewError.malformed("op missing '\(k)'") }
            return v
        }
        // `after` is `null` in JSON for the start position -> Swift `nil`.
        let after = obj["after"] as? String
        switch op {
        case "replace": return .replace(key: try str("key"), html: try str("html"))
        case "remove":  return .remove(key: try str("key"))
        case "insert":  return .insert(html: try str("html"), after: after)
        case "move":    return .move(key: try str("key"), after: after)
        default:        throw MotoViewError.malformed("unknown op '\(op)'")
        }
    }
}
