#!/usr/bin/env node
/*
 * bridge-test.js — verify the diagnostics bridge round-trips through the REAL
 * compiler and emits the R2 position payload the editor consumes for squiggles.
 *
 * Asserts:
 *   1. A clean buffer -> saveable:true, no error-severity diagnostics.
 *   2. An unsecured mutating <form @submit> -> saveable:false with a `secure-form`
 *      ERROR carrying 1-based {line,col,endLine,endCol} positions (the exact shape
 *      apps/studio/assets/mview-editor.js -> toCmDiagnostic turns into a marker).
 *
 * Exit 0 = passed. Requires $MOTOVIEW or the repo-root release compiler.
 */
'use strict';

const path = require('path');
const fs = require('fs');
const { diagnose } = require('./diagnostics-server.js');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
let ok = true;
function check(name, cond, detail) {
  if (!cond) ok = false;
  console.log(`  [${cond ? 'PASS' : 'FAIL'}] ${name}${cond ? '' : '  <- ' + (detail || '')}`);
}

// 1. clean buffer
const clean = fs.readFileSync(
  path.join(REPO_ROOT, 'apps', 'studio', 'src', 'Pages', 'Design.mview'),
  'utf8'
);
const r1 = diagnose(clean);
check('clean buffer is saveable', r1.saveable === true, `saveable=${r1.saveable}`);
check(
  'clean buffer has no error diagnostics',
  r1.diagnostics.every((d) => d.severity !== 'error')
);

// 2. insecure mutating form
const bad = [
  '@page "/"',
  '@layout StudioLayout',
  '@title "x"',
  '',
  '<form @submit="save">',
  '  <input name="amount">',
  '  <button>Send</button>',
  '</form>',
  '',
  '@code {',
  '  func save(ctx : Context) : async () { };',
  '}',
  ''
].join('\n');
const r2 = diagnose(bad);
check('insecure form is NOT saveable', r2.saveable === false, `saveable=${r2.saveable}`);
const err = r2.diagnostics.find((d) => d.severity === 'error');
check('insecure form yields an error diagnostic', !!err);
check('error is the secure-form rule', !!err && err.rule === 'secure-form', err && err.rule);
check(
  'error carries 1-based positions',
  !!err && err.line >= 1 && err.col >= 1 && err.endLine >= err.line && err.endCol >= 1,
  err && JSON.stringify({ line: err.line, col: err.col, endLine: err.endLine, endCol: err.endCol })
);

console.log('');
if (!ok) {
  console.error('FAIL: bridge round-trip assertions failed.');
  process.exit(1);
}
console.log('OK: diagnostics bridge round-trips through the real compiler with R2 positions.');
process.exit(0);
