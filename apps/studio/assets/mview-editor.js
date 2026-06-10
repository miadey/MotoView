/*
 * mview-editor.js — the MotokoStudio code editor.
 *
 * This is a DEV-TOOL script for the studio shell. The framework's rule is "no
 * app JS in SHIPPED apps"; the studio is a design tool, so an editor script here
 * is the documented exception (see tools/studio/README.md and the design notes).
 *
 * It does three things:
 *   1. Mounts CodeMirror 6 (loaded from a pinned ESM CDN) into #mv-editor.
 *   2. Highlights .mview syntax with a StreamLanguage tokenizer whose token
 *      classes mirror apps/studio/assets/mview.tmLanguage.json (template tags,
 *      @directives, @expr/@raw, secure/bind/@event attributes, embedded Motoko
 *      inside @code{...}). CodeMirror does not consume TextMate grammars natively
 *      without a lezer/textmate bridge, so the tokenizer is a faithful port — the
 *      .tmLanguage.json remains the portable source of truth for VS Code / Monaco.
 *   3. Runs an inline-diagnostics linter: it POSTs the buffer to the design-time
 *      bridge (tools/studio/diagnostics-server.js -> `motoview lint/check --json`)
 *      and renders the returned {line,col,endLine,endCol} as squiggles + a gutter.
 *
 * The script degrades gracefully: if the CDN is unreachable it leaves a readable
 * <pre> fallback in place and shows a notice; if the diagnostics bridge is down
 * the editor still highlights, it just shows "diagnostics: bridge offline".
 *
 * Pinned versions keep the dev tool reproducible.
 */

const CM_VERSION = '6.0.1';
const CDN = (pkg, ver, sub) =>
  `https://cdn.jsdelivr.net/npm/${pkg}@${ver}${sub ? '/' + sub : ''}/+esm`;

// The design-time diagnostics bridge. Same-origin /__studio/diagnostics if the
// bridge is reverse-proxied; otherwise a localhost dev port. Overridable via a
// global the page can set before this script runs.
const DIAG_ENDPOINT =
  (typeof window !== 'undefined' && window.MV_STUDIO_DIAG_ENDPOINT) ||
  'http://127.0.0.1:8731/diagnostics';

/**
 * Build a CodeMirror StreamLanguage tokenizer for .mview. The token names map to
 * CM highlight tags; they intentionally line up with the TextMate scopes so the
 * two stay in sync conceptually.
 */
