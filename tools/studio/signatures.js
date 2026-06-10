#!/usr/bin/env node
/*
 * signatures.js — the AI's backend-binding PALETTE extractor (R5).
 *
 * MotokoStudio's generation step must be "always bound to the backend": the
 * model may only call service functions that actually EXIST, with the types they
 * actually have. This script scans a project's `src/Services/*.mo` files and
 * extracts the PUBLIC surface of each stateful service — the `public func`,
 * `public type` and `public let` declarations exported from the service's
 * `public class <Name>()`. That structured list is the palette handed to the
 * generator (see generate.md's `services` input).
 *
 * This is a deliberately SCRIPT-BASED parse, NOT a compiler subcommand:
 * `compiler/` is owned by a concurrent slice, so we do not add a `motoview`
 * subcommand here. The parser is a focused, line-oriented scanner — it does not
 * try to be a full Motoko parser; it recognizes the conventional service shape
 * (a `public class Name()` body containing `public func/type/let` decls), which
 * is exactly the convention every MotoView stateful service follows.
 *
 * Usage:
 *   node tools/studio/signatures.js <project-dir>            # pretty
 *   node tools/studio/signatures.js <project-dir> --json     # machine JSON
 *
 * Output (per the generate.md contract): a list of services, each with its name
 * and a flat list of { name, kind, signature } entries (kind ∈ func|type|let).
 *
 * Also `require()`-able:  const { extractProject } = require('./signatures.js');
 */
'use strict';

const fs = require('fs');
const path = require('path');

/**
 * Strip a line-comment (`// …`) tail from a single line WITHOUT eating a `//`
 * that lives inside a string literal. Service signatures don't usually carry
 * inline comments, but the trailing `// #hex` / `// derived` notes in the real
 * Forum/Studio services do — so we trim them to keep signatures clean.
 */
function stripLineComment(line) {
  let inStr = false;
  for (let i = 0; i < line.length - 1; i++) {
    const c = line[i];
    if (c === '"' && line[i - 1] !== '\\') inStr = !inStr;
    if (!inStr && c === '/' && line[i + 1] === '/') return line.slice(0, i);
  }
  return line;
}

/** Collapse runs of whitespace (incl. newlines) into single spaces and trim. */
function normalizeWs(s) {
  return s.replace(/\s+/g, ' ').trim();
}

/**
 * Find the index just past the matching closer for the opener at `openIdx`.
 * Brackets are counted across `(){}[]<>` so a multi-line record type or a
 * multi-line param list is captured whole. String literals are skipped.
 * Returns the index of the char AFTER the matched closer, or -1 if unbalanced.
 */
// NOTE: we deliberately do NOT track `<`/`>` as brackets. In Motoko bodies they
// are comparison operators far more often than generics, and counting them
// corrupts depth tracking. Generic parameter lists on names are handled
// separately with a targeted `<[^>]*>` regex.
const OPENERS = { '(': ')', '{': '}', '[': ']' };
const CLOSERS = { ')': '(', '}': '{', ']': '[' };

/**
 * Walk a service body string from `start`, returning a "statement" — text up to
 * the `;` that terminates the declaration at depth 0 — together with the index
 * just past that `;`. Bracket depth and string literals are respected so the
 * terminating `;` is the real end of the decl, not one nested inside a record
 * type, a function param list, or (for `let`/`func` bodies) the body itself.
 */
function readDecl(body, start) {
  let depth = 0;
  let inStr = false;
  let i = start;
  for (; i < body.length; i++) {
    const c = body[i];
    const prev = body[i - 1];
    if (inStr) {
      if (c === '"' && prev !== '\\') inStr = false;
      continue;
    }
    if (c === '"') {
      inStr = true;
      continue;
    }
    if (OPENERS[c]) depth++;
    else if (CLOSERS[c]) depth--;
    else if (c === ';' && depth === 0) {
      return { text: body.slice(start, i), end: i + 1 };
    }
  }
  // Unterminated (EOF) — return what we have.
  return { text: body.slice(start, i), end: i };
}

/**
 * Extract the body of the `public class <Name>()` (or `class <Name>(...)`) in a
 * service source. Returns { name, body } or null if the file has no service
 * class (e.g. a plain `module { }` of helpers). Body excludes the outer braces.
 */
function findServiceClass(src) {
  // Match `public class Name(` — the MotoView stateful-service convention.
  const re = /public\s+class\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/g;
  const m = re.exec(src);
  if (!m) return null;
  const name = m[1];
  // Find the `{` that opens the class body (after the param list `(...)` and an
  // optional return-type annotation). Scan from the end of the match.
  let i = m.index + m[0].length;
  // skip the param list to its matching ')'
  let depth = 1;
  let inStr = false;
  for (; i < src.length && depth > 0; i++) {
    const c = src[i];
    const prev = src[i - 1];
    if (inStr) {
      if (c === '"' && prev !== '\\') inStr = false;
      continue;
    }
    if (c === '"') inStr = true;
    else if (c === '(') depth++;
    else if (c === ')') depth--;
  }
  // now find the class-body opening '{'
  const braceStart = src.indexOf('{', i);
  if (braceStart < 0) return null;
  // capture to the matching '}'
  depth = 0;
  inStr = false;
  let j = braceStart;
  for (; j < src.length; j++) {
    const c = src[j];
    const prev = src[j - 1];
    if (inStr) {
      if (c === '"' && prev !== '\\') inStr = false;
      continue;
    }
    if (c === '"') inStr = true;
    else if (c === '{') depth++;
    else if (c === '}') {
      depth--;
      if (depth === 0) {
        return { name, body: src.slice(braceStart + 1, j) };
      }
    }
  }
  return { name, body: src.slice(braceStart + 1) };
}

