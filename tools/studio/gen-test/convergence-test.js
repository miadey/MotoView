#!/usr/bin/env node
/*
 * convergence-test.js — R5 generate/validate/REPAIR loop CONVERGENCE test.
 *
 * Proves the loop drives a MOCK generator from a FAILING artifact to a GREEN
 * (saveable) one using the compiler diagnostics fed back between rounds:
 *
 *   round 1: the mock returns a mutating <form @submit> WITHOUT `secure`
 *            -> the gate (motoview lint + check over the fixture) emits a
 *               `secure-form` ERROR; saveable=false; the candidate is REJECTED.
 *   round 2: the mock — having been HANDED the fed-back secure-form diagnostic —
 *            returns the SAME form WITH `secure` (+ server-side validate)
 *            -> the gate passes; saveable=true; the loop accepts it.
 *
 * Asserts:
 *   1. the loop CONVERGES to an accepted (zero-error) result in <= 2 rounds;
 *   2. round 1's broken artifact was NOT accepted (the gate rejected it);
 *   3. the round-2 mock actually SAW the secure-form diagnostic (the repair was
 *      driven by fed-back diagnostics, not luck);
 *   4. the returned source is the saveable one and contains `secure`.
 *
 * Real compiler, real fixture project (tools/studio/gen-test/fixture). No LLM:
 * the generator is a deterministic mock injected here. Exit 0 = passed.
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

// The two candidates the mock emits. They differ ONLY by the `secure` attribute
// (+ a server-side validate block in the fixed one) — exactly the secure-form
// rule the gate enforces.
const BROKEN = `@page "/new"
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

const FIXED = `@page "/new"
@layout MainLayout
@authorize
@title "New note"

<form @submit="create" secure>
  <ValidationSummary />
  <InputText name="title" bind="@title" required />
  <TextArea name="body" bind="@body" required />
  <Button kind="primary" type="submit">Add</Button>
</form>

@code {
  var title : Text = "";
  var body : Text = "";
  func create(ctx : Context) : async () {
    validate {
      title required "Give your note a title.";
      body required "Write something.";
    };
    ignore Notes.add(title, body);
    title := ""; body := "";
    toast("Note added!");
  };
}
`;

// Record what each round's context carried so we can prove the repair was
// diagnostic-driven.
const seenContexts = [];

/**
 * The MOCK generator: broken on round 1, and — ONLY IF handed back a secure-form
 * error — fixed on round 2. If round 2 did NOT receive the diagnostic, it would
 * return BROKEN again and the loop would not converge: so convergence itself
 * proves the feedback wiring works.
 */
async function mockGenerator(context) {
  seenContexts.push(context);
  const sawSecureForm =
    Array.isArray(context.previousErrors) &&
    context.previousErrors.some((d) => d.rule === 'secure-form');
  if (context.round === 1) return BROKEN;
  return sawSecureForm ? FIXED : BROKEN;
}

(async () => {
  const result = await runGenerationLoop({
    prompt: 'A page to add a note: title + body. Signed-in only.',
    projectDir: FIXTURE,
    page: 'src/Pages/Generated.mview',
    generator: mockGenerator,
    maxRounds: 4
  });

  check('loop converged to an accepted result', result.accepted === true, result.reason);
  check('converged in <= 2 rounds', result.rounds.length <= 2, `rounds=${result.rounds.length}`);

  const round1 = result.rounds[0];
  check('round 1 ran the gate', !!round1);
  check('round 1 broken artifact was NOT saveable', round1 && round1.saveable === false, JSON.stringify(round1 && round1.saveable));
  check(
    'round 1 carried the secure-form ERROR',
    !!round1 && round1.errors.some((e) => e.rule === 'secure-form'),
    round1 && JSON.stringify(round1.errors.map((e) => e.rule))
  );

  // The round-1 broken artifact must NOT be what got returned.
  check('returned source is NOT the broken artifact', result.source !== BROKEN);
  check('returned source contains `secure`', typeof result.source === 'string' && /\bsecure\b/.test(result.source));

  // Round 2 must have SEEN the fed-back secure-form diagnostic.
  const ctx2 = seenContexts[1];
  check('round 2 received the previous diagnostics (repair was diagnostic-driven)',
    !!ctx2 && Array.isArray(ctx2.previousErrors) && ctx2.previousErrors.some((d) => d.rule === 'secure-form'),
    ctx2 && JSON.stringify(ctx2.previousErrors));
  check('round 2 context included the palette (Notes service)',
    !!ctx2 && Array.isArray(ctx2.services) && ctx2.services.some((s) => s.name === 'Notes'));

  // The accepted round (the last one) must be saveable.
  const lastRound = result.rounds[result.rounds.length - 1];
  check('final round is saveable (gate passed, 0 errors)', !!lastRound && lastRound.saveable === true);

  console.log('');
  if (!ok) {
    console.error('FAIL: convergence assertions failed.');
    process.exit(1);
  }
  console.log(`OK: loop converged to a SAVEABLE artifact in ${result.rounds.length} round(s); round-1 broken artifact was rejected by the real compiler gate.`);
  process.exit(0);
})().catch((e) => {
  console.error('ERROR:', (e && e.stack) || e);
  process.exit(1);
});