function makeMviewStreamParser(StreamLanguage, tags, HighlightStyle, syntaxHighlighting) {
  const DIRECTIVES = new Set([
    'page', 'layout', 'title', 'description', 'theme', 'head', 'yield',
    'authorize', 'cacheable', 'section', 'name'
  ]);
  const FLOW = new Set(['if', 'else', 'for', 'switch', 'case', 'default']);
  const EVENTS = new Set([
    'click', 'submit', 'input', 'change', 'keyup', 'keydown',
    'focus', 'blur', 'load', 'mount'
  ]);
  const MOTOKO_KW = new Set([
    'actor', 'and', 'async', 'assert', 'await', 'break', 'case', 'catch',
    'class', 'continue', 'debug', 'debug_show', 'do', 'else', 'for', 'func',
    'if', 'ignore', 'import', 'in', 'module', 'not', 'object', 'or', 'label',
    'let', 'loop', 'private', 'public', 'query', 'return', 'shared', 'stable',
    'switch', 'system', 'throw', 'try', 'type', 'var', 'while', 'with',
    'true', 'false', 'null'
  ]);
  const MOTOKO_TYPES = new Set([
    'Nat', 'Nat8', 'Nat16', 'Nat32', 'Nat64', 'Int', 'Int8', 'Int16', 'Int32',
    'Int64', 'Float', 'Bool', 'Text', 'Char', 'Blob', 'Principal', 'Error',
    'Any', 'None', 'Null', 'Region'
  ]);

  const parser = {
    name: 'mview',
    startState() {
      return { inCode: false, codeDepth: 0, inString: false, inTag: false };
    },
    token(stream, state) {
      // --- embedded Motoko inside @code { ... } ---
      if (state.inCode) {
        if (stream.match(/^\/\/.*/)) return 'comment';
        if (stream.match(/^"(?:[^"\\]|\\.)*"/)) return 'string';
        if (stream.match(/^#[A-Za-z_][A-Za-z0-9_]*/)) return 'tagName';
        if (stream.match(/^\}/)) {
          state.codeDepth--;
          if (state.codeDepth <= 0) { state.inCode = false; return 'brace'; }
          return 'brace';
        }
        if (stream.match(/^\{/)) { state.codeDepth++; return 'brace'; }
        const w = stream.match(/^[A-Za-z_][A-Za-z0-9_]*/);
        if (w) {
          const t = w[0];
          if (MOTOKO_KW.has(t)) return 'keyword';
          if (MOTOKO_TYPES.has(t) || /^[A-Z]/.test(t)) return 'typeName';
          return null;
        }
        if (stream.match(/^[0-9][0-9_]*(\.[0-9_]+)?/)) return 'number';
        stream.next();
        return null;
      }

      // --- @code { begins the embedded block ---
      if (stream.match(/^@code\b/)) {
        // consume up to and including the opening brace
        stream.eatSpace();
        if (stream.eat('{')) {
          state.inCode = true;
          state.codeDepth = 1;
        }
        return 'keyword';
      }

      // --- HTML comment ---
      if (stream.match(/^<!--/)) {
        while (!stream.eol()) {
          if (stream.match(/-->/)) break;
          stream.next();
        }
        return 'comment';
      }

      // --- @raw( ... ) ---
      if (stream.match(/^@raw(?=\s*\()/)) return 'keyword';

      // --- template control flow @if @for @switch ... ---
      const flow = stream.match(/^@(if|else|for|switch|case|default)\b/);
      if (flow) return 'controlKeyword';

      // --- top-of-line / header directives ---
      const dir = stream.match(/^@(page|layout|title|description|theme|head|yield|authorize|cacheable|section)\b/);
      if (dir) return 'definitionKeyword';

      // --- @event inside tags (@click, @submit, ...) ---
      const ev = stream.match(/^@(click|submit|input|change|keyup|keydown|focus|blur|load|mount)\b/);
      if (ev) return 'attributeName';

      // --- escaped @@ literal ---
      if (stream.match(/^@@/)) return null;

      // --- @ident / @(expr) interpolation ---
      if (stream.match(/^@\(/)) return 'meta';
      if (stream.match(/^@[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*(\(\))?/)) {
        return 'variableName';
      }

      // --- tag open/close ---
      if (stream.match(/^<\/?[A-Za-z][A-Za-z0-9_-]*/)) {
        state.inTag = true;
        return 'tagName';
      }
      if (state.inTag) {
        if (stream.match(/^\/?>/)) { state.inTag = false; return 'angleBracket'; }
        if (stream.match(/^"(?:[^"\\]|\\.)*"/)) return 'string';
        if (stream.match(/^'(?:[^'\\]|\\.)*'/)) return 'string';
        if (stream.match(/^\bsecure\b/)) return 'keyword';
        if (stream.match(/^\b(bind|required|disabled|readonly|checked|autofocus|novalidate)\b/)) return 'modifier';
        const at = stream.match(/^[A-Za-z_:][A-Za-z0-9_:.-]*/);
        if (at) return 'propertyName';
        stream.next();
        return null;
      }

      // --- HTML entity ---
      if (stream.match(/^&[a-zA-Z0-9#]+;/)) return 'character';

      stream.next();
      return null;
    }
  };

  const lang = StreamLanguage.define(parser);

  // Map the token names above onto highlight tags.
  const style = HighlightStyle.define([
    { tag: tags.definitionKeyword, color: '#c084fc', fontWeight: 'bold' },
    { tag: tags.controlKeyword, color: '#c084fc' },
    { tag: tags.keyword, color: '#c084fc' },
    { tag: tags.tagName, color: '#f472b6' },
    { tag: tags.typeName, color: '#7dd3fc' },
    { tag: tags.propertyName, color: '#fbbf24' },
    { tag: tags.attributeName, color: '#fbbf24', fontStyle: 'italic' },
    { tag: tags.modifier, color: '#7dd3fc' },
    { tag: tags.variableName, color: '#86efac' },
    { tag: tags.string, color: '#86efac' },
    { tag: tags.number, color: '#fdba74' },
    { tag: tags.comment, color: '#6b7280', fontStyle: 'italic' },
    { tag: tags.meta, color: '#c084fc' },
    { tag: tags.character, color: '#fdba74' },
    { tag: tags.angleBracket, color: '#9ca3af' },
    { tag: tags.brace, color: '#9ca3af' }
  ]);

  return [lang, syntaxHighlighting(style)];
}

/**
 * Convert one {line,col,endLine,endCol} diagnostic (1-based, from the R2 JSON)
 * into a CodeMirror diagnostic with absolute document offsets.
 */
function toCmDiagnostic(view, d) {
  const doc = view.state.doc;
  const clampLine = (n) => Math.min(Math.max(n, 1), doc.lines);
  const sl = clampLine(d.line || 1);
  const el = clampLine(d.endLine && d.endLine >= d.line ? d.endLine : sl);
  const startLine = doc.line(sl);
  const endLine = doc.line(el);
  let from = startLine.from + Math.max((d.col || 1) - 1, 0);
  let to = endLine.from + Math.max((d.endCol || (d.col || 1) + 1) - 1, 0);
  from = Math.min(Math.max(from, 0), doc.length);
  to = Math.min(Math.max(to, from + 0), doc.length);
  if (to <= from) to = Math.min(from + 1, doc.length);
  const sev =
    d.severity === 'error' ? 'error' :
    d.severity === 'info' ? 'info' : 'warning';
  return {
    from,
    to,
    severity: sev,
    source: d.rule ? `motoview:${d.rule}` : 'motoview',
    message: d.message || '(no message)'
  };
}

/**
 * Ask the design-time bridge to lint+check the current buffer. Returns an array
 * of R2 diagnostics, or throws if the bridge is unreachable.
 */
async function fetchDiagnostics(source) {
  const res = await fetch(DIAG_ENDPOINT, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ source, file: 'src/Pages/Design.mview' })
  });
  if (!res.ok) throw new Error('bridge HTTP ' + res.status);
  const data = await res.json();
  // The bridge returns { diagnostics: [...], saveable: bool, ... }.
  return data;
}

function setStatus(text, kind) {
  const el = typeof document !== 'undefined' && document.getElementById('mv-editor-status');
  if (!el) return;
  el.textContent = text;
  el.dataset.kind = kind || 'info';
}

/**
 * Public bootstrap. Pulls CodeMirror from the CDN, mounts it, wires diagnostics.
 * Exported so a headless test (jsdom) can drive it with injected modules.
 */
export async function mountStudioEditor(opts) {
  opts = opts || {};
  const mountSel = opts.mount || '#mv-editor';
  const mountEl =
    typeof document !== 'undefined' ? document.querySelector(mountSel) : null;
  if (!mountEl) return null;

  const initialDoc =
    opts.doc != null
      ? opts.doc
      : (mountEl.getAttribute('data-source') ||
         (mountEl.querySelector('pre') && mountEl.querySelector('pre').textContent) ||
         '');

  // Allow a test harness to inject the CM modules; otherwise import from CDN.
  const mods = opts.modules || (await loadCodeMirror());
  const {
    EditorView, EditorState, basicSetup,
    StreamLanguage, HighlightStyle, syntaxHighlighting, tags,
    linter, lintGutter
  } = mods;

  const langExt = makeMviewStreamParser(
    StreamLanguage, tags, HighlightStyle, syntaxHighlighting
  );

  // The async linter: debounced by CodeMirror; we hit the bridge per run.
  const mviewLinter = linter(
    async (view) => {
      const source = view.state.doc.toString();
      try {
        const data = await fetchDiagnostics(source);
        const diags = (data.diagnostics || []).map((d) => toCmDiagnostic(view, d));
        const errs = diags.filter((d) => d.severity === 'error').length;
        const warns = diags.filter((d) => d.severity === 'warning').length;
        setStatus(
          data.saveable
            ? `saveable · gate passed (${warns} warning${warns === 1 ? '' : 's'})`
            : `blocked by gate · ${errs} error${errs === 1 ? '' : 's'}, ${warns} warning${warns === 1 ? '' : 's'}`,
          data.saveable ? 'ok' : 'bad'
        );
        return diags;
      } catch (e) {
        setStatus('diagnostics: bridge offline (run tools/studio/diagnostics-server.js)', 'warn');
        return [];
      }
    },
    { delay: 600 }
  );

  // Clear the fallback <pre> before mounting.
  mountEl.textContent = '';

  const view = new EditorView({
    parent: mountEl,
    state: EditorState.create({
      doc: initialDoc,
      extensions: [
        basicSetup,
        langExt,
        lintGutter(),
        mviewLinter,
        EditorView.theme({
          '&': { fontSize: '13px', background: '#1c1730', color: '#e6e1f5' },
          '.cm-gutters': { background: '#171228', color: '#6b7280', border: 'none' },
          '.cm-activeLine': { background: 'rgba(255,255,255,0.03)' },
          '.cm-content': { fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace' }
        })
      ]
    })
  });

  setStatus('editor ready · type to lint', 'info');
  return view;
}

/** Pull the pinned CodeMirror 6 ESM bundle + its language/lint helpers. */
async function loadCodeMirror() {
  const [
    cmState,
    cmView,
    cmBasic,
    cmLang,
    cmHighlight,
    cmLint
  ] = await Promise.all([
    import(CDN('@codemirror/state', CM_VERSION)),
    import(CDN('@codemirror/view', CM_VERSION)),
    import(CDN('codemirror', CM_VERSION)),
    import(CDN('@codemirror/language', CM_VERSION)),
    import(CDN('@lezer/highlight', '1.2.0')),
    import(CDN('@codemirror/lint', CM_VERSION))
  ]);
  return {
    EditorState: cmState.EditorState,
    EditorView: cmView.EditorView,
    basicSetup: cmBasic.basicSetup,
    StreamLanguage: cmLang.StreamLanguage,
    HighlightStyle: cmLang.HighlightStyle,
    syntaxHighlighting: cmLang.syntaxHighlighting,
    tags: cmHighlight.tags,
    linter: cmLint.linter,
    lintGutter: cmLint.lintGutter
  };
}

// Export the pure helpers so a headless test can exercise them without a CDN.
export { makeMviewStreamParser, toCmDiagnostic, fetchDiagnostics };

// Auto-mount when loaded in a real browser with the mount point present.
// A test harness sets window.MV_STUDIO_NO_AUTOMOUNT = true and drives
// mountStudioEditor() itself with injected modules (so it never reaches the CDN).
if (
  typeof document !== 'undefined' &&
  typeof window !== 'undefined' &&
  !window.MV_STUDIO_NO_AUTOMOUNT
) {
  const boot = () => {
    const el = document.querySelector('#mv-editor');
    if (!el) return;
    // If the CDN is unreachable, keep the server-rendered <pre> fallback and just
    // show a notice — never throw an unhandled rejection.
    Promise.resolve()
      .then(() => mountStudioEditor({ mount: '#mv-editor' }))
      .catch(() => {
        setStatus('editor offline (CodeMirror CDN unreachable) — showing read-only source', 'warn');
      });
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
}
