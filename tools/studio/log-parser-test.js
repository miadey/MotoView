#!/usr/bin/env node
'use strict';
/*
 * log-parser-test.js — R7 observability: verify the structured log parser turns
 * the `--instrument` Debug.print stream into the right structured events.
 *
 * The sample stream is the EXACT format the compiler emits in the generated
 * `mvDispatch` (compiler/src/codegen.rs), including a `dfx canister logs`-style
 * line prefix the parser must tolerate, a non-MV line it must skip, and a
 * costless line (no perf counter) to exercise the null-cost path.
 *
 * Exit 0 = passed.
 */

const obs = require('./observability.js');

let ok = true;
function check(name, cond, detail) {
  if (!cond) ok = false;
  console.log(`  [${cond ? 'PASS' : 'FAIL'}] ${name}${cond ? '' : '  <- ' + (detail || '')}`);
}

// A realistic stream: dfx prefixes the seq/timestamp; an unrelated print sits in
// the middle; the last line has no costInstr field.
const PRINCIPAL = 'rdmx6-jaaaa-aaaaa-aaadq-cai';
const stream = [
  `[0. 2026-06-10T00:00:01Z]: MV|dispatch|page=Counter|handler=increment|event=increment|caller=${PRINCIPAL}|lastBatch=b1|costInstr=42`,
  `some unrelated canister log line — must be ignored`,
  `[1. 2026-06-10T00:00:02Z]: MV|dispatch|page=Counter|handler=decrement|event=decrement|caller=${PRINCIPAL}|lastBatch=b2|costInstr=37`,
  `MV|dispatch|page=Cart|handler=checkout|event=checkout|caller=2vxsx-fae|lastBatch=`,
].join('\n');

const events = obs.parseLogStream(stream);

check('parsed 3 dispatch events (skipped the unrelated line)', events.length === 3, `got ${events.length}`);

const e0 = events[0] || {};
check('event[0].handler == increment', e0.handler === 'increment', e0.handler);
check('event[0].event == increment', e0.event === 'increment', e0.event);
check('event[0].page == Counter', e0.page === 'Counter', e0.page);
check('event[0].caller == principal', e0.caller === PRINCIPAL, e0.caller);
check('event[0].lastBatch == b1', e0.lastBatch === 'b1', e0.lastBatch);
check('event[0].costInstr == 42 (number)', e0.costInstr === 42, String(e0.costInstr));
check('event[0].prefix captured the dfx prefix', /^\[0\./.test(e0.prefix || ''), e0.prefix);

const e1 = events[1] || {};
check('event[1].handler == decrement', e1.handler === 'decrement', e1.handler);
check('event[1].costInstr == 37', e1.costInstr === 37, String(e1.costInstr));

const e2 = events[2] || {};
check('event[2].page == Cart', e2.page === 'Cart', e2.page);
check('event[2].handler == checkout', e2.handler === 'checkout', e2.handler);
check('event[2].caller == anonymous-ish principal', e2.caller === '2vxsx-fae', e2.caller);
check('event[2].lastBatch is empty string', e2.lastBatch === '', JSON.stringify(e2.lastBatch));
check('event[2].costInstr is null (no perf counter)', e2.costInstr === null, String(e2.costInstr));

// summary roll-up
const summary = obs.summarizeEvents(events);
check('summary has 3 distinct handlers', summary.length === 3, `got ${summary.length}`);
const inc = summary.find((s) => s.handler === 'increment');
check('increment summary count == 1', inc && inc.count === 1, inc && String(inc.count));
check('increment summary avgCost == 42', inc && inc.avgCost === 42, inc && String(inc.avgCost));

// the timeline renderer is headless-checkable for structure
const html = obs.renderTimelineHtml(events);
check('timeline html has 3 rows', (html.match(/mv-tl-row/g) || []).length === 3);
check('timeline html escapes/embeds the handler', html.includes('increment'));

console.log();
if (ok) {
  console.log('LOG PARSER TEST: PASSED');
  process.exit(0);
} else {
  console.log('LOG PARSER TEST: FAILED');
  process.exit(1);
}
