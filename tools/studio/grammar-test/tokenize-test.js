#!/usr/bin/env node
/*
 * tokenize-test.js — HEADLESS tokenization test for the .mview TextMate grammar.
 *
 * Uses vscode-textmate + vscode-oniguruma (the exact engine that backs VS Code
 * and Monaco's textmate integration), so this is a REAL test of the grammar that
 * apps/studio ships, not a hand-rolled regex approximation.
 *
 * It loads apps/studio/assets/mview.tmLanguage.json, tokenizes a representative
 * .mview document, and asserts that key constructs receive the right scopes:
 *   - @page              -> a directive scope
 *   - <section> tag name -> entity.name.tag
 *   - <Button> component  -> entity.name.tag
 *   - secure attribute    -> the secure attribute scope
 *   - @click event        -> the event attribute scope
 *   - @count interpolation-> interpolated variable scope
 *   - @code{} body        -> source.motoko (embedded) with Motoko keywords
 *
 * Exit 0 = all assertions passed (grammarTokenizesOk). Nonzero = failure.
 */
'use strict';

const fs = require('fs');
const path = require('path');
const vsctm = require('vscode-textmate');
const oniguruma = require('vscode-oniguruma');

const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const GRAMMAR_PATH = path.join(
  REPO_ROOT,
  'apps',
  'studio',
  'assets',
  'mview.tmLanguage.json'
);

// A representative .mview document exercising every construct the grammar covers.
const SAMPLE = [
  '@page "/"',
  '@layout StudioLayout',
  '@title "Demo"',
  '@theme brand="#6d28d9"',
  '',
  '<section class="mv-container">',
  '  <h1>Hello, @name</h1>',
  '  <p>Current count: <strong>@count</strong></p>',
  '  @if ready {',
  '    <form @submit="save" secure>',
  '      <InputText name="amount" bind="@amount" required />',
  '      <Button kind="primary" @click="increment">+1</Button>',
  '    </form>',
  '  }',
  '  @for item in items {',
  '    <li>@item.label</li>',
  '  }',
  '  <pre>@raw(html)</pre>',
  '</section>',
  '',
  '@code {',
  '  var count : Nat = 0;',
  '  let name : Text = "world";',
  '  func increment() : async () { count += 1; };',
  '}'
].join('\n');

async function loadOniguruma() {
  // Resolve the wasm shipped inside vscode-oniguruma.
  const wasmPath = require.resolve('vscode-oniguruma/release/onig.wasm');
  const wasmBin = fs.readFileSync(wasmPath).buffer;
  await oniguruma.loadWASM(wasmBin);
  return {
    createOnigScanner(patterns) {
      return new oniguruma.OnigScanner(patterns);
    },
    createOnigString(s) {
      return new oniguruma.OnigString(s);
    }
  };
}

