/*
 * MotoView client glue ("the hands").
 *
 * All decision logic — adaptive polling, the render/event protocol, batch
 * interpretation, event sequencing — lives in the Rust→WASM "brain"
 * (/motoview.wasm). This file is only the unavoidable bridge to the Web APIs
 * the browser exposes solely to JavaScript: fetch, timers, and the DOM. It is
 * hand-written, dependency-free, and never touched by a bundler.
 */
(function () {
  "use strict";

  var wasm = null; // instance.exports
  var enc = new TextEncoder();
  var dec = new TextDecoder();

  function mem() {
    return new Uint8Array(wasm.memory.buffer);
  }

  // Read a (ptr,len) UTF-8 string out of WASM memory (copied).
  function readStr(ptr, len) {
    if (!len) return "";
    return dec.decode(mem().slice(ptr, ptr + len));
  }

  // Copy a JS string into a freshly-allocated WASM buffer; returns [ptr, len].
  // The receiving export takes ownership and frees it.
  function writeStr(s) {
    var bytes = enc.encode(s);
    if (bytes.length === 0) return [0, 0];
    var ptr = wasm.mv_alloc(bytes.length);
    mem().set(bytes, ptr);
    return [ptr, bytes.length];
  }

  // ---- host functions imported by the WASM brain --------------------------

  var imports = {
    env: {
      host_log: function (ptr, len) {
        console.log("[motoview]", readStr(ptr, len));
      },
      host_now: function () {
        return Date.now();
      },
      host_set_timer: function (id, ms) {
        setTimeout(function () {
          if (wasm) wasm.mv_on_timer(id >>> 0);
        }, ms);
      },
      host_fetch: function (reqId, mPtr, mLen, uPtr, uLen, bPtr, bLen) {
        var method = readStr(mPtr, mLen);
        var url = readStr(uPtr, uLen);
        var body = readStr(bPtr, bLen);
        var init = { method: method, headers: {}, cache: "no-store" };
        if (method === "POST") {
          init.headers["content-type"] = "application/x-www-form-urlencoded";
          init.body = body;
        }
        fetch(url, init)
          .then(function (r) {
            var status = r.status;
            return r.text().then(function (t) {
              deliver(reqId >>> 0, status >>> 0, t);
            });
          })
          .catch(function () {
            deliver(reqId >>> 0, 0, "");
          });
      },
      host_apply_html: function (tPtr, tLen, hPtr, hLen) {
        applyHtml(readStr(tPtr, tLen), readStr(hPtr, hLen));
      },
      host_replace_keyed: function (tPtr, tLen, kPtr, kLen, hPtr, hLen) {
        replaceKeyed(readStr(tPtr, tLen), readStr(kPtr, kLen), readStr(hPtr, hLen));
      },
      host_remove_keyed: function (tPtr, tLen, kPtr, kLen) {
        removeKeyed(readStr(tPtr, tLen), readStr(kPtr, kLen));
      },
      host_insert_keyed: function (tPtr, tLen, hPtr, hLen, aPtr, aLen) {
        insertKeyed(readStr(tPtr, tLen), readStr(hPtr, hLen), readStr(aPtr, aLen));
      },
      host_move_keyed: function (tPtr, tLen, kPtr, kLen, aPtr, aLen) {
        moveKeyed(readStr(tPtr, tLen), readStr(kPtr, kLen), readStr(aPtr, aLen));
      },
      host_effect: function (kPtr, kLen, tPtr, tLen, vPtr, vLen) {
        runEffect(readStr(kPtr, kLen), readStr(tPtr, tLen), readStr(vPtr, vLen));
      },
      host_navigate: function (uPtr, uLen) {
        var url = readStr(uPtr, uLen);
        if (url) window.location.assign(url);
      },
      host_set_title: function (ptr, len) {
        document.title = readStr(ptr, len);
      },
    },
  };

  function deliver(reqId, status, text) {
    var pl = writeStr(text);
    wasm.mv_on_response(reqId, status, pl[0], pl[1]);
  }

  // ---- batch application with focus/scroll/input preservation -------------

  function applyHtml(targetId, html) {
    var el = document.getElementById(targetId) || document.querySelector("[data-mv-root]");
    if (!el) return;

    // capture in-progress edit so polling never clobbers what you're typing
    var active = document.activeElement;
    var snap = null;
    if (active && (active.tagName === "INPUT" || active.tagName === "TEXTAREA" || active.tagName === "SELECT")) {
      snap = {
        key: active.getAttribute("data-mv-key") || active.getAttribute("name") || active.id,
        value: active.value,
        start: active.selectionStart,
        end: active.selectionEnd,
        type: active.type,
      };
    }
    var sx = window.scrollX, sy = window.scrollY;

    el.innerHTML = html;
    mvDecryptRendered(); // decrypt any [data-mv-decrypt] in the new render

    // restore the field the user was editing
    if (snap && snap.key) {
      var sel = "[data-mv-key=" + cssEscape(snap.key) + "]";
      var next = el.querySelector(sel) || document.getElementsByName(snap.key)[0];
      if (next) {
        if (next.value !== snap.value && (next.tagName === "INPUT" || next.tagName === "TEXTAREA")) {
          next.value = snap.value;
        }
        try {
          next.focus({ preventScroll: true });
          if (snap.start != null && next.setSelectionRange && /text|search|url|tel|password|textarea/i.test(snap.type || next.tagName)) {
            next.setSelectionRange(snap.start, snap.end);
          }
        } catch (e) {}
      }
    }
    window.scrollTo(sx, sy);
  }

  // Replace a single keyed region — a primitive the brain commands. Unchanged
  // regions are never touched here, so their live state is preserved.
  function replaceKeyed(targetId, key, html) {
    var root = document.getElementById(targetId) || document.querySelector("[data-mv-root]");
    if (!root) return;
    var el = root.querySelector("[data-mv-key=" + cssEscape(key) + "]");
    if (!el) return;
    var active = document.activeElement, snap = null;
    if (active && el.contains(active) && (active.tagName === "INPUT" || active.tagName === "TEXTAREA" || active.tagName === "SELECT")) {
      snap = { key: active.getAttribute("data-mv-key") || active.getAttribute("name") || active.id, value: active.value, start: active.selectionStart, end: active.selectionEnd, type: active.type };
    }
    el.outerHTML = html;
    if (snap && snap.key) {
      var next = root.querySelector("[data-mv-key=" + cssEscape(snap.key) + "]") || document.getElementsByName(snap.key)[0];
      if (next) {
        if (next.value !== snap.value && (next.tagName === "INPUT" || next.tagName === "TEXTAREA")) next.value = snap.value;
        try {
          next.focus({ preventScroll: true });
          if (snap.start != null && next.setSelectionRange && /text|search|url|tel|password|textarea/i.test(snap.type || next.tagName)) next.setSelectionRange(snap.start, snap.end);
        } catch (e) {}
      }
    }
    mvDecryptRendered(); // a keyed patch may carry fresh [data-mv-decrypt] ciphertext
  }

  // The other keyed primitives the brain commands — all dumb DOM ops.
  // ---- animation primitives: CSS @keyframes do the animating; these dumb
  // helpers only toggle the `mv-anim-<name>` class and clean it up on
  // animationend. The brain decides when (insert/remove ops, @animate effects).
  function playAnim(el, name) {
    if (!el || !name) return;
    var cls = "mv-anim-" + name;
    el.classList.add(cls);
    var done = function () {
      el.classList.remove(cls);
      el.removeEventListener("animationend", done);
    };
    el.addEventListener("animationend", done);
  }
  function playExit(el, name, after) {
    var fin = false;
    var done = function () {
      if (fin) return;
      fin = true;
      el.removeEventListener("animationend", done);
      after();
    };
    el.addEventListener("animationend", done);
    el.classList.add("mv-anim-" + name);
    setTimeout(done, 900); // safety net if animationend never fires
  }

  function rootFor(targetId) {
    return document.getElementById(targetId) || document.querySelector("[data-mv-root]");
  }
  function findKeyed(root, key) {
    return root.querySelector("[data-mv-key=" + cssEscape(key) + "]");
  }
  function nodeFromHtml(html) {
    var tpl = document.createElement("template");
    tpl.innerHTML = html;
    return tpl.content.firstElementChild;
  }
  function removeKeyed(targetId, key) {
    var root = rootFor(targetId);
    if (!root) return;
    var n = findKeyed(root, key);
    if (!n || !n.parentNode) return;
    var ex = n.getAttribute("data-mv-exit");
    if (ex) {
      playExit(n, ex, function () { if (n.parentNode) n.parentNode.removeChild(n); });
    } else {
      n.parentNode.removeChild(n);
    }
  }
  function insertKeyed(targetId, html, afterKey) {
    var root = rootFor(targetId);
    if (!root) return;
    var node = nodeFromHtml(html);
    if (!node) return;
    if (afterKey) {
      var a = findKeyed(root, afterKey);
      if (a) a.parentNode.insertBefore(node, a.nextSibling);
    } else {
      var first = root.querySelector("[data-mv-key]");
      if (first) first.parentNode.insertBefore(node, first);
      else root.appendChild(node);
    }
    var en = node.getAttribute && node.getAttribute("data-mv-enter");
    if (en) playAnim(node, en);
    mvDecryptRendered(); // an inserted keyed node may carry [data-mv-decrypt] ciphertext
  }
  function moveKeyed(targetId, key, afterKey) {
    var root = rootFor(targetId);
    if (!root) return;
    var node = findKeyed(root, key);
    if (!node) return;
    if (afterKey) {
      var a = findKeyed(root, afterKey);
      if (a && a !== node) a.parentNode.insertBefore(node, a.nextSibling);
    } else {
      var first = root.querySelector("[data-mv-key]");
      if (first && first !== node) first.parentNode.insertBefore(node, first);
    }
  }

  function runEffect(kind, target, value) {
    switch (kind) {
      case "focus": {
        var f = document.querySelector(target);
        if (f) try { f.focus(); } catch (e) {}
        break;
      }
      case "scrollTo": {
        var s = document.querySelector(target);
        if (s) s.scrollIntoView({ behavior: "smooth", block: "start" });
        break;
      }
      case "toast":
        toast(target);
        break;
      case "animate": {
        var a = document.querySelector(target);
        if (a) playAnim(a, value || "pulse");
        break;
      }
    }
  }

  function toast(message) {
    var host = document.getElementById("mv-toasts");
    if (!host) {
      host = document.createElement("div");
      host.id = "mv-toasts";
      document.body.appendChild(host);
    }
    var t = document.createElement("div");
    t.className = "mv-toast";
    t.textContent = message;
    host.appendChild(t);
    setTimeout(function () { t.classList.add("mv-toast-out"); }, 2600);
    setTimeout(function () { if (t.parentNode) t.parentNode.removeChild(t); }, 3000);
  }

  function cssEscape(s) {
    if (window.CSS && CSS.escape) return CSS.escape(s);
    return '"' + String(s).replace(/"/g, '\\"') + '"';
  }

  // ---- DOM event delegation -> the WASM brain -----------------------------

  function enc1(k, v) {
    return encodeURIComponent(k) + "=" + encodeURIComponent(v == null ? "" : v);
  }

  function emit(frag) {
    var pl = writeStr(frag);
    wasm.mv_on_event(pl[0], pl[1]);
  }

  function handlerEl(start, evName) {
    var el = start;
    while (el && el.nodeType === 1) {
      if (el.getAttribute && el.getAttribute("data-mv-handler") && el.getAttribute("data-mv-event") === evName) {
        return el;
      }
      el = el.parentElement;
    }
    return null;
  }

  function fragFor(el, evName, extraValue) {
    var parts = [enc1("__mv_handler", el.getAttribute("data-mv-handler")), enc1("__mv_event", evName)];
    // baked-in render-time argument values
    for (var i = 0; ; i++) {
      var a = el.getAttribute("data-mv-arg" + i);
      if (a == null) break;
      parts.push(enc1("__mv_arg" + i, a));
    }
    // for @input/@change, the live value is arg0
    if (extraValue != null) parts.push(enc1("__mv_arg0", extraValue));
    return parts.join("&");
  }

  // Apply + persist a theme key (data-theme + the mv_theme cookie the server's
  // inline head script reads on later loads).
  function mvSetTheme(key) {
    document.documentElement.setAttribute("data-theme", key);
    document.cookie = "mv_theme=" + key + "; path=/; max-age=31536000; samesite=lax";
  }
  // Mark the active option + summary label in every theme picker on the page.
  function mvPaintThemePicker() {
    var pickers = document.querySelectorAll(".mv-theme-picker");
    if (!pickers.length) return;
    var cur = document.documentElement.getAttribute("data-theme");
    if (!cur) cur = (window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches) ? "web-dark" : "web-light";
    if (cur === "light") cur = "web-light"; else if (cur === "dark") cur = "web-dark";
    for (var i = 0; i < pickers.length; i++) {
      var opts = pickers[i].querySelectorAll("[data-mv-theme-set]"), label = "";
      for (var j = 0; j < opts.length; j++) {
        var active = opts[j].getAttribute("data-mv-theme-set") === cur;
        opts[j].setAttribute("aria-current", active ? "true" : "false");
        if (active) { var l = opts[j].querySelector(".mv-theme-opt-label"); label = (l || opts[j]).textContent.trim(); }
      }
      var lab = pickers[i].querySelector(".mv-theme-picker-label");
      if (lab && label) lab.textContent = label;
    }
  }

  function onClick(ev) {
    // Theme switch: flip <html data-theme> instantly and persist the choice in the
    // mv_theme cookie (the server's inline head script applies it on later loads,
    // even for certified pages). A dumb framework primitive — no app logic.
    var t = ev.target.closest ? ev.target.closest("[data-mv-theme-toggle]") : null;
    if (t) {
      ev.preventDefault();
      var cur = document.documentElement.getAttribute("data-theme");
      if (!cur) cur = (window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches) ? "dark" : "light";
      var next = cur === "dark" ? "light" : "dark";
      mvSetTheme(next);
      return;
    }
    // Theme PICKER: a [data-mv-theme-set="<key>"] element selects a named Fluent
    // theme (web-light/web-dark/teams-light/teams-dark/hc).
    var ps = ev.target.closest ? ev.target.closest("[data-mv-theme-set]") : null;
    if (ps) {
      ev.preventDefault();
      mvSetTheme(ps.getAttribute("data-mv-theme-set"));
      var dt = ps.closest("details"); if (dt) dt.open = false;
      mvPaintThemePicker();
      return;
    }
    var el = handlerEl(ev.target, "click");
    if (!el) return;
    if (el.tagName === "BUTTON" || el.tagName === "A") ev.preventDefault();
    emit(fragFor(el, "click", null));
  }

  function onSubmit(ev) {
    var form = ev.target;
    if (!form || form.tagName !== "FORM" || !form.getAttribute("data-mv-handler")) return;
    ev.preventDefault();
    // Zero-trust: encrypt marked fields IN THE BROWSER before the form is sent,
    // so the canister only ever receives ciphertext.
    //   [data-mv-encrypt]            -> IBE-encrypt to the SESSION caller (self):
    //                                   the single-reader vault pattern.
    //   [data-mv-encrypt-to="p1 p2"] -> fan-out: one IBE envelope per recipient
    //                                   principal, the field value becoming
    //                                   newline-joined "<principal> <ciphertext>"
    //                                   lines. Lets EVERY recipient (not just the
    //                                   sender) decrypt — the DM / group pattern.
    // Empty fields are left untouched so server-side `required` still fires.
    var jobs = [], i;
    if (window.mvCrypto) {
      var selfFields = form.querySelectorAll("[data-mv-encrypt]");
      for (i = 0; i < selfFields.length; i++) {
        (function (f) { if (f.value) jobs.push(mvCrypto.encrypt(f.value).then(function (ct) { f.value = ct; })); })(selfFields[i]);
      }
      var fanFields = form.querySelectorAll("[data-mv-encrypt-to]");
      for (i = 0; i < fanFields.length; i++) {
        (function (f) {
          var recips = (f.getAttribute("data-mv-encrypt-to") || "").split(/\s+/).filter(Boolean);
          if (!f.value || !recips.length) return;
          var pt = f.value;
          jobs.push(Promise.all(recips.map(function (p) {
            return mvCrypto.encryptTo(p, pt).then(function (ct) { return p + " " + ct; });
          })).then(function (lines) { f.value = lines.join("\n"); }));
        })(fanFields[i]);
      }
    }
    if (jobs.length) {
      Promise.all(jobs).then(function () { submitForm(form); }).catch(function (e) { console.error("[motoview] encrypt failed", e); });
      return;
    }
    submitForm(form);
  }
  function submitForm(form) {
    var parts = [enc1("__mv_handler", form.getAttribute("data-mv-handler")), enc1("__mv_event", "submit")];
    var fd = new FormData(form);
    fd.forEach(function (v, k) { parts.push(enc1(k, v)); });
    if (form.getAttribute("data-mv-secure") === "1") {
      parts.push(enc1("__mv_secure", "1"));
      parts.push(enc1("__mv_token", form.getAttribute("data-mv-token") || ""));
      parts.push(enc1("__mv_schema", form.getAttribute("data-mv-schema") || ""));
    }
    emit(parts.join("&"));
  }

  // Zero-trust: decrypt any [data-mv-decrypt="<base64 ciphertext>"] element in the
  // browser, so the canister only ever served ciphertext. Idempotent.
  function mvDecryptRendered() {
    if (!window.mvCrypto) return;
    var els = document.querySelectorAll("[data-mv-decrypt]");
    for (var i = 0; i < els.length; i++) {
      (function (el) {
        if (el.__mvDec) return; el.__mvDec = true;
        var ct = el.getAttribute("data-mv-decrypt");
        if (!ct) return;
        if (!el.textContent) el.textContent = "🔒 decrypting…";
        mvCrypto.decrypt(ct).then(function (pt) { el.textContent = pt; el.setAttribute("data-mv-decrypted", "1"); })
          .catch(function () { el.textContent = "🔒 locked"; });
      })(els[i]);
    }
  }

  function onInput(ev) {
    var el = handlerEl(ev.target, "input") || handlerEl(ev.target, "change");
    if (!el) return;
    var evName = el.getAttribute("data-mv-event");
    emit(fragFor(el, evName, ev.target.value));
  }

  // ---- drag & drop -> server-driven move event ----------------------------
  // Cards: draggable="true" data-mv-drag="<id>". Drop zones: data-mv-drop="<handler>"
  // data-mv-dropval="<value>". On drop the WASM brain receives handler(id, value).
  var dragVal = null;

  function closestWith(el, attr) {
    while (el && el.nodeType === 1) {
      if (el.hasAttribute && el.hasAttribute(attr)) return el;
      el = el.parentElement;
    }
    return null;
  }

  function onDragStart(ev) {
    var el = closestWith(ev.target, "data-mv-drag");
    if (!el) return;
    dragVal = el.getAttribute("data-mv-drag");
    if (ev.dataTransfer) {
      ev.dataTransfer.effectAllowed = "move";
      try { ev.dataTransfer.setData("text/plain", dragVal); } catch (e) {}
    }
    el.classList.add("mv-dragging");
  }

  function clearDropHighlights() {
    var hi = document.querySelectorAll(".mv-drop-over");
    for (var i = 0; i < hi.length; i++) hi[i].classList.remove("mv-drop-over");
  }

  function onDragEnd(ev) {
    var el = closestWith(ev.target, "data-mv-drag");
    if (el) el.classList.remove("mv-dragging");
    clearDropHighlights();
  }

  function onDragOver(ev) {
    var col = closestWith(ev.target, "data-mv-drop");
    if (col) {
      ev.preventDefault();
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = "move";
      col.classList.add("mv-drop-over");
    }
  }

  function onDragLeave(ev) {
    var col = closestWith(ev.target, "data-mv-drop");
    if (col && !col.contains(ev.relatedTarget)) col.classList.remove("mv-drop-over");
  }

  function onDrop(ev) {
    var col = closestWith(ev.target, "data-mv-drop");
    if (!col) return;
    ev.preventDefault();
    clearDropHighlights();
    var handler = col.getAttribute("data-mv-drop");
    var dropval = col.getAttribute("data-mv-dropval") || "";
    if (dragVal != null && handler) {
      emit([enc1("__mv_handler", handler), enc1("__mv_event", "drop"),
            enc1("__mv_arg0", dragVal), enc1("__mv_arg1", dropval)].join("&"));
    }
    dragVal = null;
  }

  // ---- bootstrap ----------------------------------------------------------

  function start() {
    var path = window.location.pathname || "/";
    var root = document.getElementById("mv-root") || document.querySelector("[data-mv-root]");
    var batch = (root && root.getAttribute("data-mv-batch")) || "";
    var seed = root ? root.innerHTML : "";
    var p = writeStr(path);
    var b = writeStr(batch);
    var s = writeStr(seed);
    wasm.mv_start(p[0], p[1], b[0], b[1], s[0], s[1]);

    document.addEventListener("click", onClick, true);
    document.addEventListener("submit", onSubmit, true);
    document.addEventListener("input", onInput, true);
    document.addEventListener("change", onInput, true);
    document.addEventListener("dragstart", onDragStart, true);
    document.addEventListener("dragend", onDragEnd, true);
    document.addEventListener("dragover", onDragOver, true);
    document.addEventListener("dragleave", onDragLeave, true);
    document.addEventListener("drop", onDrop, true);
    document.addEventListener("visibilitychange", function () {
      if (wasm) wasm.mv_on_visibility(document.hidden ? 1 : 0);
    });

    mvDecryptRendered(); // decrypt any server-rendered ciphertext on first paint
    mvPaintThemePicker(); // mark the active option in any theme picker

    // Register the offline-first service worker the canister serves at /sw.js,
    // so every MotoView app is an installable PWA that works offline. Best
    // effort — a failure (e.g. unsupported browser) never blocks the app.
    if ("serviceWorker" in navigator) {
      navigator.serviceWorker.register("/sw.js").catch(function () {});
    }
  }

  // ---- mvCrypto: opt-in client-side vetKeys/IBE. Loads /motoview-crypto.wasm
  // (the Rust crypto module — all BLS/IBE lives there); the glue only marshals
  // bytes and fetches the canister's session-bound key. No crypto in JS. ----
  var mvCrypto = (function () {
    var C = null, session = null;
    var enc = new TextEncoder(), dec = new TextDecoder();
    function memv() { return new Uint8Array(C.memory.buffer); }
    function rand(n) { var a = new Uint8Array(n); crypto.getRandomValues(a); return a; }
    function call(op, inputs) {
      var allocd = [], args = [], i;
      for (i = 0; i < inputs.length; i++) {
        var arr = inputs[i], p = C.mvc_alloc(arr.length);
        memv().set(arr, p);
        allocd.push([p, arr.length]); args.push(p, arr.length);
      }
      var status = C[op].apply(null, args);
      for (i = 0; i < allocd.length; i++) C.mvc_dealloc(allocd[i][0], allocd[i][1]);
      if (status !== 0) throw new Error(op + " failed (status " + status + ")");
      var o = C.mvc_out_ptr(), l = C.mvc_out_len();
      return memv().slice(o, o + l);
    }
    function load() {
      if (C) return Promise.resolve();
      return fetch("/motoview-crypto.wasm", { cache: "force-cache" })
        .then(function (r) { return r.arrayBuffer(); })
        .then(function (buf) { return WebAssembly.instantiate(buf, {}); })
        .then(function (res) { C = res.instance.exports; });
    }
    function getBytes(path, opts) {
      return fetch(path, opts).then(function (r) { return r.arrayBuffer(); }).then(function (b) { return new Uint8Array(b); });
    }
    // principal text -> raw bytes (base32 decode, drop the 4-byte CRC) = the
    // IBE identity / derivation input, matching the canister's Principal.toBlob.
    function principalBytes(text) {
      var B32 = "abcdefghijklmnopqrstuvwxyz234567", s = (text || "").replace(/-/g, ""), bits = 0, val = 0, out = [], i;
      for (i = 0; i < s.length; i++) { val = (val << 5) | B32.indexOf(s[i]); bits += 5; if (bits >= 8) { out.push((val >> (bits - 8)) & 255); bits -= 8; } }
      return new Uint8Array(out).slice(4);
    }
    // Establish the session vetKey for the current (session-authenticated) caller.
    function establish() {
      if (session) return Promise.resolve(session);
      return load().then(function () {
        return window.mvAuth ? mvAuth.whoami() : Promise.resolve("2vxsx-fae");
      }).then(function (who) {
        var id = principalBytes(who || "2vxsx-fae");
        var sk = call("mvc_transport_secret_from_seed", [rand(32)]);
        var pk = call("mvc_transport_public", [sk]);
        return Promise.all([
          getBytes("/_motoview/vetkd/derive", { method: "POST", credentials: "same-origin", body: pk }),
          getBytes("/_motoview/vetkd/public-key", { credentials: "same-origin" })
        ]).then(function (res) {
          var vetkey = call("mvc_unwrap_vetkey", [res[1], res[0], id, sk]);
          session = { vetkey: vetkey, master: res[1], id: id };
          return session;
        });
      });
    }
    function toB64(u8) { var s = "", i; for (i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]); return btoa(s); }
    function fromB64(b64) { var s = atob(b64), u = new Uint8Array(s.length), i; for (i = 0; i < s.length; i++) u[i] = s.charCodeAt(i); return u; }
    return {
      reset: function () { session = null; },
      encrypt: function (text) { return establish().then(function (s) { return toB64(call("mvc_ibe_encrypt", [s.master, s.id, enc.encode(text), rand(32)])); }); },
      // Encrypt TO another principal's identity (not self). Uses the same canister
      // master public key (identity-independent); only that principal can derive
      // the matching vetKey and decrypt. The basis for multi-reader (DM) E2EE.
      encryptTo: function (principalText, text) { return establish().then(function (s) { return toB64(call("mvc_ibe_encrypt", [s.master, principalBytes(principalText), enc.encode(text), rand(32)])); }); },
      decrypt: function (b64) { return establish().then(function (s) { return dec.decode(call("mvc_ibe_decrypt", [s.vetkey, fromB64(b64)])); }); },
      selfTest: function (msg) {
        msg = msg || "hello zero-trust";
        return this.encrypt(msg).then(function (ct) {
          return mvCrypto.decrypt(ct).then(function (pt) { return { plaintext: msg, ciphertextB64Len: ct.length, decrypted: pt, ok: pt === msg }; });
        });
      }
    };
  })();
  window.mvCrypto = mvCrypto;

  function boot() {
    fetch("/motoview.wasm", { cache: "no-store" })
      .then(function (r) { return r.arrayBuffer(); })
      .then(function (buf) { return WebAssembly.instantiate(buf, imports); })
      .then(function (res) {
        wasm = res.instance.exports;
        if (document.readyState === "loading") {
          document.addEventListener("DOMContentLoaded", start);
        } else {
          start();
        }
      })
      .catch(function (e) { console.error("[motoview] failed to start:", e); });
  }

  boot();
})();
