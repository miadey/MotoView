// swift-tools-version:5.9
//
// MotoView native iOS client — Slice 8.
//
// One MotoView source -> native SwiftUI (NO WebView). This package contains:
//   * MotoViewFFI  — the hand-written C ABI module (motoview_ffi.h) over the Rust
//                    core static archive (client/, built --features ffi).
//   * MotoViewKit  — the Swift surface: a typed wrapper over the FFI, the UINode
//                    value type, the HostBridge protocol (native equivalent of
//                    the brain's ~12 host_* ABI), and `NativeView(_:)` mapping the
//                    portable UI-IR to SwiftUI views.
//   * MotoViewSmoke — a tiny executable that parses a sample IR JSON (the exact
//                    Ir.mo wire format) into a UINode tree through the REAL Rust
//                    parser and asserts the tree, then calls verify_response so
//                    the chain-key verifier is exercised end-to-end on this host.
//
// LINKING: MotoViewKit links the Rust static archive. `swift build`/`swift test`
// on THIS machine build for the host (aarch64-apple-darwin), so we link the host
// archive (libs/host/libmotoview_client.a). The device/simulator archives
// (libs/ios-arm64, libs/ios-arm64-sim) are the REAL iOS cross-compiles; wiring
// them into an actual .app / .xcframework needs full Xcode (see README — flagged,
// not available on this Command-Line-Tools-only machine).

import PackageDescription

let package = Package(
    name: "MotoView",
    platforms: [
        .macOS(.v13),
        .iOS(.v16)
    ],
    products: [
        .library(name: "MotoViewKit", targets: ["MotoViewKit"]),
        .executable(name: "motoview-smoke", targets: ["MotoViewSmoke"])
    ],
    targets: [
        // The C ABI module — exposes motoview_ffi.h to Swift.
        .target(
            name: "MotoViewFFI",
            path: "Sources/MotoViewFFI"
        ),
        // The Swift renderer + typed FFI wrapper. Links the Rust static archive.
        .target(
            name: "MotoViewKit",
            dependencies: ["MotoViewFFI"],
            path: "Sources/MotoViewKit",
            linkerSettings: [
                // The host archive for `swift build`/`swift test` on this machine.
                .unsafeFlags([
                    "-L", "libs/host",
                    "-lmotoview_client"
                ])
            ]
        ),
        // Host smoke executable — runs the REAL Rust core on this machine.
        .executableTarget(
            name: "MotoViewSmoke",
            dependencies: ["MotoViewKit"],
            path: "Sources/MotoViewSmoke"
        ),
        // Swift-test smoke: IR JSON -> UINode -> assert; cert verify fails closed.
        .testTarget(
            name: "MotoViewKitTests",
            dependencies: ["MotoViewKit"],
            path: "Tests/MotoViewKitTests"
        )
    ]
)
