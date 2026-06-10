#!/usr/bin/env node
/*
 * palette-test.js — R5 binding-PALETTE extraction test.
 *
 * The generator may only call service functions that actually EXIST, with the
 * types they actually have. signatures.js extracts that palette from a project's
 * src/Services/*.mo. This test asserts it pulls the expected public surface from
 * the fixture's `Notes` service (tools/studio/gen-test/fixture/src/Services/Notes.mo):
 *
 *   - the service is named `Notes`;
 *   - the public funcs add/all/get/count are present with the right signatures;
 *   - the public type `Note` is present with its record shape;
 *   - the public let `emptyHint : Text` is present;
 *   - the upgrade-plumbing funcs mvStableSave/mvStableLoad are EXCLUDED (they are
 *     compiler-wired, not page-facing);
 *   - NO private (`func`/`type`/`let` without `public`) decl leaks in.
 *
 * Also sanity-checks the extractor against a REAL project (apps/bzzz Forum) so it
 * is not just fixture-shaped. Exit 0 = passed.
 */
'use strict';

const path = require('path');
const { extractProject } = require('../signatures.js');

const FIXTURE = path.resolve(__dirname, 'fixture');
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');

let ok = true;
function check(name, cond, detail) {
  if (!cond) ok = false;
  console.log(`  [${cond ? 'PASS' : 'FAIL'}] ${name}${cond ? '' : '  <- ' + (detail || '')}`);
}

// ── Fixture: the Notes service ────────────────────────────────────────────────
const palette = extractProject(FIXTURE);
const notes = palette.services.find((s) => s.name === 'Notes');

check('found the Notes service', !!notes, JSON.stringify(palette.services.map((s) => s.name)));

if (notes) {
  const funcNames = notes.decls.filter((d) => d.kind === 'func').map((d) => d.name).sort();
  const typeNames = notes.decls.filter((d) => d.kind === 'type').map((d) => d.name).sort();
  const letNames = notes.decls.filter((d) => d.kind === 'let').map((d) => d.name).sort();
  const sigOf = (n) => (notes.decls.find((d) => d.name === n) || {}).signature;

  check(
    'public funcs are exactly add/all/count/get',
    JSON.stringify(funcNames) === JSON.stringify(['add', 'all', 'count', 'get']),
    JSON.stringify(funcNames)
  );
  check('public type Note present', typeNames.includes('Note'), JSON.stringify(typeNames));
  check('public let emptyHint present', letNames.includes('emptyHint'), JSON.stringify(letNames));

  check('add signature exact', sigOf('add') === 'add(title : Text, body : Text) : Nat', sigOf('add'));
  check('all signature exact', sigOf('all') === 'all() : [Note]', sigOf('all'));
  check('get signature exact', sigOf('get') === 'get(id : Nat) : ?Note', sigOf('get'));
  check('count signature exact', sigOf('count') === 'count() : Nat', sigOf('count'));

  check(
    'Note type carries its record fields',
    /id : Nat/.test(sigOf('Note')) && /title : Text/.test(sigOf('Note')) && /body : Text/.test(sigOf('Note')) && /createdAt : Int/.test(sigOf('Note')),
    sigOf('Note')
  );
  check('emptyHint typed as Text', sigOf('emptyHint') === 'emptyHint : Text', sigOf('emptyHint'));

  // Persistence plumbing must NOT be in the palette.
  check(
    'mvStableSave/mvStableLoad EXCLUDED',
    !funcNames.includes('mvStableSave') && !funcNames.includes('mvStableLoad'),
    JSON.stringify(funcNames)
  );

  // No private decls leaked in (the only private decls are vars/lets without
  // `public` — `nextId`, `notes_` — which must not appear).
  const allNames = notes.decls.map((d) => d.name);
  check('no private state (nextId/notes_) leaked', !allNames.includes('nextId') && !allNames.includes('notes_'), JSON.stringify(allNames));
}

// ── Real project: apps/bzzz Forum (sanity — not just fixture-shaped) ──────────
const bzzz = extractProject(path.join(REPO_ROOT, 'apps', 'bzzz'));
const forum = bzzz.services.find((s) => s.name === 'Forum');
check('real project: found Forum service in apps/bzzz', !!forum, JSON.stringify(bzzz.services.map((s) => s.name)));
if (forum) {
  const fns = forum.decls.filter((d) => d.kind === 'func').map((d) => d.name);
  check('Forum exposes createTopic', fns.includes('createTopic'));
  check('Forum exposes categories', fns.includes('categories'));
  const createTopic = forum.decls.find((d) => d.name === 'createTopic');
  check(
    'createTopic signature matches generate.md contract',
    !!createTopic && createTopic.signature === 'createTopic(caller : Principal, handle : Text, categoryId : Nat, title : Text, tags : [Text], body : Text) : Nat',
    createTopic && createTopic.signature
  );
  const types = forum.decls.filter((d) => d.kind === 'type').map((d) => d.name);
  check('Forum exposes Category/Topic/Post types', ['Category', 'Topic', 'Post'].every((t) => types.includes(t)), JSON.stringify(types));
}

console.log('');
if (!ok) {
  console.error('FAIL: palette extraction did not match the expected service surface.');
  process.exit(1);
}
console.log('OK: signature palette extracts the project Motoko service signatures (funcs, types, lets), excluding persistence plumbing and privates.');
process.exit(0);