async function main() {
  const onigLib = loadOniguruma();
  const registry = new vsctm.Registry({
    onigLib,
    loadGrammar: async (scopeName) => {
      if (scopeName === 'source.mview') {
        const raw = fs.readFileSync(GRAMMAR_PATH, 'utf8');
        return vsctm.parseRawGrammar(raw, GRAMMAR_PATH);
      }
      // We deliberately do not provide a real source.motoko grammar here; the
      // grammar's own #motoko fallback colours the embedded block. Returning
      // null is fine — textmate keeps the parent scope.
      return null;
    }
  });

  const grammar = await registry.loadGrammar('source.mview');
  if (!grammar) {
    console.error('FAIL: could not load grammar source.mview');
    process.exit(1);
  }

  // Tokenize line by line, collecting (line, token-text, scopes[]).
  const lines = SAMPLE.split('\n');
  let ruleStack = vsctm.INITIAL;
  /** @type {{line:number, text:string, scopes:string[]}[]} */
  const tokens = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const res = grammar.tokenizeLine(line, ruleStack);
    for (const t of res.tokens) {
      tokens.push({
        line: i,
        text: line.substring(t.startIndex, t.endIndex),
        scopes: t.scopes
      });
    }
    ruleStack = res.ruleStack;
  }

  // Helper: find a token whose trimmed text === `text` and whose scope stack
  // contains a scope matching `scopePredicate`.
  function has(text, scopeNeedle) {
    return tokens.some(
      (t) =>
        t.text.trim() === text &&
        t.scopes.some((s) => s.includes(scopeNeedle))
    );
  }
  function hasScopeAnywhereOnLine(lineText, scopeNeedle) {
    return tokens.some(
      (t) => t.scopes.some((s) => s.includes(scopeNeedle))
    );
  }

  /** @type {{name:string, ok:boolean, detail:string}[]} */
  const checks = [];
  function check(name, ok, detail) {
    checks.push({ name, ok: !!ok, detail: detail || '' });
  }

  // 1. @page -> directive scope.
  check(
    '@page is a directive scope',
    has('page', 'keyword.control.directive'),
    'expected keyword.control.directive on "page"'
  );

  // 2. @theme -> theme directive scope.
  check(
    '@theme is a theme directive scope',
    has('theme', 'directive.theme'),
    'expected keyword.control.directive.theme on "theme"'
  );

  // 3. tag name -> entity.name.tag (HTML element).
  check(
    '<section> tag name is entity.name.tag',
    has('section', 'entity.name.tag'),
    'expected entity.name.tag on "section"'
  );

  // 4. component tag <Button> -> entity.name.tag too.
  check(
    '<Button> component is entity.name.tag',
    has('Button', 'entity.name.tag'),
    'expected entity.name.tag on "Button"'
  );

  // 5. secure attribute -> the secure attribute scope.
  check(
    'secure attribute has the secure scope',
    has('secure', 'attribute.secure'),
    'expected keyword.other.attribute.secure on "secure"'
  );

  // 6. @click event binding -> event attribute scope.
  check(
    '@click is an event attribute scope',
    has('click', 'attribute-name.event'),
    'expected entity.other.attribute-name.event on "click"'
  );

  // 7. @submit event binding -> event attribute scope.
  check(
    '@submit is an event attribute scope',
    has('submit', 'attribute-name.event'),
    'expected entity.other.attribute-name.event on "submit"'
  );

  // 8. @count -> interpolated variable.
  check(
    '@count is an interpolated variable',
    has('count', 'variable.other.interpolated'),
    'expected variable.other.interpolated on "count"'
  );

  // 9. @if -> control flow.
  check(
    '@if is control flow',
    has('if', 'keyword.control.flow'),
    'expected keyword.control.flow on "if"'
  );

  // 10. @for -> control flow.
  check(
    '@for is control flow',
    has('for', 'keyword.control.flow'),
    'expected keyword.control.flow on "for"'
  );

  // 11. @raw -> raw keyword.
  check(
    '@raw is the raw keyword',
    has('raw', 'keyword.other.raw'),
    'expected keyword.other.raw on "raw"'
  );

  // 12. @code body is embedded Motoko (source.motoko on the block) and Motoko
  //     keywords get coloured by the #motoko fallback.
  const codeBodyTokens = tokens.filter((t) =>
    t.scopes.some((s) => s === 'source.motoko' || s.includes('meta.embedded.block.motoko'))
  );
  check(
    '@code body is scoped source.motoko / embedded',
    codeBodyTokens.length > 0,
    'expected some tokens under source.motoko inside @code{}'
  );
  check(
    'Motoko keyword "func" coloured inside @code',
    has('func', 'keyword.control.motoko'),
    'expected keyword.control.motoko on "func"'
  );
  check(
    'Motoko type "Nat" coloured inside @code',
    has('Nat', 'storage.type.primitive.motoko') || has('Nat', 'entity.name.type'),
    'expected a type scope on "Nat"'
  );

  // Report.
  let allOk = true;
  for (const c of checks) {
    const tag = c.ok ? 'PASS' : 'FAIL';
    if (!c.ok) allOk = false;
    console.log(`  [${tag}] ${c.name}${c.ok ? '' : '  <- ' + c.detail}`);
  }
  console.log('');
  console.log(
    `grammar tokenization: ${checks.filter((c) => c.ok).length}/${checks.length} assertions passed`
  );

  if (!allOk) {
    console.error('FAIL: one or more grammar assertions failed.');
    process.exit(1);
  }
  console.log('OK: all .mview grammar assertions passed (headless, vscode-textmate).');
  process.exit(0);
}

main().catch((err) => {
  console.error('ERROR running grammar test:', err);
  process.exit(2);
});