/**
 * Given a `func`-decl statement text (already comment-stripped, starting at
 * `func`), produce a clean one-line signature: `name(params) : ReturnType`.
 * Drops the body (`= …` / `{ … }`) — we only want the callable shape.
 */
function funcSignature(declText) {
  // declText starts with "func ". Capture name + param-list + optional `: Ret`.
  // Strip the body: everything from the FIRST top-level `{` or top-level `=`
  // that is NOT part of the return type. We do this by walking to the param
  // list close, then capturing the return type up to a `{` / `=` / end.
  const afterFunc = declText.replace(/^\s*func\s+/, '');
  const nameMatch = afterFunc.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*(<[^>]*>)?\s*\(/);
  if (!nameMatch) return { name: null, signature: normalizeWs(declText) };
  const name = nameMatch[1];
  const generics = nameMatch[2] || '';
  // find the matching close of the param list
  const parenStart = afterFunc.indexOf('(', nameMatch.index + name.length);
  let depth = 0;
  let inStr = false;
  let k = parenStart;
  for (; k < afterFunc.length; k++) {
    const c = afterFunc[k];
    const prev = afterFunc[k - 1];
    if (inStr) {
      if (c === '"' && prev !== '\\') inStr = false;
      continue;
    }
    if (c === '"') inStr = true;
    else if (c === '(') depth++;
    else if (c === ')') {
      depth--;
      if (depth === 0) {
        k++;
        break;
      }
    }
  }
  // includes the surrounding (); normalize whitespace and drop a trailing
  // comma before the close paren (Motoko allows `( a, b, )`).
  const params = normalizeWs(afterFunc.slice(parenStart, k)).replace(/,\s*\)/, ' )').replace(/\(\s+/, '(').replace(/\s+\)/, ')');
  // After the param list comes (optionally) `: ReturnType` then the BODY.
  // We capture ONLY the return type and drop the body. The wrinkle: a return
  // type can itself be a record/variant written with braces (e.g.
  //   : { #less; #equal; #greater }
  // ), so the FIRST `{` is not necessarily the body. Rule:
  //   - skip whitespace; if the next token is not `:`, there is no return type
  //     (unit-returning `func f(...) { body }`).
  //   - otherwise the return type is ONE type-expression after the `:`: a
  //     brace-/bracket-balanced group if it starts with `{`/`[`/`(`, else a run
  //     of chars until the body opener (`{` / `=` / `;`) at depth 0.
  const rest = afterFunc.slice(k);
  const colonM = rest.match(/^\s*:\s*/);
  let retClean = '';
  if (colonM) {
    const tStart = colonM[0].length;
    let d2 = 0;
    let inStr2 = false;
    let seenContent = false;
    let p = tStart;
    let ret = '';
    for (; p < rest.length; p++) {
      const c = rest[p];
      const prev = rest[p - 1];
      if (inStr2) {
        if (c === '"' && prev !== '\\') inStr2 = false;
        ret += c;
        continue;
      }
      if (c === '"') {
        inStr2 = true;
        ret += c;
        seenContent = true;
        continue;
      }
      // At depth 0, a `{` ends the type ONLY if we've already captured the
      // type-expression (i.e. it's the body). A leading `{` (record/variant
      // return type) opens the type and is balanced via depth.
      if (d2 === 0 && seenContent && (c === '{' || c === '=' || c === ';')) break;
      if (OPENERS[c]) {
        d2++;
        // a leading brace group IS the return type; once it closes we're done
      } else if (CLOSERS[c]) {
        d2--;
      }
      if (!/\s/.test(c)) seenContent = true;
      ret += c;
      // if a leading brace/bracket group just closed at depth 0, the type ends
      if (d2 === 0 && (c === '}' || c === ']' || c === ')') && ret.trim().length) {
        // only treat as end-of-type if the type STARTED with an opener
        const firstNonWs = ret.trim()[0];
        if (firstNonWs === '{' || firstNonWs === '[' || firstNonWs === '(') {
          p++;
          break;
        }
      }
    }
    retClean = normalizeWs(ret);
  }
  const signature = retClean
    ? normalizeWs(`${name}${generics}${params} : ${retClean}`)
    : normalizeWs(`${name}${generics}${params}`);
  return { name, signature };
}

