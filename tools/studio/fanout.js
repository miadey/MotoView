#!/usr/bin/env node
'use strict';
/*
 * fanout.js — R10 3-UP CANVAS forest fan-out (the "one edit -> web + iOS + Android"
 * differentiator). ONE IR forest drives THREE renderers; this module computes, for
 * each platform pane, the data that pane consumes — faithfully to the REAL native
 * NativeView mappings (clients/ios/Sources/MotoViewKit/NativeView.swift and
 * clients/android/.../ui/NativeView.kt), which are 1:1 with each other.
 *
 * The fan-out is a pure function of the forest: `fanout(forest)` -> { web, ios,
 * android } descriptors. The web descriptor mirrors the DOM mapping the preview
 * harness already uses (el->tag, text->text, raw->innerHTML). The ios/android
 * descriptors mirror the native mapping:
 *   div/section/main/nav/ul/ol/li/...      -> a vertical container (VStack / Column)
 *   span/p/h1..h6/a/label/strong/em/...    -> a text view (Text), bold for headings
 *   button                                 -> a native Button(emit(event)) with args
 *   raw                                    -> the not-yet-native HTML leaf (WebView
 *                                             fallback), flagged honestly
 *
 * This module is HEADLESS-TESTED (fanout-test.js): one forest in -> the three pane
 * descriptors out, asserted node-for-node. The studio `--serve` panel renders the
 * SAME descriptors into three columns; that VISUAL is browser-flagged, but it can
 * never silently drift from the tested core because both consume this function.
 *
 * Honest scope: the iOS/Android columns are a FOREST-FAITHFUL preview (the same
 * native element/text/button classification, styled per platform), NOT a running
 * simulator. True native panes need Xcode/Android Studio simulator processes; this
 * proves the forest fans out to the data each pane would consume.
 */

