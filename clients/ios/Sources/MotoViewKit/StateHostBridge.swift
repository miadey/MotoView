//
//  StateHostBridge.swift — a concrete, in-memory HostBridge.
//
//  A reference implementation of the native host contract that records what the
//  brain commanded (instead of mutating a real view hierarchy). Useful for:
//   * tests (assert the brain issued the right keyed ops),
//   * a starting point for a real SwiftUI app shell (replace the records with
//     actual @Published view state and a keyed view registry).
//
//  This is the native sibling of the JS glue object in the web client — but it
//  holds NO decision logic. Every decision was made in Rust; this only executes.
//

import Foundation

/// Records the effects the brain requested. Thread-confined to the main actor in
/// a real app; here it is a plain class for test inspection.
public final class StateHostBridge: HostBridge {
    public struct Record: Equatable {
        public let kind: String
        public let detail: String
    }

    public private(set) var records: [Record] = []
    public private(set) var title: String = ""
    public private(set) var route: String = "/"
    private let clock: () -> Double

    public init(clock: @escaping () -> Double = { Date().timeIntervalSince1970 * 1000 }) {
        self.clock = clock
    }

    private func record(_ kind: String, _ detail: String) {
        records.append(Record(kind: kind, detail: detail))
    }

    public func applyTree(target: String, root: [UINode]) {
        record("applyTree", "\(target) <- \(root.count) node(s)")
    }
    public func replaceKeyed(target: String, key: String, node: UINode) {
        record("replaceKeyed", "\(target)[\(key)]")
    }
    public func insertKeyed(target: String, node: UINode, after: String?) {
        record("insertKeyed", "\(target) after=\(after ?? "<start>")")
    }
    public func removeKeyed(target: String, key: String) {
        record("removeKeyed", "\(target)[\(key)]")
    }
    public func moveKeyed(target: String, key: String, after: String?) {
        record("moveKeyed", "\(target)[\(key)] after=\(after ?? "<start>")")
    }
    public func effect(kind: String, target: String, value: String) {
        record("effect", "\(kind) \(target)=\(value)")
    }
    public func navigate(url: String) {
        route = url
        record("navigate", url)
    }
    public func setTitle(_ title: String) {
        self.title = title
        record("setTitle", title)
    }
    public func fetch(reqId: UInt32, method: String, url: String, body: String,
                      completion: @escaping (Int, String) -> Void) {
        record("fetch", "\(method) \(url)")
        // No network in the reference bridge; a real shell uses URLSession.
        completion(0, "")
    }
    public func setTimer(timerId: UInt32, ms: Double, fire: @escaping () -> Void) {
        record("setTimer", "#\(timerId) +\(ms)ms")
    }
    public func now() -> Double { clock() }
    public func log(_ message: String) { record("log", message) }
}
