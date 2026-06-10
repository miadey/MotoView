#!/usr/bin/env node
/*
 * gate-test.js — R5 UNBYPASSABLE-GATE test.
 *
 * Proves the gate cannot be talked past: a MOCK generator that ALWAYS returns a
 * broken artifact (a mutating <form @submit> WITHOUT `secure`, every round) must
 * cause the loop to EXHAUST its budget and return FAILURE — never a saveable.
 *
 * This is the security-by-construction guarantee: safety is the compiler gate,
 * not model trust. No matter how many times a (bad / adversarial / broken) model
 * insists, an insecure or ill-typed artifact is NEVER accepted, NEVER returned as
 * `source`, NEVER marked saveable.
 *
 * Asserts:
 *   1. result.accepted === false (the loop refused);
 *   2. result.source === null (no failing artifact is returned dressed up);
 *   3. it ran the full budget of rounds (maxRounds);
 *   4. EVERY round was non-saveable and carried the secure-form error.
 *
 * Real compiler, real fixture. Exit 0 = passed.
 */
'use strict';

const path = require('path');
const { runGenerationLoop } = require('../gen-loop.js');

const FIXTURE = path.resolve(__dirname, 'fixture');

let ok = true;
function check(name, cond, detail) {
  if (!cond) ok = false;
  console.log(`  [${cond ? 'PASS' : 'FAIL'}] ${name}${cond ? '' : '  <- ' + (detail || '')}`);
}

// An UNSECURED mutating form — a hard `secure-form` build Error, every time.
const ALWAYS_BROKEN = `@page "/new"
@layout MainLayout
@title "New note"

<form @submit="create">
  <InputText name="title" bind="@title" required />
  <TextArea name="body" bind="@body" required />
  <Button kind="primary" type="submit">Add</Button>
</form>

@code {
  var title : Text = "";
  var body : Text = "";
  func create(ctx : Context) : async () {
    ignore Notes.add(title, body);
    title := ""; body := "";
  };
}
`;

let calls = 0;
async function alwaysBrokenGenerator(context) {
  calls++;
  // It IGNORES the fed-back diagnostics and re-emits the insecure form: the
  // worst case for the gate. The gate must still hold.
  void context;
  return ALWAYS_BROKEN;
}

(async () => {
  const MAX = 4;
  const result = await runGenerationLoop({
    prompt: 'Add a note form.',
    projectDir: FIXTURE,
    page: 'src/Pages/Generated.mview',
    generator: alwaysBrokenGenerator,
    maxRounds: MAX
  });

  check('loop did NOT accept (gate refused)', result.accepted === false, result.reason);
  check('no source returned (no failing artifact dressed up as saveable)', result.source === null, String(result.source));
  check('budget exhausted (ran all rounds)', result.rounds.length === MAX, `rounds=${result.rounds.length}`);
  check('generator was actually called each round', calls === MAX, `calls=${calls}`);
  check(
    'EVERY round was non-saveable',
    result.rounds.every((r) => r.saveable === false),
    JSON.stringify(result.rounds.map((r) => r.saveable))
  );
  check(
    'EVERY round carried the secure-form error',
    result.rounds.every((r) => r.errors.some((e) => e.rule === 'secure-form')),
    JSON.stringify(result.rounds.map((r) => r.errors.map((e) => e.rule)))
  );

  console.log('');
  if (!ok) {
    console.error('FAIL: the gate let a broken artifact through, or accepted on budget exhaustion.');
    process.exit(1);
  }
  console.log(`OK: always-broken generator exhausted ${MAX} rounds and the loop returned FAILURE — the gate was NEVER bypassed.`);
  process.exit(0);
})().catch((e) => {
  console.error('ERROR:', (e && e.stack) || e);
  process.exit(1);
});
