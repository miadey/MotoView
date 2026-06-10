'use strict';
/*
 * observability.js — MotokoStudio R7 debug/observability core (design-time, no deps).
 *
 * Two pure, testable pieces that turn a server-driven MotoView canister into an
 * OBSERVABLE app, plus a headless HTML renderer the live studio panel embeds.
 *
 *  1. parseLogStream(text)  — turns the structured `Debug.print` stream the
 *     `--instrument` build emits (e.g. piped from `dfx canister logs <id>`) into
 *     structured event records: { tag, page, handler, event, caller, lastBatch,
 *     costInstr, raw }. The wire format is the ONE the compiler emits in the
 *     generated `mvDispatch` (see compiler/src/codegen.rs):
 *
 *       MV|dispatch|page=Counter|handler=increment|event=increment|\
 *          caller=<principal>|lastBatch=<id>|costInstr=42
 *
 *     Pipe-delimited segments; the first two are the literal tag namespace
 *     (`MV`, `dispatch`); the rest are `key=value`. Unknown keys are preserved
 *     under `fields`. Non-MV lines (dfx prefixes, other prints) are ignored.
 *     `dfx canister logs` prefixes each line with a sequence/timestamp; the
 *     parser scans for the `MV|` marker anywhere in the line, so those prefixes
 *     are tolerated and captured as `prefix`.
 *
 *  2. buildViewTree(forest) — turns R4's IR forest JSON (the array `motoview
 *     preview` prints; schema in runtime/src/Ir.mo: {t:"el"|"text"|"raw",...})
 *     into a collapsible view-hierarchy: { kind, tag, label, events, attrs,
 *     handlers, children, source }. `handlers` lifts each element's event->handler
 *     map to a flat list so the timeline can deep-link a logged `handler` back to
 *     the node that fires it (click-to-source). `raw` whitespace-only text nodes
 *     are dropped so the tree reads like the authored markup.
 *
 *  3. renderTreeHtml / renderTimelineHtml — headless, dependency-free HTML the
 *     studio panel mounts. Tested for STRUCTURE/escaping here; the live panel
 *     (collapse interactions, click-to-source wiring) is browser-flagged.
 *
 * EXPLICITLY OUT OF SCOPE (honest): source-step debugging / attaching a debugger.
 * Impossible on the replicated IC — there is no single deterministic stepping
 * host to break into. Faithful record/replay is the R10 leapfrog item, not this.
 */

// ---- 1. log parser --------------------------------------------------------

const TAG = 'MV';
const KIND = 'dispatch';

/**
 * Parse one log line. Returns a structured record, or null if the line carries
 * no MotoView dispatch marker.
 */
