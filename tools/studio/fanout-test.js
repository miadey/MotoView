#!/usr/bin/env node
'use strict';
/*
 * fanout-test.js — R10 3-UP CANVAS forest fan-out test (HEADLESS).
 *
 * Proves the leapfrog claim "one edit -> web + iOS + Android from ONE IR forest":
 * a single forest (the REAL one from `motoview preview examples/counter`, or a
 * fallback fixture if moc is absent) fans out to the THREE pane descriptors, and we
 * assert, node-for-node, that:
 *   - the WEB pane is the faithful DOM mapping (4 buttons under the actions div);
 *   - the iOS + Android panes share the SAME native descriptor (their renderers are
 *     1:1) — the <strong> becomes a native Text reading the counter value, and the
 *     four <button>s become native Buttons carrying their real @click handlers;
 *   - the native panes DROP whitespace-only raw glue (as the real renderers do)
 *     while the web pane preserves it;
 *   - the SAME fanout() the studio panel inlines is what we test (panel parity), so
 *     the browser-flagged 3-up visual can't silently drift from this tested core.
 *
 * The VISUAL three columns are browser-flagged (rendered by the --serve panel);
 * this test asserts the DATA each column consumes.
 *
 * Exit 0 = passed.
 */

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');
const fanoutMod = require('./fanout.js');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const BIN = process.env.MOTOVIEW || path.join(REPO_ROOT, 'compiler', 'target', 'release', 'motoview');
const COUNTER = path.join(REPO_ROOT, 'examples', 'counter');

let ok = true;
function check(name, cond, detail) {
  if (!cond) ok = false;
  console.log(`  [${cond ? 'PASS' : 'FAIL'}] ${name}${cond ? '' : '  <- ' + (detail || '')}`);
}

// The counter page forest as a fallback if moc/preview is unavailable.
const FALLBACK_FOREST = [
  {
    t: 'el', tag: 'section', attrs: { class: 'mv-container' }, events: {},
    children: [
      { t: 'raw', html: '\n    ' },
      { t: 'el', tag: 'h1', attrs: {}, events: {}, children: [{ t: 'raw', html: 'Counter' }] },
      {
        t: 'el', tag: 'p', attrs: { class: 'counter-value' }, events: {},
        children: [
          { t: 'raw', html: 'Current value: ' },
          { t: 'el', tag: 'strong', attrs: {}, events: {}, children: [{ t: 'text', value: '0' }] },
        ],
      },
      {
        t: 'el', tag: 'div', attrs: { class: 'counter-actions' }, events: {},
        children: [
          { t: 'raw', html: '\n        ' },
          { t: 'el', tag: 'button', attrs: { 'data-mv-arg0': '1' }, events: { click: 'increment' }, children: [{ t: 'raw', html: '+1' }] },
          { t: 'raw', html: '\n        ' },
          { t: 'el', tag: 'button', attrs: { 'data-mv-arg0': '5' }, events: { click: 'increment' }, children: [{ t: 'raw', html: '+5' }] },
          { t: 'raw', html: '\n        ' },
          { t: 'el', tag: 'button', attrs: {}, events: { click: 'decrement' }, children: [{ t: 'raw', html: '-1' }] },
          { t: 'raw', html: '\n        ' },
          { t: 'el', tag: 'button', attrs: {}, events: { click: 'reset' }, children: [{ t: 'raw', html: 'Reset' }] },
        ],
      },
    ],
  },
];

let forest;
let source;
try {
  const out = execFileSync(BIN, ['preview', COUNTER], { encoding: 'utf8' });
  const line = out.split(/\r?\n/).map((l) => l.trim()).filter((l) => l.startsWith('[') && l.endsWith(']')).pop();
  forest = JSON.parse(line);
  source = 'motoview preview examples/counter (live IR forest)';
} catch (e) {
  forest = FALLBACK_FOREST;
  source = 'inline fixture (moc/preview unavailable: ' + (e.message || e) + ')';
}
console.log(`  forest source: ${source}`);

const out = fanoutMod.fanout(forest);

// ---- the SAME forest drives all three panes ---------------------------------
check('fan-out has web + ios + android panes', !!(out.web && out.ios && out.android));
check('panes declare their renderer', out.ios.renderer.includes('SwiftUI') && out.android.renderer.includes('Compose'));

// ---- iOS and Android consume the IDENTICAL native descriptor (renderers 1:1) -
check('iOS and Android native descriptors are identical', JSON.stringify(out.ios.nodes) === JSON.stringify(out.android.nodes));

