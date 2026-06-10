# Play Integrity (Android) — config stub + wiring (Slice 11)

> **STATUS: documented stub, NOT activated here.** Play Integrity needs a Google
> Play Console app, the Play Integrity API enabled in a Google Cloud project, and
> a server (here a canister/backend) that decodes + verifies the integrity
> verdict. None of those exist on the build machine. This file is the exact
> plug-in point + the project setup the user must do.

## What Play Integrity gives you

A Google-signed **verdict** that the request comes from a **genuine Play install
of your app** on a **device that passes integrity checks** (not rooted/emulated,
recognized by Google). The Android analogue of iOS App Attest. The canister can
reject calls whose verdict is not `MEETS_DEVICE_INTEGRITY` / not from your
package, complementing the client's chain-key verification of the canister.

## Google-side prerequisites (USER must do)

1. Google Play Console account ($25 one-time) with the app published (at least
   to internal testing) under the `applicationId` in `app/build.gradle.kts`.
2. **Link a Google Cloud project** to the app and **enable the Play Integrity
   API** in that project (Play Console -> App integrity).
3. Configure response encryption: either let Google manage the response keys
   (recommended) or supply your own. The **decryption/verification** happens on
   your server (here a canister/backend), not on-device.
4. A server endpoint that:
   - issues a one-time **nonce**,
   - receives the integrity **token**, calls Google's
     `decodeIntegrityToken` (or verifies the Google-managed response), and
   - checks `appIntegrity.packageName`, `deviceIntegrity`, and the nonce.

## Where it plugs into the native client

The `com.google.android.play:integrity` dependency is already declared in
`app/build.gradle.kts`. The runtime flow:

```kotlin
import com.google.android.play.core.integrity.IntegrityManagerFactory
import com.google.android.play.core.integrity.IntegrityTokenRequest

val manager = IntegrityManagerFactory.create(context)
// nonce comes from the canister (one-time, bound to the request).
val request = IntegrityTokenRequest.builder().setNonce(nonce).build()
manager.requestIntegrityToken(request)
    .addOnSuccessListener { resp ->
        val token = resp.token()        // opaque, Google-signed
        // POST token to the canister; it verifies the verdict server-side.
    }
    .addOnFailureListener { /* deny / retry */ }
```

Suggested home: a new
`motoview/src/main/kotlin/dev/motoview/attest/PlayIntegrity.kt` exposing
`suspend fun integrityToken(nonce: String): String`, called from the
`HostBridge.fetch` path before a consequential update call. (Not added as code
yet — it is inert without the Play Console project + a verifying endpoint.)

## Where it plugs into CI

Nothing extra to BUILD: the integrity client is just a Gradle dependency
(already declared). The release pipeline ships the .aab as usual. The
**verifying side** (canister + the linked Cloud project) is a separate setup,
not part of the Android CI.

## Honest limitations

- Play Integrity returns a meaningful verdict only for an app **installed from
  Play** (or a Play-recognized internal-test track) and requires the Cloud
  project link, so it is **not exercised** here.
- The verifying side (canister) is **not implemented** here — only the
  client-call shape and the project setup are specified.
