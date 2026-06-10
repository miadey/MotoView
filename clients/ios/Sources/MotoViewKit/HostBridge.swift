//
//  HostBridge.swift — the native equivalent of the brain's ~12 host_* ABI.
//
//  On the web, the WASM brain calls a dozen `host_*` imports (the "hands") that
//  the JS glue implements: apply_html / replace_keyed / insert_keyed /
//  remove_keyed / move_keyed / effect / navigate / set_title / fetch / set_timer
//  / now / log (see client/src/abi.rs). On native there is no JS glue — the host
//  is the Swift app. This protocol is that contract, restated for a native shell.
//
//  A SwiftUI app conforms to `HostBridge` (or uses the provided
//  `StateHostBridge`) and the renderer / a future native brain loop drives it.
//  The methods take the SAME shapes as the web ABI so the brain logic is portable
//  1:1: targets are CSS-selector-ish ids, keyed ops carry (key, html, after).
//

import Foundation

/// The native host contract — the hands the native brain commands.
///
/// Method names and parameters intentionally mirror `abi::host` in the Rust
/// crate so a native polling brain (a future slice) can drive this exactly as
/// the WASM brain drives the JS glue.
public protocol HostBridge: AnyObject {
    // --- DOM/tree application (the keyed-op vocabulary) --------------------

    /// Replace the entire rendered tree of `target` with `node` (full plan).
    func applyTree(target: String, root: [UINode])

    /// Replace just the keyed region `key` inside `target` with `node`.
    func replaceKeyed(target: String, key: String, node: UINode)

    /// Insert `node` as a new keyed region after region `after` (nil = start).
    func insertKeyed(target: String, node: UINode, after: String?)

    /// Remove the keyed region `key` from `target`.
    func removeKeyed(target: String, key: String)

    /// Move the existing keyed region `key` to after region `after` (nil = start).
    func moveKeyed(target: String, key: String, after: String?)

    // --- side effects -----------------------------------------------------

    /// Run a named effect (focus/scroll/blur/...) on `target` with a value, the
    /// native analogue of `host_effect(kind, target, value)`.
    func effect(kind: String, target: String, value: String)

    /// Navigate to `url` (push a new route / open externally).
    func navigate(url: String)

    /// Set the window/scene title.
    func setTitle(_ title: String)

    // --- network / timing / diagnostics -----------------------------------

    /// Issue an HTTP request; the host calls back with the response keyed by
    /// `reqId` (the native analogue of `host_fetch`). Async — completion runs on
    /// the main actor.
    func fetch(reqId: UInt32, method: String, url: String, body: String,
               completion: @escaping (Int, String) -> Void)

    /// Schedule a one-shot timer; the host fires it after `ms` keyed by `timerId`.
    func setTimer(timerId: UInt32, ms: Double, fire: @escaping () -> Void)

    /// Monotonic-ish wall clock in ms (the native `host_now`).
    func now() -> Double

    /// Diagnostic log line (the native `host_log`).
    func log(_ message: String)
}

// MARK: - Applying a DiffPlan through a HostBridge

public extension HostBridge {
    /// Execute a `DiffPlan` produced by `MotoViewCore.irDiff`. For `.full`, the
    /// caller must re-parse+apply the new forest (we don't carry it here). For
    /// `.patch`, the ops carry their own HTML payloads, so we drive the keyed
    /// methods directly — but since the native renderer wants `UINode`, not HTML,
    /// the bridge re-parses each op's HTML through the core. Apps that render
    /// from `UINode` should prefer re-running `NativeView` on the new forest and
    /// using SwiftUI's own diffing; this hook exists for hosts that maintain an
    /// imperative keyed view registry (the closest native analogue to the DOM).
    func apply(plan: DiffPlan, to target: String, core: MotoViewCore) {
        switch plan {
        case .full:
            log("plan=full for \(target): caller must re-render the forest")
        case .patch(let ops):
            for op in ops {
                switch op {
                case .replace(let key, let html):
                    if let node = try? core.parseNode(htmlAsRawNode(html)) {
                        replaceKeyed(target: target, key: key, node: node)
                    }
                case .remove(let key):
                    removeKeyed(target: target, key: key)
                case .insert(let html, let after):
                    if let node = try? core.parseNode(htmlAsRawNode(html)) {
                        insertKeyed(target: target, node: node, after: after)
                    }
                case .move(let key, let after):
                    moveKeyed(target: target, key: key, after: after)
                }
            }
        }
    }

    /// Op HTML payloads are rendered HTML, not IR. Wrap them as a `raw` IR node
    /// so the existing `NativeView(.raw)` fallback container renders them. (A
    /// future slice can carry the IR subtree in the op instead of HTML, removing
    /// this wrap — see README "keyed ops carry HTML today".)
    private func htmlAsRawNode(_ html: String) -> String {
        // Build the canonical raw-node JSON the core's parseNode expects.
        let escaped = html
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n")
            .replacingOccurrences(of: "\r", with: "\\r")
            .replacingOccurrences(of: "\t", with: "\\t")
        return "{\"t\":\"raw\",\"html\":\"\(escaped)\"}"
    }
}
