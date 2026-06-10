#!/usr/bin/env node
/*
 * gen-loop.js — the MotokoStudio AI generate / validate / repair LOOP (R5).
 *
 * This is the heart of the studio's authoring loop, and its whole point is one
 * invariant:
 *
 *     THE MODEL CANNOT SHIP AN APP THAT FAILS TYPE-CHECK OR THE SECURITY LINT.
 *
 * Safety is the unbypassable COMPILER gate, not model trust. The loop:
 *
 *   1. builds a generation CONTEXT = prompt + the project's real service PALETTE
 *      (signatures.js) + (on a retry) the compiler diagnostics from the previous
 *      attempt;
 *   2. calls a pluggable GENERATOR(context) -> one candidate `.mview` text;
 *   3. STAGES that candidate into a throwaway copy of the project and runs the
 *      same gate validate.sh enforces — `motoview lint --json` + `motoview check
 *      --json` — over it;
 *   4. if there are ANY error-severity diagnostics, the candidate is REJECTED,
 *      its diagnostics are appended to the context, and the loop RETRIES (up to a
 *      budget);
 *   5. a candidate is RETURNED as `accepted` ONLY when BOTH gates pass with zero
 *      errors (saveable). A failing candidate is NEVER returned as accepted.
 *
 * The GENERATOR is pluggable:
 *   - the REAL one (httpGenerator) POSTs to $MOTOVIEW_STUDIO_LLM_ENDPOINT. It is
 *     an EXTERNAL, OFF-canister, design-time call. It is FLAGGED and is NOT
 *     exercised by the tests (no endpoint, no key in CI).
 *   - a MOCK generator is injected by the tests to drive the loop deterministically.
 *
 * Off-canister & design-time: nothing here runs in a canister. See generate.md
 * (the contract), validate.sh (the gate), README.md (the loop diagram).
 *
 * Usage (programmatic):
 *   const { runGenerationLoop, httpGenerator } = require('./gen-loop.js');
 *   const r = await runGenerationLoop({ prompt, projectDir, page, generator, maxRounds });
 *   // r.accepted (bool), r.source (saveable .mview or null), r.rounds[…], r.reason
 */
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');
const { extractProject, paletteText } = require('./signatures.js');

const REPO_ROOT = path.resolve(__dirname, '..', '..');

/** Resolve the compiler binary: $MOTOVIEW, else repo release build, else PATH. */
function resolveMotoview() {
  if (process.env.MOTOVIEW) return process.env.MOTOVIEW;
  const rel = path.join(REPO_ROOT, 'compiler', 'target', 'release', 'motoview');
  if (fs.existsSync(rel)) return rel;
  return 'motoview';
}

/** Recursively copy a directory, skipping build artefacts. (Same skip-set as the bridge.) */
function copyDir(src, dst) {
  fs.mkdirSync(dst, { recursive: true });
  for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
    if (['.mvbuild', 'node_modules', '.git', '.dfx'].includes(entry.name)) continue;
    const s = path.join(src, entry.name);
    const d = path.join(dst, entry.name);
    if (entry.isDirectory()) copyDir(s, d);
    else fs.copyFileSync(s, d);
  }
}

/** Pull the JSON array out of compiler stdout (it may print log lines around it). */
function parseDiagArray(out) {
  const start = out.indexOf('[');
  const end = out.lastIndexOf(']');
  if (start < 0 || end < start) return [];
  try {
    const arr = JSON.parse(out.slice(start, end + 1));
    return Array.isArray(arr) ? arr : [];
  } catch {
    return [];
  }
}

/** Run `motoview <sub> --json <dir>`; a nonzero exit still carries JSON on stdout. */
function runJson(bin, sub, dir) {
  try {
    // stdio: capture stdout (the JSON array), pipe stdin, and SILENCE stderr —
    // the compiler also prints a human diagnostic dump to stderr which would
    // otherwise clutter the loop/test output. The machine-readable JSON is on
    // stdout regardless of exit code.
    const out = execFileSync(bin, [sub, '--json', dir], {
      cwd: REPO_ROOT,
      encoding: 'utf8',
      maxBuffer: 16 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'ignore']
    });
    return { ok: true, diags: parseDiagArray(out) };
  } catch (e) {
    const out = (e.stdout || '').toString();
    return { ok: false, diags: parseDiagArray(out), exit: e.status };
  }
}

/**
 * Stage a candidate `.mview` into a temp copy of `projectDir` at `page`, then run
 * the SAME gate validate.sh enforces (lint + check), via the --json path so we
 * get machine-readable diagnostics to feed back. Returns the gate verdict.
 *
 * `saveable` mirrors validate.sh / the bridge: no error-severity diagnostics from
 * EITHER pass, and lint did not hard-fail. Warnings do NOT block (they never
 * block the gate — only errors do).
 */
