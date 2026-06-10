#!/usr/bin/env node
'use strict';
/*
 * view-tree-test.js — R7 observability: verify the view-hierarchy tree-builder
 * turns a REAL IR forest (from `motoview preview examples/counter`) into the
 * expected collapsible node tree — the button(s) + text MotoView renders for the
 * counter page, with the @click handlers lifted for click-to-source.
 *
 * Runs the real compiler's `preview` (moc -r, no deploy). If `moc` is absent we
 * fall back to an inline forest fixture so the tree-builder is still asserted;
 * the run prints which path it took.
 *
 * Exit 0 = passed.
 */

const path = require('path');
const { execFileSync } = require('child_process');
const obs = require('./observability.js');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const BIN = process.env.MOTOVIEW || path.join(REPO_ROOT, 'compiler', 'target', 'release', 'motoview');
const COUNTER = path.join(REPO_ROOT, 'examples', 'counter');

let ok = true;
function check(name, cond, detail) {
  if (!cond) ok = false;
  console.log(`  [${cond ? 'PASS' : 'FAIL'}] ${name}${cond ? '' : '  <- ' + (detail || '')}`);
}

// The counter page's authored markup, as a fallback forest if moc is missing.
const FALLBACK_FOREST = [
  {
    t: 'el',
    tag: 'section',
    attrs: { class: 'mv-container' },
    events: {},
    children: [
      { t: 'raw', html: '\n    ' },
      { t: 'el', tag: 'h1', attrs: {}, events: {}, children: [{ t: 'raw', html: 'Counter' }] },
      {
        t: 'el',
        tag: 'p',
        attrs: { class: 'counter-value' },
        events: {},
        children: [
          { t: 'raw', html: 'Current value: ' },
          { t: 'el', tag: 'strong', attrs: {}, events: {}, children: [{ t: 'text', value: '0' }] },
        ],
      },
      {
        t: 'el',
        tag: 'div',
        attrs: { class: 'counter-actions' },
        events: {},
        children: [
          { t: 'el', tag: 'button', attrs: {}, events: { click: 'increment' }, children: [{ t: 'raw', html: '+1' }] },
          { t: 'el', tag: 'button', attrs: {}, events: { click: 'increment' }, children: [{ t: 'raw', html: '+5' }] },
          { t: 'el', tag: 'button', attrs: {}, events: { click: 'decrement' }, children: [{ t: 'raw', html: '-1' }] },
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
  // preview prints the forest as the last JSON-array line.
  const line = out
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.startsWith('[') && l.endsWith(']'))
    .pop();
  forest = JSON.parse(line);
  source = 'motoview preview examples/counter (live IR forest)';
} catch (e) {
  forest = FALLBACK_FOREST;
  source = 'inline fixture (moc/preview unavailable: ' + (e.message || e) + ')';
}
console.log(`  forest source: ${source}`);

const roots = obs.buildViewTree(forest);

check('one root view-node (the <section>)', roots.length === 1, `got ${roots.length}`);
const section = roots[0] || {};
check('root is el <section>', section.kind === 'el' && section.tag === 'section', `${section.kind} ${section.tag}`);

// whitespace-only raw nodes are dropped -> section has h1, p, div
const tags = (section.children || []).map((c) => c.tag || c.kind);
check('section children are [h1, p, div] (whitespace dropped)', JSON.stringify(tags) === JSON.stringify(['h1', 'p', 'div']), JSON.stringify(tags));

// the <p> has a <strong> whose child is the dynamic text node "0"
const p = (section.children || []).find((c) => c.tag === 'p');
const strong = p && p.children.find((c) => c.tag === 'strong');
const textNode = strong && strong.children[0];
check('<strong> contains a text node', !!textNode && textNode.kind === 'text', textNode && textNode.kind);

// the action div has exactly four <button> elements, all carrying a @click
const div = (section.children || []).find((c) => c.tag === 'div');
const buttons = (div && div.children) || [];
check('div has 4 buttons', buttons.length === 4 && buttons.every((b) => b.tag === 'button'), `got ${buttons.length}`);
check('every button has a click handler', buttons.every((b) => b.handlers.some((h) => h.event === 'click')), JSON.stringify(buttons.map((b) => b.handlers)));

// click-to-source: a logged `increment` handler maps back to its firing node(s)
const incNodes = obs.nodesForHandler(roots, 'increment');
check('handler "increment" maps to 2 source nodes (+1 and +5)', incNodes.length === 2, `got ${incNodes.length}`);
const resetNodes = obs.nodesForHandler(roots, 'reset');
check('handler "reset" maps to 1 source node', resetNodes.length === 1, `got ${resetNodes.length}`);

// headless tree HTML renders the buttons + their handler badges
const html = obs.renderTreeHtml(roots);
check('tree html mentions <button>', html.includes('&lt;button&gt;'));
check('tree html carries the increment handler badge', html.includes('data-handler="increment"'));

// PANEL PARITY: the browser panel inlines the SAME pure core. Extract its
// `buildViewTree` from the inlined <script> and assert it yields the identical
// tree shape on this real forest, so the browser-flagged UI can't silently drift
// from the headless-tested module. (Only the pure functions are eval'd; no DOM.)
try {
  const fs = require('fs');
  const panelSrc = fs.readFileSync(path.join(__dirname, 'observability-panel.html'), 'utf8');
  const m = panelSrc.match(/<script>([\s\S]*?)<\/script>/);
  const body = m ? m[1] : '';
  // Cut the script at the DOM-wiring section (everything before `const $ =`),
  // so only the pure parser/tree-builder is evaluated.
  const pure = body.split('// ---- DOM wiring')[0];
  const factory = new Function(pure + '\nreturn { buildViewTree, parseLogStream };');
  const panel = factory();
  const panelRoots = panel.buildViewTree(forest);
  const shape = (ns) => ns.map((n) => ({ k: n.kind, t: n.tag, h: (n.handlers || []).map((x) => x.handler), c: shape(n.children || []) }));
  check('panel buildViewTree matches the module', JSON.stringify(shape(panelRoots)) === JSON.stringify(shape(roots)));
  const panelEvents = panel.parseLogStream('MV|dispatch|page=Counter|handler=increment|event=increment|caller=2vxsx-fae|lastBatch=b|costInstr=9');
  check('panel parseLogStream matches the module', panelEvents.length === 1 && panelEvents[0].handler === 'increment' && panelEvents[0].costInstr === 9);
} catch (e) {
  check('panel parity (inlined core eval)', false, e.message);
}

console.log();
if (ok) {
  console.log('VIEW-TREE TEST: PASSED');
  process.exit(0);
} else {
  console.log('VIEW-TREE TEST: FAILED');
  process.exit(1);
}