// ---- WEB pane: faithful DOM, the actions div has 4 button elements ----------
function findEl(nodes, tag, cls) {
  for (const n of nodes) {
    if (n.kind === 'el' && n.tag === tag && (!cls || (n.attrs && n.attrs.class === cls))) return n;
    if (n.children) {
      const f = findEl(n.children, tag, cls);
      if (f) return f;
    }
  }
  return null;
}
const webSection = out.web.nodes.find((n) => n.kind === 'el' && n.tag === 'section');
check('web pane root is the <section>', !!webSection, JSON.stringify(out.web.nodes.map((n) => n.tag || n.kind)));
const webDiv = webSection && findEl(webSection.children, 'div', 'counter-actions');
const webButtons = (webDiv ? webDiv.children : []).filter((c) => c.kind === 'el' && c.tag === 'button');
check('web pane: actions div has 4 <button> elements', webButtons.length === 4, `got ${webButtons.length}`);

// ---- NATIVE panes: the 4 buttons become native Buttons w/ real handlers -----
const nativeButtons = fanoutMod.paneButtons(out.ios.nodes);
check('native pane: 4 native Buttons', nativeButtons.length === 4, `got ${nativeButtons.length}`);
const handlers = nativeButtons.map((b) => b.handler).sort();
check(
  'native Buttons carry the real @click handlers (increment x2, decrement, reset)',
  JSON.stringify(handlers) === JSON.stringify(['decrement', 'increment', 'increment', 'reset']),
  JSON.stringify(handlers)
);
const labels = nativeButtons.map((b) => b.label);
check('native Button labels are +1 / +5 / -1 / Reset', JSON.stringify(labels) === JSON.stringify(['+1', '+5', '-1', 'Reset']), JSON.stringify(labels));

// ---- the dynamic counter value reaches a native Text view -------------------
// The <p class="counter-value"> is a TEXT tag, so the native renderer flattens its
// whole subtree (incl. the <strong>0</strong>) into ONE Text view — exactly what
// SwiftUI/Compose do (a paragraph is one Text). So we assert the counter VALUE "0"
// is carried inside that flattened native Text node.
function collectTexts(nodes, out = []) {
  for (const n of nodes) {
    if (n.kind === 'text') out.push(n.text);
    if (n.children) collectTexts(n.children, out);
  }
  return out;
}
const texts = collectTexts(out.ios.nodes);
const counterText = texts.find((t) => t.includes('Current value:') && t.includes('0'));
check(
  'native pane: counter value reaches a Text view (flattened "Current value: 0")',
  !!counterText,
  JSON.stringify(texts)
);

// ---- native panes DROP whitespace-only raw glue; web keeps the DOM -----------
function countRaw(nodes) {
  let c = 0;
  for (const n of nodes) {
    if (n.kind === 'raw') c++;
    if (n.children) c += countRaw(n.children);
  }
  return c;
}
const nativeRaw = countRaw(out.ios.nodes);
const webRaw = countRaw(out.web.nodes);
check('native panes drop whitespace-only raw glue', nativeRaw < webRaw, `native=${nativeRaw} web=${webRaw}`);

// ---- PANEL PARITY: the studio --serve panel inlines fanout(); assert the panel
// copy yields the IDENTICAL fan-out so the browser-flagged 3-up can't drift. ----
try {
  const main = fs.readFileSync(path.join(REPO_ROOT, 'compiler', 'src', 'main.rs'), 'utf8');
  // The panel inlines a JS block tagged between // MV-FANOUT-BEGIN / // MV-FANOUT-END.
  // Skip the rest of the BEGIN marker line (it carries a trailing comment) so the
  // extracted body is valid JS starting at the next line.
  const m = main.match(/\/\/ MV-FANOUT-BEGIN[^\n]*\n([\s\S]*?)\/\/ MV-FANOUT-END/);
  check('panel inlines the fan-out core (MV-FANOUT markers present)', !!m);
  if (m) {
    const factory = new Function(m[1] + '\nreturn { fanout, paneButtons };');
    const panel = factory();
    const panelOut = panel.fanout(forest);
    check(
      'panel fanout() matches the module fanout() (no drift)',
      JSON.stringify(panelOut.ios.nodes) === JSON.stringify(out.ios.nodes) &&
        JSON.stringify(panelOut.web.nodes) === JSON.stringify(out.web.nodes),
      'panel/module fan-out differ'
    );
  }
} catch (e) {
  check('panel parity (inlined fan-out eval)', false, e.message);
}

console.log();
if (ok) {
  console.log('FANOUT (3-UP CANVAS) TEST: PASSED');
  process.exit(0);
} else {
  console.log('FANOUT (3-UP CANVAS) TEST: FAILED');
  process.exit(1);
}