function gateCandidate(source, projectDir, page, bin) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'mvstudio-gen-'));
  try {
    copyDir(projectDir, tmp);
    const pagePath = path.join(tmp, page);
    fs.mkdirSync(path.dirname(pagePath), { recursive: true });
    fs.writeFileSync(pagePath, source, 'utf8');

    const lint = runJson(bin, 'lint', tmp);
    const check = runJson(bin, 'check', tmp);
    const diagnostics = [...lint.diags, ...check.diags];
    const errors = diagnostics.filter((d) => d.severity === 'error');
    const saveable = errors.length === 0 && lint.ok;
    return {
      diagnostics,
      errors,
      saveable,
      lintPassed: lint.ok,
      checkPassed: check.ok,
      lintExit: lint.exit ?? 0,
      checkExit: check.exit ?? 0
    };
  } finally {
    try {
      fs.rmSync(tmp, { recursive: true, force: true });
    } catch {
      /* best effort */
    }
  }
}

/** Render diagnostics as the compact feedback block appended to a retry context. */
function diagnosticsText(diags) {
  if (!diags.length) return '(no diagnostics)';
  return diags
    .map((d) => {
      const pos = d.line ? ` (line ${d.line}:${d.col})` : '';
      return `[${d.severity}] ${d.rule}${pos}: ${d.message}`;
    })
    .join('\n');
}

/**
 * Build the generation context object handed to the generator. On a retry,
 * `previousErrors` (the compiler diagnostics that rejected the last candidate)
 * and `previousSource` are included so the model can REPAIR rather than restart.
 */
function buildContext({ prompt, palette, round, previousSource, previousErrors }) {
  return {
    prompt,
    // The binding palette as both structured services and a ready-to-embed text
    // block (generate.md's `services` input + a prompt-friendly rendering).
    services: palette.services,
    paletteText: paletteText(palette),
    securityModules: ['Security', 'Roles', 'EncStore', 'VetKeys', 'Audit', 'ChainKey', 'WalletAuth', 'CertV2'],
    round,
    // Repair feedback (null on the first round).
    previousSource: previousSource || null,
    previousErrors: previousErrors || null,
    previousErrorsText: previousErrors ? diagnosticsText(previousErrors) : null
  };
}

/**
 * THE LOOP. Generate → gate → (on errors) feed back → retry, until a saveable
 * candidate is accepted or the budget is exhausted.
 *
 * @param {object} opts
 * @param {string}   opts.prompt        the human design request
 * @param {string}   opts.projectDir    the MotoView project (for the palette + the gate)
 * @param {string}  [opts.page]         where the candidate is staged (default src/Pages/Generated.mview)
 * @param {function} opts.generator     async (context) -> candidate .mview text (REQUIRED; pluggable)
 * @param {number}  [opts.maxRounds]    retry budget (default 4)
 * @param {string}  [opts.bin]          compiler binary (default resolveMotoview())
 * @param {function}[opts.onRound]      optional progress callback({round, saveable, errors})
 *
 * @returns {Promise<{accepted:boolean, source:?string, rounds:Array, reason:string}>}
 *   accepted=true  -> `source` is a saveable (zero-error) .mview; the gate passed.
 *   accepted=false -> budget exhausted; `source` is null. The gate was NEVER bypassed.
 */
