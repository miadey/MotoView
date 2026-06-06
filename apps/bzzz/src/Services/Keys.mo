/// Keys service — E2EE public key DIRECTORY for the Bzzz Messenger.
///
/// IMPORTANT HONESTY NOTE: this canister is a key DIRECTORY + prekey
/// distributor ONLY. No cryptography happens here. The real X25519 Diffie-
/// Hellman, the X3DH handshake, and AEAD (e.g. XChaCha20-Poly1305) encryption
/// all run in the user's BROWSER. This service simply stores the public bytes
/// each device chooses to publish (identity key, signed prekey + its signature,
/// and a pool of one-time prekeys) so that a peer can fetch a bundle and start
/// an encrypted session. We store all keys as opaque base64 `Text` and never
/// inspect, validate, or use them — the client is responsible for verifying the
/// signed-prekey signature against the published identity key.
///
/// Stateful service: the MotoView compiler instantiates one shared `Keys` at
/// actor scope, so every page sees the same directory for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — hence the framework
/// convention of a service file exporting `public class <Name>()`.)
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Array "mo:base/Array";
import Buffer "mo:base/Buffer";
import Int "mo:base/Int";

module {
  /// A published device key set for one principal. All key material is opaque
  /// base64 `Text`; the canister never interprets it.
  
  /// What a peer fetches to begin an X3DH session. Carries the public material
  /// plus AT MOST ONE one-time prekey, popped from the pool (null when the pool
  /// is exhausted — the client falls back to the signed prekey alone).
  
  public class Keys() {
    public type Device = {
    deviceId : Text; // client-chosen stable id (e.g. a random/installation id)
    ikPub : Text; // base64 X25519 long-term identity public key
    spkPub : Text; // base64 signed-prekey public key
    spkSig : Text; // base64 signature over spkPub by the identity key (client-verified)
    spkAt : Int; // Time.now() when the signed prekey was published
    otpks : [Text]; // pool of base64 one-time prekey public keys (consumed one-per-session)
  };
    public type KeyBundle = {
    deviceId : Text;
    ikPub : Text;
    spkPub : Text;
    spkSig : Text;
    spkAt : Int;
    otpk : ?Text; // a single one-time prekey, or null if the pool is empty
  };

    // principal -> their published devices (in publish order)
    let devices = HashMap.HashMap<Principal, [Device]>(64, Principal.equal, Principal.hash);

    /// Publish (or fully replace) a device's key set for the caller. A device is
    /// identified by `deviceId`; re-publishing the same `deviceId` overwrites the
    /// previous entry (e.g. rotating the signed prekey). Returns false on missing
    /// required key material.
    public func publishDevice(
      caller : Principal,
      deviceId : Text,
      ikPub : Text,
      spkPub : Text,
      spkSig : Text,
      otpks : [Text],
    ) : Bool {
      if (deviceId == "" or ikPub == "" or spkPub == "" or spkSig == "") {
        return false;
      };
      let dev : Device = {
        deviceId;
        ikPub;
        spkPub;
        spkSig;
        spkAt = Time.now();
        otpks;
      };
      let current = switch (devices.get(caller)) { case (?d) d; case null [] };
      let buf = Buffer.Buffer<Device>(current.size() + 1);
      var replaced = false;
      for (d in current.vals()) {
        if (d.deviceId == deviceId) {
          buf.add(dev);
          replaced := true;
        } else {
          buf.add(d);
        };
      };
      if (not replaced) { buf.add(dev) };
      devices.put(caller, Buffer.toArray(buf));
      true;
    };

    /// Top up a device's one-time prekey pool with `more` fresh base64 prekeys.
    /// Returns false if the caller has no such device.
    public func replenishOtpks(caller : Principal, deviceId : Text, more : [Text]) : Bool {
      switch (devices.get(caller)) {
        case null { false };
        case (?current) {
          var found = false;
          let updated = Array.map<Device, Device>(
            current,
            func(d : Device) : Device {
              if (d.deviceId == deviceId) {
                found := true;
                { d with otpks = Array.append(d.otpks, more) };
              } else { d };
            },
          );
          if (not found) { return false };
          devices.put(caller, updated);
          true;
        };
      };
    };

    /// All devices published by a principal (empty if none).
    public func devicesOf(p : Principal) : [Device] {
      switch (devices.get(p)) { case (?d) d; case null [] };
    };

    /// Whether a principal has published at least one device.
    public func hasKeys(p : Principal) : Bool {
      switch (devices.get(p)) {
        case (?d) d.size() > 0;
        case null false;
      };
    };

    /// Total number of one-time prekeys currently available across a principal's
    /// devices — useful for a "low prekeys, please replenish" UI hint.
    public func otpkCount(p : Principal) : Nat {
      var n = 0;
      for (d in devicesOf(p).vals()) { n += d.otpks.size() };
      n;
    };

    /// Number of devices a principal has registered.
    public func deviceCount(p : Principal) : Nat { devicesOf(p).size() };

    /// Fetch a session bundle for `peer`'s FIRST device and POP one one-time
    /// prekey from that device's pool (so each requester gets a distinct OTPK).
    /// Returns null if the peer has published no devices. The returned `otpk` is
    /// null when the pool is exhausted, in which case the client proceeds with
    /// the signed prekey only.
    public func fetchBundle(peer : Principal) : ?KeyBundle {
      switch (devices.get(peer)) {
        case null { null };
        case (?devs) {
          if (devs.size() == 0) { return null };
          let first = devs[0];
          // pop the head of the one-time prekey pool, if any
          var popped : ?Text = null;
          let remaining : [Text] =
            if (first.otpks.size() > 0) {
              popped := ?first.otpks[0];
              Array.subArray<Text>(first.otpks, 1, first.otpks.size() - 1);
            } else { [] };
          // write the device back with the shrunken pool
          let updatedFirst : Device = { first with otpks = remaining };
          let rest = Array.subArray<Device>(devs, 1, devs.size() - 1);
          devices.put(peer, Array.append([updatedFirst], rest));
          ?{
            deviceId = first.deviceId;
            ikPub = first.ikPub;
            spkPub = first.spkPub;
            spkSig = first.spkSig;
            spkAt = first.spkAt;
            otpk = popped;
          };
        };
      };
    };

    /// Number of principals who have published any keys.
    public func directorySize() : Nat { devices.size() };

    // ---- Text-returning helpers for thin .mview pages ----

    /// Short, human-readable freshness for a signed prekey timestamp.
    public func relativeTime(at : Int) : Text {
      let now = Time.now();
      let diff = now - at;
      if (diff < 0) { return "just now" };
      let secs = diff / 1_000_000_000;
      if (secs < 60) { return "just now" };
      let mins = secs / 60;
      if (mins < 60) { return Int.toText(mins) # "m ago" };
      let hours = mins / 60;
      if (hours < 24) { return Int.toText(hours) # "h ago" };
      let days = hours / 24;
      if (days < 30) { return Int.toText(days) # "d ago" };
      let months = days / 30;
      if (months < 12) { return Int.toText(months) # "mo ago" };
      Int.toText(days / 365) # "y ago";
    };

    /// A short, render-safe fingerprint of a base64 public key for display
    /// (first 10 chars + ellipsis). Purely cosmetic; the client compares full
    /// keys / out-of-band safety numbers for real verification.
    public func keyFingerprint(b64 : Text) : Text {
      let chars = Text.toArray(b64);
      if (chars.size() <= 10) { b64 } else {
        Text.fromIter(Array.subArray<Char>(chars, 0, 10).vals()) # "…";
      };
    };

    /// A one-line, page-friendly status summary for a principal's key state.
    /// `handle` is supplied by the PAGE from the Identity service (services do
    /// not call each other).
    public func statusLine(p : Principal, handle : Text) : Text {
      if (not hasKeys(p)) {
        return handle # " has not published encryption keys yet";
      };
      let nd = deviceCount(p);
      let nk = otpkCount(p);
      let devWord = if (nd == 1) "device" else "devices";
      handle # ": " # Int.toText(nd) # " " # devWord # ", " # Int.toText(nk) # " one-time prekeys available";
    };

    // ---- Upgrade-stable persistence (MotoView framework hooks) ----

    public func mvStableSave() : Blob {
      to_candid ({
        devices = Iter.toArray(devices.entries());
      });
    };

    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?{ devices : [(Principal, [Device])] }) {
        case (?saved) {
          let savedDevices = saved.devices;
          for (k in Iter.toArray(devices.keys()).vals()) { devices.delete(k) };
          for ((k, v) in savedDevices.vals()) { devices.put(k, v) };
        };
        case null {};
      };
    };
  };
};
