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
//    4. CROSS-PLATFORM PROOF: loads the COMMITTED CRM UI-IR forest fixture (the
//       exact `motoview preview examples/crm` output — the same forest the web
//       canister was Playwright-driven against), parses it through the REAL Rust
//       core, and asserts the SwiftUI mapping: the "+ New deal" button -> Button,
//       the kanban columns/cards -> VStacks, the deal titles -> Text. This proves
//       the SwiftUI renderer (NativeView) would draw the CRM from the one source.
//       (Launching an iOS SIMULATOR is out of scope — full Xcode absent; this is
//       the forest -> SwiftUI widget MAPPING that NativeView performs.)
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
    print("[3/4] verifyResponse failed closed on garbage as expected: \(reason)")
} catch {
    fail("verifyResponse threw unexpected error: \(error)")
}

// 4) CROSS-PLATFORM PROOF — the CRM forest renders on SwiftUI from the ONE source.
//
// Load the COMMITTED fixture (Tests/Fixtures/crm.forest.json), parse it through
// the REAL Rust core, then assert NativeView's SwiftUI mapping via the pure,
// assertable shadow `nativeWidgetKind` (which mirrors NativeView.renderElement
// 1:1). Resolve the fixture relative to THIS source file so cwd doesn't matter.

func crmFixtureURL() -> URL {
    // main.swift lives at clients/ios/Sources/MotoViewSmoke/main.swift; the
    // fixture is at clients/ios/Tests/Fixtures/crm.forest.json.
    let here = URL(fileURLWithPath: #filePath)
    return here
        .deletingLastPathComponent()      // .../Sources/MotoViewSmoke
        .deletingLastPathComponent()      // .../Sources
        .deletingLastPathComponent()      // .../ios
        .appendingPathComponent("Tests/Fixtures/crm.forest.json")
}

let crmURL = crmFixtureURL()
guard let crmJSON = try? String(contentsOf: crmURL, encoding: .utf8) else {
    fail("could not read CRM fixture at \(crmURL.path)")
}

let crmForest: [UINode]
do {
    crmForest = try core.parseForest(crmJSON)
} catch {
    fail("parseForest(CRM) threw: \(error)")
}
guard !crmForest.isEmpty else { fail("CRM forest is empty") }

// Walk helper over the parsed tree.
func crmWalk(_ nodes: [UINode], _ visit: (UINode) -> Void) {
    for n in nodes {
        visit(n)
        if case let .element(_, _, _, _, children) = n {
            crmWalk(children, visit)
        }
    }
}

func attrValue(_ node: UINode, _ name: String) -> String? {
    if case let .element(_, attrs, _, _, _) = node {
        return attrs.first(where: { $0.name == name })?.value
    }
    return nil
}

func clickHandler(_ node: UINode) -> String? {
    if case let .element(tag, _, events, _, _) = node, tag == "button" {
        return events.first(where: { $0.name == "click" })?.value
    }
    return nil
}

// (a) It IS the CRM board: the "Pipeline" h1, 4 kanban-col <section>s, 6
//     deal-card <article>s, and the "+ New deal" button (click=toggleCreate).
var h1Text: String? = nil
var kanbanCols = 0
var dealCards = 0
var dealTitles: [String] = []
var newDealButtons: [UINode] = []
var cardButtons: [UINode] = []   // removeDeal / moveBack / moveFwd

crmWalk(crmForest) { node in
    guard case let .element(tag, _, _, _, _) = node else { return }
    if tag == "h1" { h1Text = nativeFlattenedText(node) }
    let cls = attrValue(node, "class")
    if cls == "kanban-col" {
        kanbanCols += 1
        if tag != "section" { fail("kanban-col should be a <section>, got <\(tag)>") }
    }
    if cls == "deal-card" {
        dealCards += 1
        if tag != "article" { fail("deal-card should be an <article>, got <\(tag)>") }
    }
    if cls == "deal-title" { dealTitles.append(nativeFlattenedText(node)) }
    switch clickHandler(node) {
    case "toggleCreate": newDealButtons.append(node)
    case "removeDeal", "moveBack", "moveFwd": cardButtons.append(node)
    default: break
    }
}

guard h1Text == "Pipeline" else { fail("CRM h1 must read 'Pipeline', got \(h1Text ?? "nil")") }
guard kanbanCols == 4 else { fail("expected 4 kanban columns, got \(kanbanCols)") }
guard dealCards == 6 else { fail("expected 6 deal cards, got \(dealCards)") }
guard dealTitles.count == 6 else { fail("expected 6 deal titles, got \(dealTitles)") }
guard dealTitles.contains("Website redesign") else {
    fail("expected the 'Website redesign' deal, got \(dealTitles)")
}
guard newDealButtons.count == 1 else { fail("expected one '+ New deal' button, got \(newDealButtons.count)") }
guard nativeFlattenedText(newDealButtons[0]) == "+ New deal" else {
    fail("toggleCreate button label != '+ New deal'")
}
guard cardButtons.count == 18 else {
    // 6 cards x (removeDeal + moveBack + moveFwd) = 18 interactive controls.
    fail("expected 18 per-card action buttons, got \(cardButtons.count)")
}

// (b) The SwiftUI MAPPING (NativeView's dispatch, via the assertable shadow):
//     - the "+ New deal" + every per-card action button -> .button (Button)
//     - the kanban columns + deal cards -> .stack (VStack)
//     - the deal titles + the h1 -> .text (Text)
guard nativeWidgetKind(newDealButtons[0]) == .button else {
    fail("'+ New deal' must map to a SwiftUI Button")
}
for b in cardButtons where nativeWidgetKind(b) != .button {
    fail("a per-card action <button> did not map to a SwiftUI Button")
}
crmWalk(crmForest) { node in
    let cls = attrValue(node, "class")
    if cls == "kanban-col" || cls == "kanban-col-body" || cls == "deal-card" {
        if nativeWidgetKind(node) != .stack {
            fail("\(cls ?? "?") must map to a SwiftUI VStack")
        }
    }
    if cls == "deal-title" {
        if nativeWidgetKind(node) != .text {
            fail("a deal title must map to a SwiftUI Text")
        }
        if nativeFlattenedText(node).isEmpty {
            fail("a deal title has no visible text")
        }
    }
    if case let .element(tag, _, _, _, _) = node, tag == "h1" {
        if nativeWidgetKind(node) != .text { fail("the <h1> must map to a SwiftUI Text") }
    }
}

print("[4/4] CRM forest -> SwiftUI mapping asserted OK "
    + "(\(kanbanCols) columns -> VStack, \(dealCards) deal cards, "
    + "\(1 + cardButtons.count) Buttons, \(dealTitles.count) deal-title Texts; h1 'Pipeline')")

print("SMOKE OK: Rust core reached over the C ABI; IR parse + diff + cert-verify + CRM cross-platform render all exercised.")
