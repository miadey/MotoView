//
//  MotoViewKitTests.swift — XCTest smoke for the native core.
//
//  ⚠️  REQUIRES FULL XCODE TO RUN. This machine has only the Command-Line-Tools,
//      which do NOT ship a usable macOS `XCTest` (nor a SwiftPM-wired `Testing`)
//      module, so `swift test` cannot resolve `import XCTest` here. On a machine
//      with full Xcode, `swift test` runs these. On THIS machine the EXACT same
//      assertions run as a plain executable — `swift run motoview-smoke` (see
//      Sources/MotoViewSmoke), which DID run and pass. `swift build` compiles the
//      library/executable fine (it does not build test targets).
//
//  Exercises the REAL Rust core through the C ABI:
//   * parseForest / parseNode over the EXACT Ir.mo wire format,
//   * irDiff keyed reconcile,
//   * verify_response fails closed on garbage,
//   * a DiffPlan drives a StateHostBridge (the brain/hands split).
//

import XCTest
@testable import MotoViewKit

final class MotoViewKitTests: XCTestCase {

    let core = MotoViewCore()

    func testParseForestGolden() throws {
        let json = """
        [{"t":"el","tag":"section","attrs":{},"events":{},"children":[\
        {"t":"el","tag":"h1","attrs":{},"events":{},"children":[\
        {"t":"raw","html":"Hi "},\
        {"t":"text","value":"Ada & Co"}]},\
        {"t":"el","tag":"li","attrs":{},"events":{},"key":"it-1","children":[\
        {"t":"el","tag":"button","attrs":{"data-mv-arg0":"it-1"},"events":{"click":"pick"},"children":[\
        {"t":"text","value":"it-1"}]}]}]}]
        """
        let forest = try core.parseForest(json)
        XCTAssertEqual(forest.count, 1)
        guard case let .element(tag, _, _, key, children) = forest[0] else {
            return XCTFail("top not element")
        }
        XCTAssertEqual(tag, "section")
        XCTAssertNil(key)
        XCTAssertEqual(children.count, 2)
        XCTAssertEqual(children[1].key, "it-1")
    }

