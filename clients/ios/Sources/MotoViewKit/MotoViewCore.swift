//
//  MotoViewCore.swift — the typed Swift facade over the Rust C ABI.
//
//  Every method here calls a `mv_ffi_*` C function (declared in motoview_ffi.h,
//  implemented in client/src/ffi.rs), decodes the `{ok,value}`/`{ok,error}` JSON
//  envelope, and surfaces a Swift `Result`/throwing API. The Rust core does ALL
//  the real work — parsing, the keyed LIS diff, the chain-key cert verification.
//  Swift never reimplements any of it; this is the dumb-hands / smart-brain split
//  the web client already uses, applied to native.
//

import Foundation
import MotoViewFFI

public enum MotoViewError: Error, Equatable {
    /// The Rust core returned `{"ok":false,"error":"..."}`.
    case core(String)
    /// The envelope itself was not the shape we expected (should never happen).
    case malformed(String)
}

/// The MotoView native core. Stateless; every call is a pure function over the
/// Rust FFI. Hold one instance (or use the static helpers) per app.
public struct MotoViewCore {

    public init() {}

    // MARK: - Envelope plumbing

    /// Call a 1-string FFI function, free the returned C string, and return the
    /// decoded envelope's `value` (as Foundation JSON Any) or throw on `ok:false`.
    private func call1(_ input: String,
                       _ fn: (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?) throws -> Any {
        let raw: UnsafeMutablePointer<CChar>? = input.withCString { cstr in
            fn(cstr)
        }
        return try decodeEnvelope(raw)
    }

    /// Decode an FFI envelope C string into its `value`, freeing the buffer.
    private func decodeEnvelope(_ raw: UnsafeMutablePointer<CChar>?) throws -> Any {
        guard let raw = raw else { throw MotoViewError.malformed("null FFI result") }
        defer { mv_ffi_string_free(raw) }
        let json = String(cString: raw)
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw MotoViewError.malformed("non-JSON envelope: \(json)")
        }
        if let ok = obj["ok"] as? Bool, ok == true {
            guard let value = obj["value"] else {
                throw MotoViewError.malformed("envelope missing 'value'")
            }
            return value
        }
        let reason = (obj["error"] as? String) ?? "unknown"
        throw MotoViewError.core(reason)
    }

    // MARK: - IR parsing

    /// Parse a JSON IR forest (the Ir.mo.toJsonForest wire format) into a UINode
    /// tree, running the REAL Rust parser (depth guard, escape decoding, schema).
    public func parseForest(_ json: String) throws -> [UINode] {
        let value = try call1(json) { mv_ffi_parse_forest($0) }
        guard let arr = value as? [Any] else {
            throw MotoViewError.malformed("forest value not an array")
        }
        return try arr.map { try UINode.fromJSONObject($0) }
    }

    /// Parse a single JSON IR node into a UINode.
    public func parseNode(_ json: String) throws -> UINode {
        let value = try call1(json) { mv_ffi_parse_node($0) }
        return try UINode.fromJSONObject(value)
    }

    /// Render a JSON IR forest to HTML (used by the `.raw` fallback / debugging).
    public func renderForest(_ json: String) throws -> String {
        let value = try call1(json) { mv_ffi_render_forest($0) }
        guard let s = value as? String else {
            throw MotoViewError.malformed("render value not a string")
        }
        return s
    }

    // MARK: - Keyed diff

    /// Diff two JSON IR forests with the same keyed-reconcile/LIS the web brain
    /// uses, returning a `DiffPlan` (full re-render or an ordered op list).
    public func irDiff(old oldJSON: String, new newJSON: String) throws -> DiffPlan {
        let raw: UnsafeMutablePointer<CChar>? = oldJSON.withCString { o in
            newJSON.withCString { n in
                mv_ffi_ir_diff(o, n)
            }
        }
        let value = try decodeEnvelope(raw)
        return try DiffPlan.fromJSONObject(value)
    }

    // MARK: - Chain-key certificate verification

    /// Verify an IC response against the PINNED NNS root key (mainnet). Returns
    /// the certified `/time` (ns) as a `UInt64`-safe `String` (it is a u128 in
    /// Rust). Throws `.core("<CertError>")` on any failure (fails closed).
    ///
    /// `path` is the certified-data tree path, e.g.
    /// `[Data("canister".utf8), canisterId, Data("certified_data".utf8)]`.
    @discardableResult
    public func verifyResponse(cert: Data,
                               canister: Data,
                               path: [Data],
                               body: Data,
                               nowNs: UInt128Pair,
                               maxOffsetNs: UInt64 = 0) throws -> String {
        // Encode the path as length-prefixed segments: [u32_le len][bytes]...
        var pathBuf = Data()
        for seg in path {
            var len = UInt32(seg.count).littleEndian
            withUnsafeBytes(of: &len) { pathBuf.append(contentsOf: $0) }
            pathBuf.append(seg)
        }

        let raw: UnsafeMutablePointer<CChar>? = cert.withUnsafeBytes { certPtr in
            canister.withUnsafeBytes { canPtr in
                pathBuf.withUnsafeBytes { pathPtr in
                    body.withUnsafeBytes { bodyPtr in
                        mv_ffi_verify_response(
                            certPtr.bindMemory(to: UInt8.self).baseAddress, cert.count,
                            canPtr.bindMemory(to: UInt8.self).baseAddress, canister.count,
                            pathPtr.bindMemory(to: UInt8.self).baseAddress, pathBuf.count,
                            bodyPtr.bindMemory(to: UInt8.self).baseAddress, body.count,
                            nowNs.hi, nowNs.lo, maxOffsetNs
                        )
                    }
                }
            }
        }
        let value = try decodeEnvelope(raw)
        guard let obj = value as? [String: Any], let timeNs = obj["time_ns"] as? String else {
            throw MotoViewError.malformed("verify value missing 'time_ns'")
        }
        return timeNs
    }
}

/// A 128-bit nanosecond timestamp split into two u64 halves, matching the FFI
/// (C has no u128). Build from a Foundation `Date` via `.now()`.
public struct UInt128Pair {
    public let hi: UInt64
    public let lo: UInt64
    public init(hi: UInt64, lo: UInt64) {
        self.hi = hi
        self.lo = lo
    }
    /// Nanoseconds since the Unix epoch for `date`, as a hi/lo pair. The value
    /// fits in 64 bits until the year ~2554, so `hi` is 0 in practice — but we
    /// keep the split so the ABI is future-proof and matches the u128 Rust side.
    public static func nanos(since date: Date = Date()) -> UInt128Pair {
        let ns = UInt64(date.timeIntervalSince1970 * 1_000_000_000)
        return UInt128Pair(hi: 0, lo: ns)
    }
}
