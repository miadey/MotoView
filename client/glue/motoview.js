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
  }

  // The other keyed primitives the brain commands — all dumb DOM ops.
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
    if (n && n.parentNode) n.parentNode.removeChild(n);
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
        if (a) {
          a.classList.add("mv-anim-" + (value || "pulse"));
          setTimeout(function () { a.classList.remove("mv-anim-" + (value || "pulse")); }, 700);
        }
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

  function onClick(ev) {
    var el = handlerEl(ev.target, "click");
    if (!el) return;
    if (el.tagName === "BUTTON" || el.tagName === "A") ev.preventDefault();
    emit(fragFor(el, "click", null));
  }

  function onSubmit(ev) {
    var form = ev.target;
    if (!form || form.tagName !== "FORM" || !form.getAttribute("data-mv-handler")) return;
    ev.preventDefault();
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

    // Register the offline-first service worker the canister serves at /sw.js,
    // so every MotoView app is an installable PWA that works offline. Best
    // effort — a failure (e.g. unsupported browser) never blocks the app.
    if ("serviceWorker" in navigator) {
      navigator.serviceWorker.register("/sw.js").catch(function () {});
    }
  }

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
