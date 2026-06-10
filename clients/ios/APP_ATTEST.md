# App Attest (iOS) — config stub + wiring (Slice 11)

> **STATUS: documented stub, NOT activated here.** App Attest needs an Apple
> Developer account, an App ID with the App Attest capability, and a server (here
> a canister/backend) that verifies the attestation. None of those exist on the
> build machine. This file is the exact plug-in point + the project setup the
> user must do.

## What App Attest gives you

A hardware-backed proof that a request comes from a **genuine, unmodified build
of your app** on a **real Apple device** (Secure Enclave key). For a
banking-grade MotoView client it lets the canister reject requests that did not
originate from the real app — complementing the chain-key verification that the
client does in the *other* direction (client verifies the canister).

## Apple-side prerequisites (USER must do)

1. Apple Developer Program membership ($99/yr).
2. The App ID must have the **App Attest** capability (Certificates, Identifiers
   & Profiles -> your App ID -> Capabilities).
3. Add the `com.apple.developer.devicecheck.appattest-environment` entitlement to
   the app target (`production` for release, `development` for debug).
4. A server endpoint that:
   - issues a one-time **challenge** (nonce),
   - verifies the **attestation object** (validates the cert chain to Apple's
     App Attest root, the nonce, the app id hash, the counter), and
   - on later calls verifies per-request **assertions**.
   In MotoView this lives in the canister (Motoko) or a backend it calls.

## Where it plugs into the native client

`DCAppAttestService` (DeviceCheck framework) on iOS 14+. The flow:

```swift
import DeviceCheck

// 1. One-time per install: generate a hardware key.
let service = DCAppAttestService.shared
guard service.isSupported else { /* fall back / deny */ return }
service.generateKey { keyId, err in
    // 2. Ask the canister for a challenge (nonce), hash (clientDataHash) it.
    // 3. Attest the key over that hash:
    service.attestKey(keyId!, clientDataHash: hash) { attestation, err in
        // 4. POST keyId + attestation to the canister; it verifies + stores it.
    }
}

// Per consequential request afterwards:
service.generateAssertion(keyId, clientDataHash: requestHash) { assertion, err in
    // attach `assertion` to the IC update call; canister verifies the counter.
}
```

Suggested home in the Swift package: a new
`Sources/MotoViewKit/Attestation/AppAttest.swift` exposing
`func attestedHeaders(for body: Data) async throws -> [String: String]`, called
by `HostBridge.fetch` before a consequential update call. (Not added as code yet
— it is inert without the Apple entitlement + a verifying canister endpoint.)

## Where it plugs into CI

Nothing extra to BUILD: App Attest is a runtime capability + entitlement. The
release pipeline only needs the entitlement present in the signed app
(provisioned via the App ID capability above and the `match` appstore profile).
The **server/canister verifier** is a separate deploy, not part of this iOS CI.

## Honest limitations

- App Attest cannot run in the iOS **Simulator** and not at all without the
  entitlement + Apple account, so it is **not exercised** on this machine.
- The verifying side (canister) is **not implemented** here — only the
  client-call shape and the project setup are specified.