/** Signature for a `type` decl: `Name = <rhs>` (rhs normalized). */
function typeSignature(declText) {
  const after = declText.replace(/^\s*type\s+/, '');
  const m = after.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*(<[^>]*>)?\s*=/);
  if (!m) return { name: null, signature: normalizeWs(declText) };
  const name = m[1];
  return { name, signature: normalizeWs(`${name}${m[2] || ''} = ${normalizeWs(after.slice(after.indexOf('=') + 1))}`) };
}

/** Signature for a `let` decl: `Name : Type` (value/body dropped). */
function letSignature(declText) {
  const after = declText.replace(/^\s*let\s+/, '');
  const m = after.match(/^([A-Za-z_][A-Za-z0-9_]*)/);
  if (!m) return { name: null, signature: normalizeWs(declText) };
  const name = m[1];
  // type annotation between name and `=`
  const eq = after.indexOf('=');
  const head = eq >= 0 ? after.slice(0, eq) : after;
  const tm = head.match(/:\s*([\s\S]+)/);
  const ty = tm ? normalizeWs(tm[1]) : '';
  return { name, signature: ty ? `${name} : ${ty}` : name };
}

/**
 * Extract all PUBLIC declarations from a service-class body. Skips the
 * upgrade-persistence plumbing (`mvStableSave`/`mvStableLoad`) — those are
 * compiler-wired, not part of the page-facing palette.
 */
const PERSISTENCE = new Set(['mvStableSave', 'mvStableLoad']);

function extractDecls(body) {
  const decls = [];
  // Scan for `public func|type|let` at any indentation. We anchor on the
  // keyword sequence, then read the whole decl with bracket-aware readDecl.
  const re = /\bpublic\s+(func|type|let)\b/g;
  let m;
  while ((m = re.exec(body)) !== null) {
    const kind = m[1];
    // The decl text starts at the keyword (func/type/let), right after `public`.
    const declStart = m.index + m[0].length - kind.length;
    const { text } = readDecl(body, declStart);
    const stripped = text
      .split('\n')
      .map(stripLineComment)
      .join('\n');
    let sig;
    if (kind === 'func') sig = funcSignature(stripped);
    else if (kind === 'type') sig = typeSignature(stripped);
    else sig = letSignature(stripped);
    if (!sig.name) continue;
    if (kind === 'func' && PERSISTENCE.has(sig.name)) continue;
    decls.push({ name: sig.name, kind, signature: sig.signature });
  }
  return decls;
}

/** Extract the palette for one Service .mo file. Returns {name, decls} or null. */
function extractFile(filePath) {
  const src = fs.readFileSync(filePath, 'utf8');
  const cls = findServiceClass(src);
  if (!cls) return null;
  const decls = extractDecls(cls.body);
  return { name: cls.name, file: path.basename(filePath), decls };
}

/**
 * Extract the full palette for a project: every src/Services/*.mo with a service
 * class. Returns a list of { name, file, methods, types, lets, decls } so callers
 * can use either the flat `decls` (the {name,kind,signature} list this slice's
 * contract specifies) or the grouped form generate.md's input shows.
 */
function extractProject(projectDir) {
  const servicesDir = path.join(projectDir, 'src', 'Services');
  if (!fs.existsSync(servicesDir)) return { services: [] };
  const files = fs
    .readdirSync(servicesDir)
    .filter((f) => f.endsWith('.mo'))
    .sort();
  const services = [];
  for (const f of files) {
    const res = extractFile(path.join(servicesDir, f));
    if (!res) continue;
    const methods = res.decls.filter((d) => d.kind === 'func').map((d) => d.signature);
    const types = res.decls.filter((d) => d.kind === 'type').map((d) => d.signature);
    const lets = res.decls.filter((d) => d.kind === 'let').map((d) => d.signature);
    services.push({ name: res.name, file: res.file, decls: res.decls, methods, types, lets });
  }
  return { services };
}

/**
 * Render the palette as the compact text block the generator prompt embeds. One
 * `service Name { ... }` per service; funcs/types/lets one per line.
 */
function paletteText(palette) {
  const out = [];
  for (const s of palette.services) {
    out.push(`service ${s.name} {`);
    for (const d of s.decls) {
      const kw = d.kind === 'func' ? 'func' : d.kind === 'type' ? 'type' : 'let';
      out.push(`  ${kw} ${d.signature}`);
    }
    out.push('}');
  }
  return out.join('\n');
}

// ── CLI ───────────────────────────────────────────────────────────────────────
if (require.main === module) {
  const args = process.argv.slice(2);
  const json = args.includes('--json');
  const dir = args.find((a) => !a.startsWith('--'));
  if (!dir) {
    console.error('usage: node tools/studio/signatures.js <project-dir> [--json]');
    process.exit(2);
  }
  const palette = extractProject(path.resolve(dir));
  if (json) {
    process.stdout.write(JSON.stringify(palette, null, 2) + '\n');
  } else {
    if (palette.services.length === 0) {
      console.log('(no src/Services/*.mo service classes found)');
    } else {
      console.log(paletteText(palette));
    }
  }
}

module.exports = {
  extractProject,
  extractFile,
  findServiceClass,
  extractDecls,
  funcSignature,
  typeSignature,
  letSignature,
  paletteText
};
