/*
 * mv-auth.js — hand-written, dependency-free Internet Identity login for MotoView.
 *
 * No npm, no agent-js. Implements just enough of the IC call protocol to make a
 * single authenticated `mvEstablish(nonce)` update call, so the canister sees the
 * real signed-in principal; the canister then mints an HMAC session cookie that
 * every later request carries (see runtime/src/App.mo). Two entry points:
 *   mvAuth.iiLogin()   — Internet Identity (passkey) via the II postMessage flow
 *   mvAuth.devLogin()  — a locally generated Ed25519 identity (no passkey; for
 *                        local dev where the II frontend isn't reachable)
 *   mvAuth.logout()
 *
 * Verified end-to-end: the signing path is byte-identical to the Node prototype
 * the IC replica accepted (202) and whose principal the canister recorded.
 */
(function () {
  "use strict";
  var te = new TextEncoder();
  function cat() { var as = arguments, n = 0, i; for (i = 0; i < as.length; i++) n += as[i].length; var o = new Uint8Array(n), k = 0; for (i = 0; i < as.length; i++) { o.set(as[i], k); k += as[i].length; } return o; }

  // ---- pure-JS SHA-256 / SHA-224 (browser WebCrypto has no SHA-224) ----
  var K = (function () {
    var k = [], p = 2, i, n, r, f;
    function isP(x) { for (var d = 2; d * d <= x; d++) if (x % d === 0) return false; return true; }
    n = 0;
    for (p = 2; n < 64; p++) { if (isP(p)) { r = Math.pow(p, 1 / 3); f = Math.floor((r - Math.floor(r)) * Math.pow(2, 32)); k.push(f >>> 0); n++; } }
    return k;
  })();
  function shaCore(bytes, H) {
    var l = bytes.length, bl = l * 8;
    var withOne = l + 1, padLen = (withOne % 64 <= 56) ? (56 - (withOne % 64)) : (120 - (withOne % 64));
    var total = l + 1 + padLen + 8, msg = new Uint8Array(total);
    msg.set(bytes); msg[l] = 0x80;
    var hi = Math.floor(bl / 0x100000000), lo = bl >>> 0;
    msg[total - 8] = (hi >>> 24) & 255; msg[total - 7] = (hi >>> 16) & 255; msg[total - 6] = (hi >>> 8) & 255; msg[total - 5] = hi & 255;
    msg[total - 4] = (lo >>> 24) & 255; msg[total - 3] = (lo >>> 16) & 255; msg[total - 2] = (lo >>> 8) & 255; msg[total - 1] = lo & 255;
    var h = H.slice(), w = new Int32Array(64), i, t;
    function rotr(x, n) { return (x >>> n) | (x << (32 - n)); }
    for (i = 0; i < total; i += 64) {
      for (t = 0; t < 16; t++) w[t] = (msg[i + 4 * t] << 24) | (msg[i + 4 * t + 1] << 16) | (msg[i + 4 * t + 2] << 8) | (msg[i + 4 * t + 3]);
      for (t = 16; t < 64; t++) { var s0 = rotr(w[t - 15], 7) ^ rotr(w[t - 15], 18) ^ (w[t - 15] >>> 3); var s1 = rotr(w[t - 2], 17) ^ rotr(w[t - 2], 19) ^ (w[t - 2] >>> 10); w[t] = (w[t - 16] + s0 + w[t - 7] + s1) | 0; }
      var a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
      for (t = 0; t < 64; t++) {
        var S1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25), ch = (e & f) ^ (~e & g), t1 = (hh + S1 + ch + K[t] + w[t]) | 0;
        var S0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22), maj = (a & b) ^ (a & c) ^ (b & c), t2 = (S0 + maj) | 0;
        hh = g; g = f; f = e; e = (d + t1) | 0; d = c; c = b; b = a; a = (t1 + t2) | 0;
      }
      h[0] = (h[0] + a) | 0; h[1] = (h[1] + b) | 0; h[2] = (h[2] + c) | 0; h[3] = (h[3] + d) | 0; h[4] = (h[4] + e) | 0; h[5] = (h[5] + f) | 0; h[6] = (h[6] + g) | 0; h[7] = (h[7] + hh) | 0;
    }
    var out = new Uint8Array(32);
    for (i = 0; i < 8; i++) { out[4 * i] = (h[i] >>> 24) & 255; out[4 * i + 1] = (h[i] >>> 16) & 255; out[4 * i + 2] = (h[i] >>> 8) & 255; out[4 * i + 3] = h[i] & 255; }
    return out;
  }
  var H256 = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  var H224 = [0xc1059ed8, 0x367cd507, 0x3070dd17, 0xf70e5939, 0xffc00b31, 0x68581511, 0x64f98fa7, 0xbefa4fa4];
  function sha256(b) { return shaCore(b, H256); }
  function sha224(b) { return shaCore(b, H224).slice(0, 28); }

  // ---- base32 + crc32 (principal text <-> bytes) ----
  var B32 = "abcdefghijklmnopqrstuvwxyz234567";
  function b32decode(s) { var bits = 0, val = 0, out = []; for (var i = 0; i < s.length; i++) { val = (val << 5) | B32.indexOf(s[i]); bits += 5; if (bits >= 8) { out.push((val >> (bits - 8)) & 255); bits -= 8; } } return new Uint8Array(out); }
  function principalToBytes(text) { return b32decode(text.replace(/-/g, "")).slice(4); }
  function crc32(b) { var c = ~0, i, j; for (i = 0; i < b.length; i++) { c ^= b[i]; for (j = 0; j < 8; j++) c = (c >>> 1) ^ (0xEDB88320 & -(c & 1)); } return (~c) >>> 0; }
  function bytesToPrincipal(b) { var crc = crc32(b), cb = new Uint8Array([(crc >>> 24) & 255, (crc >>> 16) & 255, (crc >>> 8) & 255, crc & 255]); var full = cat(cb, b), bits = 0, val = 0, out = "", i; for (i = 0; i < full.length; i++) { val = (val << 8) | full[i]; bits += 8; while (bits >= 5) { out += B32[(val >> (bits - 5)) & 31]; bits -= 5; } } if (bits > 0) out += B32[(val << (5 - bits)) & 31]; return out.replace(/(.{5})/g, "$1-").replace(/-$/, ""); }

  // ---- leb128 / CBOR / candid / DER ----
  function leb128(n) { n = BigInt(n); var out = []; do { var b = Number(n & 0x7fn); n >>= 7n; if (n !== 0n) b |= 0x80; out.push(b); } while (n !== 0n); return new Uint8Array(out); }
  function head(m, l) { m <<= 5; if (l < 24) return new Uint8Array([m | l]); if (l < 256) return new Uint8Array([m | 24, l]); if (l < 65536) return new Uint8Array([m | 25, l >> 8, l & 255]); return new Uint8Array([m | 26, (l >>> 24) & 255, (l >> 16) & 255, (l >> 8) & 255, l & 255]); }
  function cbBytes(b) { return cat(head(2, b.length), b); }
  function cbText(s) { var b = te.encode(s); return cat(head(3, b.length), b); }
  function cbUint(n) { n = BigInt(n); if (n < 24n) return new Uint8Array([Number(n)]); if (n < 256n) return new Uint8Array([0x18, Number(n)]); if (n < 65536n) return new Uint8Array([0x19, Number(n >> 8n), Number(n & 255n)]); if (n < 4294967296n) { var o = new Uint8Array([0x1a, 0, 0, 0, 0]); for (var i = 0; i < 4; i++) o[4 - i] = Number((n >> BigInt(8 * i)) & 255n); return o; } var p = new Uint8Array([0x1b, 0, 0, 0, 0, 0, 0, 0, 0]); for (var j = 0; j < 8; j++) p[8 - j] = Number((n >> BigInt(8 * j)) & 255n); return p; }
  function cbArr(items) { var o = head(4, items.length); for (var i = 0; i < items.length; i++) o = cat(o, items[i]); return o; }
  function cbMap(entries) { var o = head(5, entries.length), i; for (i = 0; i < entries.length; i++) o = cat(o, cbText(entries[i][0]), entries[i][1]); return o; }
  function candidText(s) { var b = te.encode(s); return cat(te.encode("DIDL"), new Uint8Array([0x00, 0x01, 0x71]), leb128(b.length), b); }
  var ED_DER = new Uint8Array([0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00]);
  function derEd(raw) { return cat(ED_DER, raw); }

  function hashVal(kind, v) { if (kind === "bytes") return sha256(v); if (kind === "text") return sha256(te.encode(v)); return sha256(leb128(v)); }
  function requestId(fields) {
    var rows = fields.map(function (f) { return cat(sha256(te.encode(f[0])), hashVal(f[1], f[2])); });
    rows.sort(function (a, b) { for (var i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] - b[i]; return a.length - b.length; });
    return sha256(cat.apply(null, rows));
  }

  // canister id of the app = the subdomain of the current host
  function appCanisterId() { var h = location.hostname; var m = h.match(/^([a-z0-9-]+)\.(raw\.)?localhost$/) || h.match(/^([a-z0-9-]+)\.(raw\.)?ic0\.app$/) || h.match(/^([a-z0-9-]+)\./); return m ? m[1] : ""; }
  function apiOrigin() { return location.protocol + "//" + location.host.replace(/^[a-z0-9-]+\.(raw\.)?/, ""); }

  // ---- the one authenticated call: mvEstablish(nonce) ----
  // identity = { sign(msg)->Promise<Uint8Array>, pubkeyDer:Uint8Array, delegations?:[...] }
  async function establish(identity, nonce) {
    var canisterId = appCanisterId();
    var canisterBytes = principalToBytes(canisterId);
    var sender = cat(sha224(identity.pubkeyDer), new Uint8Array([0x02]));
    var arg = candidText(nonce);
    var expiry = (BigInt(Date.now()) * 1000000n) + (4n * 60n * 1000000000n);
    var reqNonce = crypto.getRandomValues(new Uint8Array(16));
    var fields = [
      ["request_type", "text", "call"], ["canister_id", "bytes", canisterBytes],
      ["method_name", "text", "mvEstablish"], ["arg", "bytes", arg],
      ["sender", "bytes", sender], ["ingress_expiry", "nat", expiry], ["nonce", "bytes", reqNonce]
    ];
    var reqId = requestId(fields);
    var sig = await identity.sign(cat(new Uint8Array([0x0a]), te.encode("ic-request"), reqId));
    var content = cbMap([
      ["request_type", cbText("call")], ["canister_id", cbBytes(canisterBytes)],
      ["method_name", cbText("mvEstablish")], ["arg", cbBytes(arg)],
      ["sender", cbBytes(sender)], ["ingress_expiry", cbUint(expiry)], ["nonce", cbBytes(reqNonce)]
    ]);
    var env = [["content", content], ["sender_pubkey", cbBytes(identity.pubkeyDer)], ["sender_sig", cbBytes(sig)]];
    if (identity.delegations) {
      env.push(["sender_delegation", cbArr(identity.delegations.map(function (d) {
        return cbMap([
          ["delegation", cbMap([["pubkey", cbBytes(d.delegation.pubkey)], ["expiration", cbUint(d.delegation.expiration)]])],
          ["signature", cbBytes(d.signature)]
        ]);
      }))]);
    }
    var body = cat(new Uint8Array([0xd9, 0xd9, 0xf7]), cbMap(env));
    var res = await fetch(apiOrigin() + "/api/v2/canister/" + canisterId + "/call", { method: "POST", headers: { "content-type": "application/cbor" }, body: body });
    if (res.status >= 400) throw new Error("call rejected: " + res.status + " " + (await res.text()).slice(0, 200));
    // exchange the nonce for a session token (poll the gateway)
    var tok = "";
    for (var i = 0; i < 15 && !tok; i++) { await new Promise(function (r) { setTimeout(r, 500); }); tok = (await (await fetch("/mv-session?nonce=" + encodeURIComponent(nonce), { cache: "no-store" })).text()).trim(); }
    if (!tok) throw new Error("the call was accepted but the session was not established");
    return tok;
  }

  function setSession(token) { document.cookie = "mv_session=" + token + "; path=/; max-age=86400; samesite=Lax"; }

  // ---- a locally generated Ed25519 identity (dev login) ----
  async function ephemeralIdentity() {
    var kp = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    var raw = new Uint8Array(await crypto.subtle.exportKey("raw", kp.publicKey));
    return {
      pubkeyDer: derEd(raw),
      sign: async function (m) { return new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, kp.privateKey, m)); }
    };
  }

  // ---- Internet Identity delegation flow (postMessage) ----
  // Generates a SESSION key, sends its DER pubkey to II, receives a delegation
  // chain for the user's identity; signs the call with the session key.
  async function iiIdentity(iiUrl) {
    var sk = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    var sessRaw = new Uint8Array(await crypto.subtle.exportKey("raw", sk.publicKey));
    var sessDer = derEd(sessRaw);
    var win = window.open(iiUrl + "#authorize", "ii-window", "width=420,height=640");
    if (!win) throw new Error("popup blocked — allow popups and retry");
    var result = await new Promise(function (resolve, reject) {
      var timer = setTimeout(function () { reject(new Error("II login timed out")); }, 5 * 60 * 1000);
      function onMsg(ev) {
        if (ev.source !== win) return;
        var d = ev.data || {};
        if (d.kind === "authorize-ready") {
          win.postMessage({ kind: "authorize-client", sessionPublicKey: sessDer, maxTimeToLive: BigInt(8) * 60n * 60n * 1000000000n }, "*");
        } else if (d.kind === "authorize-client-success") {
          window.removeEventListener("message", onMsg); clearTimeout(timer); resolve(d);
        } else if (d.kind === "authorize-client-failure") {
          window.removeEventListener("message", onMsg); clearTimeout(timer); reject(new Error(d.text || "II login failed"));
        }
      }
      window.addEventListener("message", onMsg);
    });
    var userKey = new Uint8Array(result.userPublicKey);
    var delegations = result.delegations.map(function (d) {
      return { delegation: { pubkey: new Uint8Array(d.delegation.pubkey), expiration: BigInt(d.delegation.expiration) }, signature: new Uint8Array(d.signature) };
    });
    return {
      pubkeyDer: userKey,                 // sender = self-auth(userPublicKey)
      delegations: delegations,           // userKey -> sessionKey, signed by II
      sign: async function (m) { return new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, sk.privateKey, m)); }
    };
  }

  function newNonce() { var b = crypto.getRandomValues(new Uint8Array(12)); return Array.from(b).map(function (x) { return x.toString(16).padStart(2, "0"); }).join(""); }

  window.mvAuth = {
    principalOfToken: function (t) { return (t || "").split(".")[0] || ""; },
    devLogin: async function () { var id = await ephemeralIdentity(); var tok = await establish(id, newNonce()); setSession(tok); return mvAuth.principalOfToken(tok); },
    iiLogin: async function (iiUrl) { var id = await iiIdentity(iiUrl); var tok = await establish(id, newNonce()); setSession(tok); return mvAuth.principalOfToken(tok); },
    logout: function () { document.cookie = "mv_session=; path=/; max-age=0"; },
    _sha256: sha256, _sha224: sha224, _principal: bytesToPrincipal
  };
})();
