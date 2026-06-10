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
}