    func testParseNodeTextEscapes() throws {
        let node = try core.parseNode(#"{"t":"text","value":"a \"q\" & <b>\n\tend"}"#)
        XCTAssertEqual(node, .text("a \"q\" & <b>\n\tend"))
    }

    func testParseRejectsMalformed() {
        XCTAssertThrowsError(try core.parseForest("["))
        XCTAssertThrowsError(try core.parseNode(#"{"t":"el"}"#))
    }

    func testIrDiffReplace() throws {
        let old = #"[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"A"}]}]"#
        let new = #"[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"B"}]}]"#
        let plan = try core.irDiff(old: old, new: new)
        guard case let .patch(ops) = plan else { return XCTFail("not patch: \(plan)") }
        XCTAssertEqual(ops.count, 1)
        guard case let .replace(key, _) = ops[0] else { return XCTFail("not replace") }
        XCTAssertEqual(key, "a")
    }

    func testIrDiffAppendInsert() throws {
        let old = #"[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"A"}]}]"#
        let new = #"[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"A"}]},{"t":"el","tag":"li","attrs":{},"events":{},"key":"b","children":[{"t":"text","value":"B"}]}]"#
        let plan = try core.irDiff(old: old, new: new)
        guard case let .patch(ops) = plan, ops.count == 1, case let .insert(_, after) = ops[0] else {
            return XCTFail("not single insert: \(plan)")
        }
        XCTAssertEqual(after, "a")
    }

    func testIrDiffNoKeysIsFull() throws {
        let old = #"[{"t":"el","tag":"p","attrs":{},"events":{},"children":[{"t":"text","value":"a"}]}]"#
        let new = #"[{"t":"el","tag":"p","attrs":{},"events":{},"children":[{"t":"text","value":"b"}]}]"#
        XCTAssertEqual(try core.irDiff(old: old, new: new), .full)
    }

    func testRenderForestHTML() throws {
        let json = #"[{"t":"el","tag":"p","attrs":{},"events":{},"children":[{"t":"text","value":"x & y"}]}]"#
        let html = try core.renderForest(json)
        XCTAssertEqual(html, "<p>x &amp; y</p>")
    }

    func testVerifyResponseFailsClosedOnGarbage() {
        let cert = Data([0, 1, 2, 3, 4, 5, 6, 7])
        let canister = Data(repeating: 0, count: 10)
        let body = Data([1, 2, 3])
        let path = [Data("canister".utf8), canister, Data("certified_data".utf8)]
        XCTAssertThrowsError(
            try core.verifyResponse(cert: cert, canister: canister, path: path,
                                    body: body, nowNs: .nanos())
        ) { err in
            guard case MotoViewError.core = err else {
                return XCTFail("expected .core(named error), got \(err)")
            }
        }
    }

    func testDiffPlanDrivesHostBridge() throws {
        let old = #"[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"A"}]},{"t":"el","tag":"li","attrs":{},"events":{},"key":"b","children":[{"t":"text","value":"B"}]}]"#
        let new = #"[{"t":"el","tag":"li","attrs":{},"events":{},"key":"a","children":[{"t":"text","value":"A2"}]},{"t":"el","tag":"li","attrs":{},"events":{},"key":"b","children":[{"t":"text","value":"B"}]}]"#
        let plan = try core.irDiff(old: old, new: new)
        let bridge = StateHostBridge()
        bridge.apply(plan: plan, to: "list", core: core)
        XCTAssertTrue(bridge.records.contains(where: { $0.kind == "replaceKeyed" && $0.detail == "list[a]" }),
                      "expected a replaceKeyed for key a, got \(bridge.records)")
    }

    func testButtonEventContextArgs() throws {
        let json = #"[{"t":"el","tag":"button","attrs":{"data-mv-arg0":"7"},"events":{"click":"pick"},"children":[{"t":"text","value":"Pick"}]}]"#
        let forest = try core.parseForest(json)
        guard case let .element(_, attrs, events, _, _) = forest[0] else {
            return XCTFail("not element")
        }
        XCTAssertEqual(events.first(where: { $0.name == "click" })?.value, "pick")
        XCTAssertEqual(attrs.first(where: { $0.name == "data-mv-arg0" })?.value, "7")
    }

    // MARK: - CROSS-PLATFORM PROOF: the CRM forest -> SwiftUI widgets
    //
    // Loads the COMMITTED CRM UI-IR fixture (the exact `motoview preview
    // examples/crm` output — the same forest the deployed web canister was
    // Playwright-driven against), parses it through the REAL Rust core, and
    // asserts NativeView's SwiftUI mapping via the pure `nativeWidgetKind`
    // shadow. This XCTest is the full-Xcode mirror of step [4/4] in the
    // `motoview-smoke` executable (which runs the SAME assertions on this
    // Command-Line-Tools-only machine).

    private func crmForest() throws -> [UINode] {
        // This test file lives at clients/ios/Tests/MotoViewKitTests/…; the
        // fixture is at clients/ios/Tests/Fixtures/crm.forest.json.
        let here = URL(fileURLWithPath: #filePath)
        let url = here
            .deletingLastPathComponent()   // MotoViewKitTests
            .deletingLastPathComponent()   // Tests
            .appendingPathComponent("Fixtures/crm.forest.json")
        let json = try String(contentsOf: url, encoding: .utf8)
        return try core.parseForest(json)
    }

    private func walk(_ nodes: [UINode], _ visit: (UINode) -> Void) {
        for n in nodes {
            visit(n)
            if case let .element(_, _, _, _, children) = n { walk(children, visit) }
        }
    }

    func testCRMForestIsTheCRMBoard() throws {
        let forest = try crmForest()
        XCTAssertFalse(forest.isEmpty)

        var h1: String?
        var cols = 0, cards = 0
        var titles: [String] = []
        var newDeal = 0
        walk(forest) { node in
            guard case let .element(tag, attrs, events, _, _) = node else { return }
            if tag == "h1" { h1 = nativeFlattenedText(node) }
            let cls = attrs.first(where: { $0.name == "class" })?.value
            if cls == "kanban-col" { cols += 1; XCTAssertEqual(tag, "section") }
            if cls == "deal-card" { cards += 1; XCTAssertEqual(tag, "article") }
            if cls == "deal-title" { titles.append(nativeFlattenedText(node)) }
            if tag == "button",
               events.first(where: { $0.name == "click" })?.value == "toggleCreate" {
                newDeal += 1
                XCTAssertEqual(nativeFlattenedText(node), "+ New deal")
            }
        }
        XCTAssertEqual(h1, "Pipeline")
        XCTAssertEqual(cols, 4)
        XCTAssertEqual(cards, 6)
        XCTAssertEqual(titles.count, 6)
        XCTAssertTrue(titles.contains("Website redesign"))
        XCTAssertEqual(newDeal, 1)
    }

    func testCRMForestMapsToSwiftUIWidgets() throws {
        let forest = try crmForest()
        var buttons = 0
        walk(forest) { node in
            guard case let .element(tag, attrs, events, _, _) = node else { return }
            let cls = attrs.first(where: { $0.name == "class" })?.value
            if tag == "button" {
                XCTAssertEqual(nativeWidgetKind(node), .button,
                               "every CRM <button> maps to a SwiftUI Button")
                let h = events.first(where: { $0.name == "click" })?.value ?? ""
                if ["toggleCreate", "removeDeal", "moveBack", "moveFwd"].contains(h) {
                    buttons += 1
                }
            }
            if cls == "kanban-col" || cls == "kanban-col-body" || cls == "deal-card" {
                XCTAssertEqual(nativeWidgetKind(node), .stack,
                               "\(cls!) maps to a SwiftUI VStack")
            }
            if cls == "deal-title" {
                XCTAssertEqual(nativeWidgetKind(node), .text,
                               "a deal title maps to a SwiftUI Text")
                XCTAssertFalse(nativeFlattenedText(node).isEmpty)
            }
            if tag == "h1" {
                XCTAssertEqual(nativeWidgetKind(node), .text)
            }
        }
        // 1 "+ New deal" + 6x(removeDeal+moveBack+moveFwd) = 19 mapped buttons.
        XCTAssertEqual(buttons, 19)
    }
}