function parseLogLine(line) {
  if (typeof line !== 'string') return null;
  const at = line.indexOf(TAG + '|');
  if (at < 0) return null;
  const prefix = line.slice(0, at).trim();
  // Some logging hosts wrap the payload in quotes/brackets; trim a trailing
  // quote so the last field doesn't absorb it.
  let payload = line.slice(at).trim();
  payload = payload.replace(/["'\]\)]+$/, '');
  const segs = payload.split('|');
  if (segs.length < 2 || segs[0] !== TAG || segs[1] !== KIND) return null;

  const fields = {};
  for (let i = 2; i < segs.length; i++) {
    const seg = segs[i];
    const eq = seg.indexOf('=');
    if (eq < 0) continue;
    const k = seg.slice(0, eq);
    const v = seg.slice(eq + 1);
    fields[k] = v;
  }

  const costRaw = fields.costInstr;
  const cost = costRaw != null && /^\d+$/.test(costRaw) ? Number(costRaw) : null;

  return {
    tag: TAG,
    kind: KIND,
    page: fields.page || '',
    handler: fields.handler || '',
    event: fields.event || fields.handler || '',
    caller: fields.caller || '',
    lastBatch: fields.lastBatch || '',
    costInstr: cost,
    prefix: prefix || null,
    fields,
    raw: payload,
  };
}

/**
 * Parse a whole log stream (e.g. the stdout of `dfx canister logs <id>`).
 * Returns the structured dispatch events in stream order. Lines without an
 * `MV|dispatch|` marker are silently skipped.
 */
function parseLogStream(text) {
  if (text == null) return [];
  const out = [];
  for (const line of String(text).split(/\r?\n/)) {
    const rec = parseLogLine(line);
    if (rec) out.push(rec);
  }
  return out;
}

/**
 * Roll up a parsed stream into a per-handler summary: count + total/avg/max
 * instruction cost. Feeds the panel's "hot handlers" view.
 */
function summarizeEvents(events) {
  const by = new Map();
  for (const e of events) {
    const key = `${e.page}.${e.handler}`;
    const s = by.get(key) || {
      page: e.page,
      handler: e.handler,
      count: 0,
      totalCost: 0,
      maxCost: 0,
      withCost: 0,
    };
    s.count += 1;
    if (typeof e.costInstr === 'number') {
      s.totalCost += e.costInstr;
      s.withCost += 1;
      if (e.costInstr > s.maxCost) s.maxCost = e.costInstr;
    }
    by.set(key, s);
  }
  return Array.from(by.values()).map((s) => ({
    page: s.page,
    handler: s.handler,
    count: s.count,
    totalCost: s.totalCost,
    avgCost: s.withCost ? Math.round(s.totalCost / s.withCost) : null,
    maxCost: s.withCost ? s.maxCost : null,
  }));
}

// ---- 2. view-tree builder -------------------------------------------------

function isWhitespaceRaw(node) {
  return (
    node &&
    node.t === 'raw' &&
    typeof node.html === 'string' &&
    node.html.trim() === ''
  );
}

/**
 * Build the collapsible view-hierarchy for ONE IR node. `path` is the dotted
 * index path from the forest root (used for stable node ids / click-to-source).
 */
function buildViewNode(node, path) {
  if (!node || typeof node !== 'object') return null;
  const id = path.join('.');
  switch (node.t) {
    case 'el': {
      const events = node.events && typeof node.events === 'object' ? node.events : {};
      // Flatten the event->handler map into [{event, handler}] so the timeline
      // can map a logged `handler` back to the firing node (click-to-source).
      const handlers = Object.keys(events).map((ev) => ({
        event: ev,
        handler: events[ev],
      }));
      const children = [];
      const kids = Array.isArray(node.children) ? node.children : [];
      kids.forEach((c, i) => {
        if (isWhitespaceRaw(c)) return; // drop layout whitespace
        const built = buildViewNode(c, path.concat(i));
        if (built) children.push(built);
      });
      return {
        kind: 'el',
        id,
        tag: node.tag || '',
        label: node.tag || 'el',
        attrs: node.attrs && typeof node.attrs === 'object' ? node.attrs : {},
        events,
        handlers,
        children,
        source: { path: id, tag: node.tag || '', handlers },
      };
    }
    case 'text':
      return {
        kind: 'text',
        id,
        label: 'text',
        value: typeof node.value === 'string' ? node.value : '',
        children: [],
        source: { path: id },
      };
    case 'raw':
      return {
        kind: 'raw',
        id,
        label: 'raw',
        html: typeof node.html === 'string' ? node.html : '',
        children: [],
        source: { path: id },
      };
    default:
      return null;
  }
}

/**
 * Build the view hierarchy for a whole IR forest (the array `motoview preview`
 * emits). Accepts either the parsed array or its JSON text. Returns an array of
 * root view-nodes; whitespace-only raw nodes are dropped.
 */
function buildViewTree(forest) {
  let arr = forest;
  if (typeof forest === 'string') {
    arr = JSON.parse(forest);
  }
  if (!Array.isArray(arr)) {
    throw new Error('buildViewTree: expected an IR forest array');
  }
  const roots = [];
  arr.forEach((n, i) => {
    if (isWhitespaceRaw(n)) return;
    const built = buildViewNode(n, [i]);
    if (built) roots.push(built);
  });
  return roots;
}

/**
 * Walk the view tree depth-first, yielding each node with its depth. Convenience
 * for tests and the flat-list panel rendering.
 */
function flattenTree(roots, depth = 0, out = []) {
  for (const n of roots) {
    out.push({ depth, node: n });
    flattenTree(n.children || [], depth + 1, out);
  }
  return out;
}

/**
 * Find every view-node that fires a given handler name. The timeline uses this
 * to deep-link a logged `handler` to its source node(s).
 */
function nodesForHandler(roots, handler) {
  const hits = [];
  for (const { node } of flattenTree(roots)) {
    if (node.handlers && node.handlers.some((h) => h.handler === handler)) {
      hits.push(node);
    }
  }
  return hits;
}

// ---- 3. headless HTML renderers (panel body; interactions browser-flagged) -

function esc(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/** Render the view hierarchy as a collapsible <ul> tree (details/summary). */
function renderTreeHtml(roots) {
  const li = (n) => {
    const handlerBadge =
      n.handlers && n.handlers.length
        ? n.handlers
            .map(
              (h) =>
                `<span class="mv-ev" data-handler="${esc(h.handler)}">@${esc(
                  h.event
                )}=${esc(h.handler)}</span>`
            )
            .join('')
        : '';
    let head;
    if (n.kind === 'el') head = `&lt;${esc(n.tag)}&gt;`;
    else if (n.kind === 'text') head = `"${esc(n.value)}"`;
    else head = 'raw';
    const summary = `<summary data-path="${esc(n.id)}" data-kind="${esc(
      n.kind
    )}">${head}${handlerBadge}</summary>`;
    if (n.children && n.children.length) {
      return `<li><details open>${summary}<ul>${n.children
        .map(li)
        .join('')}</ul></details></li>`;
    }
    return `<li>${summary}</li>`;
  };
  return `<ul class="mv-viewtree">${roots.map(li).join('')}</ul>`;
}

/** Render the event/batch timeline as an ordered list of rows. */
function renderTimelineHtml(events) {
  const rows = events
    .map((e, i) => {
      const cost = typeof e.costInstr === 'number' ? `${e.costInstr} instr` : '—';
      return (
        `<li class="mv-tl-row" data-handler="${esc(e.handler)}" data-batch="${esc(
          e.lastBatch
        )}">` +
        `<span class="mv-tl-i">#${i + 1}</span>` +
        `<span class="mv-tl-page">${esc(e.page)}</span>` +
        `<span class="mv-tl-handler">${esc(e.handler)}</span>` +
        `<span class="mv-tl-caller">${esc(e.caller)}</span>` +
        `<span class="mv-tl-batch">${esc(e.lastBatch)}</span>` +
        `<span class="mv-tl-cost">${esc(cost)}</span>` +
        `</li>`
      );
    })
    .join('');
  return `<ol class="mv-timeline">${rows}</ol>`;
}

module.exports = {
  parseLogLine,
  parseLogStream,
  summarizeEvents,
  buildViewNode,
  buildViewTree,
  flattenTree,
  nodesForHandler,
  renderTreeHtml,
  renderTimelineHtml,
};
