#!/usr/bin/env node
/*
 * diagnostics-server.js — the MotokoStudio DESIGN-TIME diagnostics bridge.
 *
 * This is an OFF-canister, design-time helper (it never runs in a canister). The
 * studio editor (apps/studio/assets/mview-editor.js) POSTs the .mview buffer
 * here; this process runs the unbypassable compiler over it and returns the R2
 * machine-readable diagnostics so the editor can draw inline squiggles + a
 * gutter, plus the save-gate verdict (the same lint+check gate validate.sh
 * enforces).
 *
 * It is intentionally tiny and dependency-free (Node core only). It does NOT
 * deploy or touch a replica; it just shells out to:
 *     motoview lint  --json <tmpProject>
 *     motoview check --json <tmpProject>
 * over a throwaway copy of apps/studio with the edited page substituted in, then
 * merges the two JSON arrays and computes `saveable = no errors from either`.
 *
 * Usage:
 *     node tools/studio/diagnostics-server.js [--port 8731] [--project apps/studio]
 *     MOTOVIEW=/path/to/motoview node tools/studio/diagnostics-server.js
 *
 * Security: binds to 127.0.0.1 only. This is a localhost dev tool; do not expose
 * it. CORS is permissive so the local studio canister page can call it during
 * development.
 */
'use strict';

const http = require('http');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');

const REPO_ROOT = path.resolve(__dirname, '..', '..');

function arg(name, def) {
  const i = process.argv.indexOf(name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : def;
}

const PORT = parseInt(arg('--port', process.env.MV_STUDIO_DIAG_PORT || '8731'), 10);
const HOST = '127.0.0.1';
const BASE_PROJECT = path.resolve(arg('--project', path.join(REPO_ROOT, 'apps', 'studio')));
// Which page in the project the editor is editing (the buffer replaces it).
const TARGET_PAGE = arg('--page', 'src/Pages/Design.mview');

function resolveMotoview() {
  if (process.env.MOTOVIEW) return process.env.MOTOVIEW;
  const rel = path.join(REPO_ROOT, 'compiler', 'target', 'release', 'motoview');
  if (fs.existsSync(rel)) return rel;
  return 'motoview'; // fall back to PATH
}
const BIN = resolveMotoview();

/** Recursively copy a directory, skipping build artefacts. */
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

/** Run `motoview <sub> --json <dir>` and parse the JSON array it prints. */
function runJson(sub, dir) {
  try {
    const out = execFileSync(BIN, [sub, '--json', dir], {
      cwd: REPO_ROOT,
      encoding: 'utf8',
      maxBuffer: 16 * 1024 * 1024
    });
    return { ok: true, diags: parseDiagArray(out) };
  } catch (e) {
    // A nonzero exit (e.g. a hard lint error) STILL prints the JSON on stdout.
    const out = (e.stdout || '').toString();
    return { ok: false, diags: parseDiagArray(out), exit: e.status };
  }
}

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

/**
 * Lint + check a .mview buffer by staging it into a temp copy of the project.
 * Returns merged diagnostics and the save-gate verdict.
 */
function diagnose(source) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'mvstudio-'));
  try {
    copyDir(BASE_PROJECT, tmp);
    const pagePath = path.join(tmp, TARGET_PAGE);
    fs.mkdirSync(path.dirname(pagePath), { recursive: true });
    fs.writeFileSync(pagePath, source, 'utf8');

    const lint = runJson('lint', tmp);
    const check = runJson('check', tmp);

    // Re-root file paths from the temp dir back onto the edited page name so the
    // editor matches them to the buffer.
    const all = [...lint.diags, ...check.diags];
    const hasError = all.some((d) => d.severity === 'error');
    return {
      diagnostics: all,
      saveable: !hasError && lint.ok,
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

function sendJson(res, status, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(status, {
    'content-type': 'application/json',
    'access-control-allow-origin': '*',
    'access-control-allow-headers': 'content-type',
    'access-control-allow-methods': 'POST, OPTIONS'
  });
  res.end(body);
}

const server = http.createServer((req, res) => {
  if (req.method === 'OPTIONS') {
    sendJson(res, 204, {});
    return;
  }
  if (req.method === 'GET' && req.url === '/health') {
    sendJson(res, 200, { ok: true, bin: BIN, project: BASE_PROJECT, page: TARGET_PAGE });
    return;
  }
  if (req.method !== 'POST') {
    sendJson(res, 405, { error: 'POST a {source} body to /diagnostics' });
    return;
  }
  let body = '';
  req.on('data', (c) => {
    body += c;
    if (body.length > 8 * 1024 * 1024) req.destroy();
  });
  req.on('end', () => {
    let source = '';
    try {
      source = JSON.parse(body).source || '';
    } catch {
      sendJson(res, 400, { error: 'invalid JSON body' });
      return;
    }
    try {
      const result = diagnose(source);
      sendJson(res, 200, result);
    } catch (e) {
      sendJson(res, 500, { error: String((e && e.message) || e) });
    }
  });
});

// Allow `require()` from a test without starting the listener.
if (require.main === module) {
  server.listen(PORT, HOST, () => {
    /* eslint-disable no-console */
    console.log(`MotokoStudio diagnostics bridge listening on http://${HOST}:${PORT}`);
    console.log(`  compiler: ${BIN}`);
    console.log(`  project:  ${BASE_PROJECT}`);
    console.log(`  page:     ${TARGET_PAGE}`);
    console.log(`  POST {source} -> /diagnostics  ·  GET /health`);
  });
}

module.exports = { diagnose, parseDiagArray, server };