async function runGenerationLoop(opts) {
  const {
    prompt,
    projectDir,
    page = 'src/Pages/Generated.mview',
    generator,
    maxRounds = 4,
    bin = resolveMotoview(),
    onRound
  } = opts;

  if (typeof generator !== 'function') {
    throw new Error('runGenerationLoop: a `generator(context)` function is required (pluggable).');
  }
  if (!projectDir || !fs.existsSync(projectDir)) {
    throw new Error(`runGenerationLoop: projectDir not found: ${projectDir}`);
  }

  const palette = extractProject(projectDir);
  const rounds = [];
  let previousSource = null;
  let previousErrors = null;

  for (let round = 1; round <= maxRounds; round++) {
    const context = buildContext({ prompt, palette, round, previousSource, previousErrors });
    const candidate = await generator(context);

    if (typeof candidate !== 'string' || candidate.trim() === '') {
      // A generator that returns nothing usable is treated as a failed round.
      rounds.push({ round, saveable: false, errors: [], note: 'generator returned no candidate' });
      previousSource = null;
      previousErrors = [{ severity: 'error', rule: 'empty-candidate', message: 'generator returned no candidate' }];
      if (onRound) onRound({ round, saveable: false, errors: previousErrors });
      continue;
    }

    const verdict = gateCandidate(candidate, projectDir, page, bin);
    rounds.push({
      round,
      saveable: verdict.saveable,
      errors: verdict.errors,
      diagnostics: verdict.diagnostics,
      lintPassed: verdict.lintPassed,
      checkPassed: verdict.checkPassed,
      candidateLength: candidate.length
    });
    if (onRound) onRound({ round, saveable: verdict.saveable, errors: verdict.errors });

    if (verdict.saveable) {
      // GATE PASSED — and ONLY now is a candidate accepted/returned.
      return {
        accepted: true,
        source: candidate,
        rounds,
        reason: `saveable after ${round} round(s): lint + check clean (0 errors)`
      };
    }

    // REJECTED. Feed the compiler's errors back into the next round's context.
    previousSource = candidate;
    previousErrors = verdict.errors.length ? verdict.errors : verdict.diagnostics;
  }

  // Budget exhausted with no saveable candidate. The gate was NEVER bypassed:
  // we return FAILURE, never a failing artifact dressed up as accepted.
  return {
    accepted: false,
    source: null,
    rounds,
    reason: `exhausted ${maxRounds} round(s) without a saveable candidate; gate refused every attempt`
  };
}

/**
 * The REAL generator — an EXTERNAL, OFF-canister, design-time HTTP call. This is
 * a thin seam, FLAGGED on $MOTOVIEW_STUDIO_LLM_ENDPOINT. It is NOT exercised by
 * the tests (there is no endpoint and no key in CI), and it embeds NO key or
 * model: wiring a real endpoint is a deployment choice. Whatever it returns is
 * still subject to the unbypassable gate above.
 */
function httpGenerator({ endpoint, apiKey, system } = {}) {
  const url = endpoint || process.env.MOTOVIEW_STUDIO_LLM_ENDPOINT;
  const key = apiKey || process.env.MOTOVIEW_STUDIO_LLM_KEY;
  return async function generate(context) {
    if (!url) {
      throw new Error(
        'httpGenerator: no LLM endpoint. Set $MOTOVIEW_STUDIO_LLM_ENDPOINT (external, ' +
          'off-canister). This seam is intentionally not wired in tests.'
      );
    }
    // Node 18+/22 has global fetch. The body mirrors generate.md's contract.
    const res = await fetch(url, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        ...(key ? { authorization: `Bearer ${key}` } : {})
      },
      body: JSON.stringify({
        system: system || 'You generate ONE complete, secure-by-construction .mview artifact. See generate.md.',
        input: {
          prompt: context.prompt,
          services: context.services,
          securityModules: context.securityModules,
          currentSource: context.previousSource,
          // On a retry we hand the model the COMPILER's rejection so it repairs.
          compilerErrors: context.previousErrorsText
        }
      })
    });
    if (!res.ok) throw new Error(`httpGenerator: endpoint returned ${res.status}`);
    const data = await res.json();
    return data.mview || data.source || '';
  };
}

// ── CLI (uses the real, flagged endpoint; refuses without it) ──────────────────
if (require.main === module) {
  const args = process.argv.slice(2);
  const arg = (n, d) => {
    const i = args.indexOf(n);
    return i >= 0 && args[i + 1] ? args[i + 1] : d;
  };
  const projectDir = path.resolve(arg('--project', path.join(REPO_ROOT, 'apps', 'studio')));
  const page = arg('--page', 'src/Pages/Generated.mview');
  const prompt = arg('--prompt', '');
  const maxRounds = parseInt(arg('--max-rounds', '4'), 10);
  if (!prompt) {
    console.error('usage: node tools/studio/gen-loop.js --prompt "..." [--project DIR] [--page P] [--max-rounds N]');
    console.error('  requires $MOTOVIEW_STUDIO_LLM_ENDPOINT (external, off-canister LLM). Not faked.');
    process.exit(2);
  }
  runGenerationLoop({ prompt, projectDir, page, generator: httpGenerator(), maxRounds })
    .then((r) => {
      console.log(r.reason);
      if (r.accepted) {
        process.stdout.write(r.source);
        process.exit(0);
      } else {
        console.error('REFUSED: no saveable candidate. The gate was not bypassed.');
        process.exit(1);
      }
    })
    .catch((e) => {
      console.error(String((e && e.message) || e));
      process.exit(1);
    });
}

module.exports = {
  runGenerationLoop,
  gateCandidate,
  buildContext,
  diagnosticsText,
  httpGenerator,
  parseDiagArray
};
