//
//  main.swift — the host smoke test for the MotoView native core.
//
//  Runs on THIS machine (aarch64-apple-darwin) via `swift run motoview-smoke`.
//  It exercises the REAL Rust core through the C ABI:
//    1. parses the EXACT Ir.mo builder-forest golden JSON into a UINode tree and
//       asserts the tree shape (proving the Rust parser ran across the FFI),
//    2. runs a keyed ir_diff and asserts a Replace op,
//    3. calls verify_response with garbage and asserts it fails CLOSED (named
//       error, no crash) — the chain-key verifier path is reached.
//  Exits non-zero on any assertion failure so CI / the build script can gate.
//

import Foundation
import MotoViewKit

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write(Data("SMOKE FAIL: \(msg)\n".utf8))
    exit(1)
}

let core = MotoViewCore()

// 1) Parse the EXACT builder-forest golden (matches ir.rs / IrTest.mo).
let goldenForest = """
[{"t":"el","tag":"section","attrs":{},"events":{},"children":[\
{"t":"el","tag":"h1","attrs":{},"events":{},"children":[\
{"t":"raw","html":"Hi "},\
{"t":"text","value":"Ada & Co"}]},\
{"t":"el","tag":"li","attrs":{},"events":{},"key":"it-1","children":[\
{"t":"el","tag":"button","attrs":{"data-mv-arg0":"it-1"},"events":{"click":"pick"},"children":[\
{"t":"text","value":"it-1"}]}]}]}]
"""

let forest: [UINode]
do {
    forest = try core.parseForest(goldenForest)
} catch {
    fail("parseForest threw: \(error)")
}

guard forest.count == 1 else { fail("expected 1 top node, got \(forest.count)") }
guard case let .element(tag, _, _, key, children) = forest[0] else {
    fail("top node not an element")
}
guard tag == "section" else { fail("top tag = \(tag), want section") }
guard key == nil else { fail("section should have no key") }
guard children.count == 2 else { fail("section children = \(children.count), want 2") }

// child[0] = <h1> with raw "Hi " + text "Ada & Co"
guard case let .element(h1tag, _, _, _, h1kids) = children[0], h1tag == "h1" else {
    fail("child0 not <h1>")
}
guard case .raw("Hi ") = h1kids[0] else { fail("h1 child0 not raw 'Hi '") }
guard case .text("Ada & Co") = h1kids[1] else { fail("h1 child1 not text 'Ada & Co'") }

// child[1] = keyed <li key=it-1> wrapping a <button click=pick>
guard case let .element(litag, _, _, liKey, liKids) = children[1],
      litag == "li", liKey == "it-1" else {
    fail("child1 not keyed <li it-1>")
}
guard case let .element(btag, battrs, bevents, _, _) = liKids[0], btag == "button" else {
    fail("li child not <button>")
}
guard bevents.contains(where: { $0.name == "click" && $0.value == "pick" }) else {
    fail("button missing click=pick event")
}
guard battrs.contains(where: { $0.name == "data-mv-arg0" && $0.value == "it-1" }) else {
    fail("button missing data-mv-arg0=it-1")
}
print("[1/3] parseForest -> UINode tree asserted OK (\(forest.count) top node)")

// 2) Keyed ir_diff: same key, changed body -> Replace.
let oldF = """
[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"A"}]}]
"""
let newF = """
[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"B"}]}]
"""
let plan: DiffPlan
do {
    plan = try core.irDiff(old: oldF, new: newF)
} catch {
    fail("irDiff threw: \(error)")
}
guard case let .patch(ops) = plan, ops.count == 1, case let .replace(k, _) = ops[0], k == "a" else {
    fail("irDiff plan not [Replace key=a]: \(plan)")
}
print("[2/3] irDiff -> Patch[Replace key=a] asserted OK")

// 3) verify_response must FAIL CLOSED on garbage (named error, no crash).
let garbageCert = Data([0, 1, 2, 3, 4, 5, 6, 7])
let canister = Data(repeating: 0, count: 10)
let body = Data([1, 2, 3])
let path = [Data("canister".utf8), canister, Data("certified_data".utf8)]
do {
    let t = try core.verifyResponse(cert: garbageCert, canister: canister,
                                    path: path, body: body,
                                    nowNs: UInt128Pair.nanos())
    fail("verifyResponse unexpectedly succeeded on garbage: time_ns=\(t)")
} catch let MotoViewError.core(reason) {
    // Expected: a named CertError variant (fails closed).
    print("[3/3] verifyResponse failed closed on garbage as expected: \(reason)")
} catch {
    fail("verifyResponse threw unexpected error: \(error)")
}

print("SMOKE OK: Rust core reached over the C ABI; IR parse + diff + cert-verify all exercised.")