// The tag families, copied verbatim from BOTH native renderers (they agree 1:1).
const BLOCK_TAGS = new Set([
  'div', 'section', 'main', 'nav', 'header', 'footer', 'article',
  'aside', 'ul', 'ol', 'li', 'form', 'fieldset', 'figure',
]);
const TEXT_TAGS = new Set([
  'span', 'p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
  'a', 'label', 'strong', 'em', 'b', 'i', 'small', 'code', 'pre', 'blockquote',
]);
const BOLD_TAGS = new Set(['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'strong', 'b']);

/** Strip HTML tags from a raw string (matches the native `stripTags`). */
function stripTags(html) {
  let out = '';
  let inTag = false;
  for (const c of html || '') {
    if (c === '<') inTag = true;
    else if (c === '>') inTag = false;
    else if (!inTag) out += c;
  }
  return out;
}

/** Flatten a node subtree to its text (matches the native `flattenText`). */
function flattenText(nodes) {
  let out = '';
  for (const n of nodes || []) {
    if (n.t === 'text') out += n.value || '';
    else if (n.t === 'raw') out += stripTags(n.html || '');
    else if (n.t === 'el') out += flattenText(n.children || []);
  }
  return out;
}

/** The handler + args a `button` element carries (matches the native emit wiring). */
function buttonEmit(node) {
  const ev = node.events || {};
  const handler = ev.click || Object.values(ev)[0] || '';
  const args = {};
  const attrs = node.attrs || {};
  for (const k in attrs) if (k.startsWith('data-mv-arg')) args[k] = attrs[k];
  return { event: 'click', handler, args };
}

/**
 * Map ONE IR node to a NATIVE pane descriptor (platform-agnostic; the SwiftUI and
 * Compose renderers are 1:1). Kinds: 'container' (VStack/Column), 'text' (Text),
 * 'button' (Button), 'raw' (the WebView fallback leaf), 'text-leaf' (a bare text
 * node). Whitespace-only raw nodes are dropped (they are layout glue, not content),
 * matching the native renderers which never surface them as views.
 */
function nativeNode(node) {
  if (node.t === 'text') return { kind: 'text-leaf', value: node.value || '' };
  if (node.t === 'raw') {
    if (!stripTags(node.html).trim()) return null; // whitespace glue -> dropped
    return { kind: 'raw', html: node.html || '', native: false };
  }
  if (node.t === 'el') {
    const tag = (node.tag || '').toLowerCase();
    if (tag === 'button') {
      return { kind: 'button', tag, label: flattenText(node.children), emit: buttonEmit(node), native: true };
    }
    if (TEXT_TAGS.has(tag)) {
      // A text view renders its FLATTENED text, bold for headings/strong.
      return { kind: 'text', tag, text: flattenText(node.children), bold: BOLD_TAGS.has(tag), native: true };
    }
    // block tags + unknown tags -> a vertical container of child views.
    const children = (node.children || []).map(nativeNode).filter(Boolean);
    return { kind: 'container', tag, children, native: true };
  }
  return null;
}

/** The native pane (iOS/Android) descriptor for a whole forest. */
function nativePane(forest) {
  return (Array.isArray(forest) ? forest : [forest]).map(nativeNode).filter(Boolean);
}

/**
 * Map ONE IR node to a WEB pane descriptor (mirrors the harness DOM mapping:
 * el->element, text->text, raw->innerHTML). Whitespace raw is KEPT for the web pane
 * (HTML preserves it), so the web descriptor is the faithful DOM, while the native
 * panes drop layout glue — exactly the real behaviour difference.
 */
function webNode(node) {
  if (node.t === 'text') return { kind: 'text', value: node.value || '' };
  if (node.t === 'raw') return { kind: 'raw', html: node.html || '' };
  if (node.t === 'el') {
    return {
      kind: 'el',
      tag: node.tag || 'div',
      attrs: node.attrs || {},
      events: node.events || {},
      children: (node.children || []).map(webNode),
    };
  }
  return { kind: 'unknown' };
}

function webPane(forest) {
  return (Array.isArray(forest) ? forest : [forest]).map(webNode);
}

/**
 * Fan ONE IR forest out to the THREE panes. The web pane is the live DOM mapping;
 * the iOS + Android panes share the SAME native descriptor (their renderers are
 * 1:1), labelled per platform. Returns a stable, JSON-serializable object — the
 * data each pane consumes from the single source forest.
 */
function fanout(forest) {
  const web = webPane(forest);
  const native = nativePane(forest);
  return {
    source: 'one IR forest',
    web: { platform: 'web', renderer: 'DOM (live)', nodes: web },
    ios: { platform: 'ios', renderer: 'SwiftUI NativeView (forest-faithful preview)', nodes: native },
    android: { platform: 'android', renderer: 'Compose NativeView (forest-faithful preview)', nodes: native },
  };
}

/** Count the interactive buttons (with their handlers) a native pane exposes. */
function paneButtons(paneNodes) {
  const out = [];
  (function walk(ns) {
    for (const n of ns) {
      if (n.kind === 'button') out.push({ label: n.label, handler: n.emit.handler });
      if (n.children) walk(n.children);
    }
  })(paneNodes);
  return out;
}

module.exports = {
  BLOCK_TAGS,
  TEXT_TAGS,
  BOLD_TAGS,
  stripTags,
  flattenText,
  buttonEmit,
  nativeNode,
  nativePane,
  webNode,
  webPane,
  fanout,
  paneButtons,
};

// ── CLI ───────────────────────────────────────────────────────────────────────
if (require.main === module) {
  const fs = require('fs');
  const arg = process.argv[2];
  const raw = arg ? fs.readFileSync(arg, 'utf8') : fs.readFileSync(0, 'utf8');
  // Accept a raw forest JSON (possibly with stderr noise): take the last [..] line.
  const line = raw
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.startsWith('[') && l.endsWith(']'))
    .pop() || raw;
  const forest = JSON.parse(line);
  process.stdout.write(JSON.stringify(fanout(forest), null, 2) + '\n');
}
