// Reload tests for the Aegis Console (development only; the daemon never serves dev/).
//
//   node --test apps/aegis-console/dev/tests/history.test.mjs
//
// The runs panel, orders grid and approvals panel are rebuilt from the daemon's history endpoints
// (GET /api/runs, /api/paypal/orders, /api/approvals; API.md) and merged with live WebSocket messages
// by run_id + seq. The real assets/*.js modules run against fake-dom.mjs with fetch stubbed. Message
// streams are generated here the way aegis-daemon broadcasts them (per-run seq, run.started = 1);
// every number is synthetic (Math.random) and expectations use the console's own formatters.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { installFakeDom, el, roles } from './fake-dom.mjs';

process.env.TZ = 'America/Los_Angeles';

installFakeDom();
const dom = await import('../../assets/dom.js');
const workspaceMod = await import('../../assets/workspace.js');
const paypalMod = await import('../../assets/paypal.js');
const approvalsMod = await import('../../assets/approvals.js');

const text = (node) => (node ? node.textContent.replace(/\s+/g, ' ').trim() : '');
const norm = (v) => v.replace(/\s+/g, ' ').trim();

/** fetch stub: records calls; call(i) resolves once the i-th request has been made. */
function stubFetch() {
  const calls = [];
  const waiters = new Map();
  globalThis.fetch = (url, init = {}) => new Promise((resolve) => {
    const entry = {
      url,
      method: init.method || 'GET',
      respond(data, status = 200) {
        resolve({ ok: status < 400, status, statusText: '', text: async () => JSON.stringify(data) });
      },
    };
    calls.push(entry);
    const w = waiters.get(calls.length - 1);
    if (w) w(entry);
  });
  return {
    calls,
    call(i) {
      return i < calls.length ? Promise.resolve(calls[i]) : new Promise((resolve) => waiters.set(i, resolve));
    },
  };
}

// ---- synthetic daemon run streams ------------------------------------------------------------

const SECRET_LEAK = [['branch', 'ok'], ['edit', 'ok'], ['diff', 'ok'], ['features', 'ok'], ['score', 'ok'], ['policy', 'fail'],
  ['verify', 'skip'], ['rollback', 'ok'], ['finish', 'rolled_back'], ['reap', 'ok']];
const SAFE_REFACTOR = [['branch', 'ok'], ['edit', 'ok'], ['diff', 'ok'], ['features', 'ok'], ['score', 'ok'], ['policy', 'ok'],
  ['verify', 'ok'], ['commit', 'ok'], ['finish', 'committed']];
const MASS_DELETE_APPROVED = [['branch', 'ok'], ['edit', 'ok'], ['diff', 'ok'], ['features', 'ok'], ['score', 'ok'], ['policy', 'fail'],
  ['verify', 'ok'], ['approval', 'pending'], ['finish', 'pending_approval'], ['approval', 'ok'], ['policy', 'ok'], ['commit', 'ok'],
  ['finish', 'committed']];

/** The messages aegis-daemon broadcasts for one run, in order, each with its per-run seq. */
function simulateRun(runId, scenario, title, steps) {
  const ts = () => new Date().toISOString();
  let seq = 0;
  let finishes = 0;
  const messages = [{ type: 'run.started', ts: ts(), run_id: runId, scenario, title, seq: ++seq }];
  for (const [a, b] of steps) {
    if (a === 'finish') {
      messages.push({ type: 'run.finished', ts: ts(), run_id: runId, outcome: b, total_ms: Math.random(), queued_ms: finishes === 0 ? Math.random() : null, seq: ++seq });
      finishes += 1;
    } else {
      messages.push({ type: 'run.stage', ts: ts(), run_id: runId, stage: a, status: b, ms: Math.random(), detail: { synthetic: Math.random() }, seq: ++seq });
    }
  }
  return messages;
}

/** GET /api/runs RunRecord for a run whose broadcast so far is `messages`. */
function historyOf(messages) {
  const started = messages[0];
  const finished = messages.filter((m) => m.type === 'run.finished');
  const last = finished[finished.length - 1];
  const queued = finished.find((f) => f.queued_ms !== null);
  return {
    run_id: started.run_id, scenario: started.scenario, title: started.title, started_at: started.ts,
    outcome: last ? last.outcome : null, total_ms: last ? last.total_ms : null, queued_ms: queued ? queued.queued_ms : null,
    stages: messages.filter((m) => m.type === 'run.stage'), finished, last_seq: messages[messages.length - 1].seq,
  };
}

// ---- runs panel --------------------------------------------------------------------------------

function workspacePanel() {
  const doc = installFakeDom();
  doc.body.append(el('section', { id: 'panel-workspace' },
    ...roles('error', 'summary', 'scenarios', 'action-status', 'pool:dl', 'reaper:dl', 'runs', 'runs-empty:p', 'eventlog:ol', 'reset:button')));
  const panel = workspaceMod.createWorkspacePanel();
  const runsEl = doc.querySelector('[data-role="runs"]');
  const logEl = doc.querySelector('[data-role="eventlog"]');
  const emptyEl = doc.querySelector('[data-role="runs-empty"]');
  const cards = () => runsEl.querySelectorAll('article');
  const card = (runId) => runsEl.querySelector(`article[aria-labelledby="run-${runId}-title"]`);
  const slots = (c) => Object.fromEntries(c.querySelectorAll('li').map((li) => [
    text(li.querySelector('.stage__name')), text(li.querySelector('.stage__head .badge__label'))]));
  const outcome = (c) => text(c.querySelector('.run__head .badge__label'));
  const logLines = () => logEl.children.map((li) => text(li.querySelector('.log__text')));
  return { panel, runsEl, emptyEl, cards, card, slots, outcome, logLines };
}

async function loadHistory(fetch, panel, index, body) {
  const loading = panel.loadRuns();
  const req = await fetch.call(index);
  assert.equal(req.url, '/api/runs');
  assert.equal(req.method, 'GET');
  req.respond(body);
  await loading;
}

test('after a reload the runs panel is rebuilt from GET /api/runs: newest first, outcomes, stages, no placeholders', async () => {
  const fetch = stubFetch();
  const ui = workspacePanel();
  assert.equal(ui.emptyEl.hidden, false);
  const leak = simulateRun('r-000001', 'secret_leak', 'Unformatted secret in config', SECRET_LEAK);
  const md = simulateRun('r-000002', 'mass_delete', 'Mass delete', MASS_DELETE_APPROVED);
  await loadHistory(fetch, ui.panel, 0, { runs: [historyOf(md), historyOf(leak)], limit: 10 });

  const cards = ui.cards();
  assert.deepEqual(cards.map((c) => text(c.querySelector('.run__title'))), ['Mass delete', 'Unformatted secret in config']);
  assert.equal(ui.emptyEl.hidden, true);
  assert.equal(ui.outcome(cards[0]), 'Committed');
  assert.equal(ui.outcome(cards[1]), 'Rolled back');
  for (const [c, messages] of [[cards[0], md], [cards[1], leak]]) {
    assert.equal(c.querySelector('.run__note'), null, 'a run in the history is fully observed');
    const s = ui.slots(c);
    assert.ok(!Object.values(s).includes('not observed'), JSON.stringify(s));
    assert.ok(!Object.values(s).includes('outcome not observed'));
    assert.equal(c.querySelectorAll('.stage__event').length, messages.filter((m) => m.type === 'run.stage').length);
    const finished = messages.filter((m) => m.type === 'run.finished');
    assert.equal(text(c.querySelector('.run__total')), norm(`total ${dom.fmtMs(finished[finished.length - 1].total_ms)}`));
    assert.equal(text(c.querySelector('.run__queued')), norm(`queued ${dom.fmtMs(finished[0].queued_ms)}`), 'queued_ms of the first run.finished');
  }
  assert.equal(ui.slots(cards[1]).Reap, 'OK');
  assert.equal(ui.slots(cards[0]).Reap, 'not run', 'fully observed, so an empty slot is "not run"');
  // The approval slot shows the pending request, the approval and the commit.
  const decision = cards[0].querySelectorAll('li').find((li) => text(li.querySelector('.stage__name')) === 'Approval / commit / rollback');
  assert.deepEqual(decision.querySelectorAll('.stage__sub .mono').map(text), ['approval', 'approval', 'commit']);
});

test('live messages and the history merge by run_id + seq: nothing is shown or logged twice', async () => {
  const fetch = stubFetch();
  const ui = workspacePanel();
  const msgs = simulateRun('r-000003', 'safe_refactor', 'Safe multi-file refactor', SAFE_REFACTOR);
  const stageCount = msgs.filter((m) => m.type === 'run.stage').length;
  // The page loads mid-run: two live stages arrive before the history answers.
  ui.panel.onMessage(msgs[3]);
  ui.panel.onMessage(msgs[4]);
  assert.match(text(ui.card('r-000003').querySelector('.run__note')), /did not see this run start/);
  // The history covers seq 1..6, so it overlaps both live stages and the next one.
  await loadHistory(fetch, ui.panel, 0, { runs: [historyOf(msgs.slice(0, 6))], limit: 10 });
  assert.equal(ui.card('r-000003').querySelector('.run__note'), null, 'complete once the history fills seq 1..');
  for (const m of msgs.slice(5)) ui.panel.onMessage(m); // seq 6 again (already in the history), then the rest
  const c = ui.card('r-000003');
  assert.equal(c.querySelectorAll('.stage__event').length, stageCount, 'every stage exactly once');
  assert.equal(ui.outcome(c), 'Committed');
  assert.equal(c.querySelector('.run__note'), null);
  assert.ok(!Object.values(ui.slots(c)).includes('not observed'));
  const stageLines = ui.logLines().filter((l) => l.startsWith('r-000003 '));
  assert.equal(new Set(stageLines).size, stageLines.length, `a stage was logged twice: ${stageLines.join(' | ')}`);
  assert.equal(ui.logLines().filter((l) => l.startsWith('run r-000003 finished')).length, 1);

  // A later reload of the same history (reconnect, lag notice) changes nothing.
  await loadHistory(fetch, ui.panel, 1, { runs: [historyOf(msgs)], limit: 10 });
  assert.equal(ui.card('r-000003').querySelectorAll('.stage__event').length, stageCount);
  assert.equal(ui.cards().length, 1);
});

test('a history answer older than a live run.finished does not undo the newer outcome', async () => {
  const fetch = stubFetch();
  const ui = workspacePanel();
  const md = simulateRun('r-000004', 'mass_delete', 'Mass delete', MASS_DELETE_APPROVED);
  const firstFinish = md.findIndex((m) => m.type === 'run.finished');
  const loading = ui.panel.loadRuns();
  const req = await fetch.call(0);
  for (const m of md) ui.panel.onMessage(m); // live, while the request is in flight
  req.respond({ runs: [historyOf(md.slice(0, firstFinish + 1))], limit: 10 }); // snapshot at pending_approval
  await loading;
  const c = ui.card('r-000004');
  assert.equal(ui.outcome(c), 'Committed');
  assert.equal(text(c.querySelector('.run__queued')), norm(`queued ${dom.fmtMs(md[firstFinish].queued_ms)}`));
  assert.equal(c.querySelectorAll('.stage__event').length, md.filter((m) => m.type === 'run.stage').length);
});

test('a stream gap is filled by a history load made after it, not by one already in flight', async () => {
  const fetch = stubFetch();
  const ui = workspacePanel();
  const msgs = simulateRun('r-000005', 'safe_refactor', 'Safe multi-file refactor', SAFE_REFACTOR);
  ui.panel.onMessage(msgs[0]);
  ui.panel.onMessage(msgs[1]);
  ui.panel.markGap('disconnect');
  assert.equal(ui.outcome(ui.card('r-000005')), 'outcome not observed');
  // Messages broadcast during the gap never reach the page; the reconnect reload has them.
  await loadHistory(fetch, ui.panel, 0, { runs: [historyOf(msgs)], limit: 10 });
  let c = ui.card('r-000005');
  assert.equal(c.querySelector('.run__note'), null);
  assert.equal(ui.outcome(c), 'Committed');

  const next = simulateRun('r-000006', 'safe_refactor', 'Safe multi-file refactor', SAFE_REFACTOR);
  ui.panel.onMessage(next[0]);
  const loading = ui.panel.loadRuns();
  const req = await fetch.call(1);
  ui.panel.markGap('lag'); // dropped messages after the daemon may already have answered
  req.respond({ runs: [historyOf(next.slice(0, 3)), historyOf(msgs)], limit: 10 });
  await loading;
  c = ui.card('r-000006');
  assert.match(text(c.querySelector('.run__note')), /fell behind/);
  assert.equal(ui.outcome(c), 'outcome not observed');
});

test('the panel keeps the daemon limit of runs, ordered by run id whatever order they arrive in', async () => {
  const fetch = stubFetch();
  const ui = workspacePanel();
  const limit = 10;
  const ids = Array.from({ length: limit }, (_, i) => `r-${String(i + 1).padStart(6, '0')}`);
  const shuffled = ids.map((id) => [Math.random(), id]).sort((a, b) => a[0] - b[0]).map(([, id]) => id);
  const runs = shuffled.map((id) => historyOf(simulateRun(id, 'secret_leak', `Run ${id}`, SECRET_LEAK)));
  await loadHistory(fetch, ui.panel, 0, { runs, limit });
  const shown = () => ui.cards().map((c) => c.getAttribute('aria-labelledby').replace(/^run-|-title$/g, ''));
  assert.deepEqual(shown(), ids.slice().reverse());
  const newest = `r-${String(limit + 1).padStart(6, '0')}`;
  ui.panel.onMessage(simulateRun(newest, 'secret_leak', 'newest', SECRET_LEAK)[0]);
  assert.deepEqual(shown(), [newest, ...ids.slice(1).reverse()], 'the oldest run makes room');
  // A late message of the dropped run does not push a newer one out.
  ui.panel.onMessage({ type: 'run.stage', ts: new Date().toISOString(), run_id: ids[0], stage: 'reap', status: 'ok', ms: Math.random(), detail: {} });
  assert.deepEqual(shown(), [newest, ...ids.slice(1).reverse()]);
  assert.equal(workspaceMod.compareRuns('r-1000000', 'r-999999') < 0, true, 'numeric, not text, order');
});

test('a run whose seq numbers have a hole is marked partial', async () => {
  const fetch = stubFetch();
  const ui = workspacePanel();
  const msgs = simulateRun('r-000007', 'secret_leak', 'Unformatted secret in config', SECRET_LEAK);
  const rec = historyOf(msgs);
  rec.stages = rec.stages.filter((m) => m.seq !== 4);
  await loadHistory(fetch, ui.panel, 0, { runs: [rec], limit: 10 });
  assert.match(text(ui.card('r-000007').querySelector('.run__note')), /seq numbers have gaps/);
  assert.equal(workspaceMod.seqComplete({ seqs: new Set([1, 2, 3]), lastSeq: 3 }), true);
  assert.equal(workspaceMod.seqComplete({ seqs: new Set([1, 3]), lastSeq: 3 }), false);
  assert.equal(workspaceMod.seqComplete({ seqs: new Set(), lastSeq: null }), true, 'no seq information: older daemon');
});

test('without GET /api/runs (older daemon) the panel says so and still drops exact live duplicates', async () => {
  const fetch = stubFetch();
  const ui = workspacePanel();
  const loading = ui.panel.loadRuns();
  const req = await fetch.call(0);
  assert.equal(req.url, '/api/runs');
  req.respond({ error: 'not found' }, 404);
  await loading;
  assert.ok(ui.logLines().some((l) => /GET \/api\/runs: HTTP 404/.test(l)), ui.logLines().join(' | '));
  const stage = { type: 'run.stage', ts: new Date().toISOString(), run_id: 'r-000008', stage: 'branch', status: 'ok', ms: Math.random(), detail: { source: 'pool' } };
  ui.panel.onMessage(stage);
  ui.panel.onMessage({ ...stage });
  assert.equal(ui.card('r-000008').querySelectorAll('.stage__event').length, 1);
  const other = { ...stage, stage: 'recover', detail: { tier: 1 } };
  ui.panel.onMessage(other);
  ui.panel.onMessage({ ...other, detail: { tier: 2 } });
  assert.equal(ui.card('r-000008').querySelectorAll('.stage__event').length, 3, 'same stage and ts but different detail is not a duplicate');
});

// ---- orders grid -------------------------------------------------------------------------------

/** Minimal AG Grid: createGrid keeps rowData; transactions add (at addIndex) and remove rows. */
function fakeAgGrid() {
  const grid = { rows: [] };
  const theme = { withPart() { return theme; }, withParams() { return theme; } };
  globalThis.agGrid = {
    themeQuartz: theme,
    colorSchemeDark: {},
    createGrid(_el, options) {
      grid.rows = options.rowData.slice();
      return {
        destroy() {},
        applyTransactionAsync(tx) {
          if (tx.remove) grid.rows = grid.rows.filter((r) => !tx.remove.includes(r));
          if (tx.add) grid.rows.splice(tx.addIndex || 0, 0, ...tx.add);
        },
      };
    },
  };
  return grid;
}

function paypalPanel() {
  const doc = installFakeDom();
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  doc.body.append(el('section', { id: 'panel-paypal' },
    ...roles('error:p', 'sandbox', 'scenarios', 'action-status:p', 'stream:form', 'stream-toggle:button', 'stream-interval:input',
      'stream-apply:button', 'grid', 'row-count:span', 'timeline', 'timeline-table', 'timeline-note:span', 'scale-toggle:button')));
  const grid = fakeAgGrid();
  const panel = paypalMod.createPaypalPanel();
  const flush = () => { while (frames.length) frames.shift()(); };
  return { panel, grid, flush, count: doc.querySelector('[data-role="row-count"]'), table: doc.querySelector('[data-role="timeline-table"]') };
}

let orderN = 0;
function syntheticOrder() {
  orderN += 1;
  const reference = `agent-${String(orderN).padStart(4, '0')}`;
  const id = `${reference}-${Math.random().toString(16).slice(2, 10)}`;
  const start = Date.now() + orderN;
  const events = ['redact', 'sign', 'verify', 'score', 'policy'].map((resourceId) => {
    const durationUs = Math.random();
    return { id: `${id}:${resourceId}`, resourceId, name: `${reference} ${resourceId}`, startDate: new Date(start).toISOString(),
      endDate: new Date(start).toISOString(), durationUs, orderId: id, cls: 'ok' };
  });
  return { row: { id, reference, submission: 'not_sent', decision: 'Allow' }, events };
}

const RESOURCES = ['redact', 'score', 'policy', 'sign', 'verify', 'submit'].map((id) => ({ id, name: id }));
const ordersBody = (orders, limit) => ({
  rows: orders.map((o) => o.row), columnDefs: [{ field: 'reference', headerName: 'Order' }],
  timeline: { resources: RESOURCES, events: orders.flatMap((o) => o.events) }, ...(limit === undefined ? {} : { limit }),
});

test('orders render from GET /api/paypal/orders on load and the header count matches the grid before and after a reload', () => {
  const limit = 20;
  const placed = Array.from({ length: limit + 5 }, syntheticOrder);
  const before = paypalPanel();
  before.panel.setOrders(ordersBody(placed.slice(0, 7), limit)); // the page loaded with 7 orders
  for (const o of placed.slice(7)) before.panel.onMessage({ type: 'paypal.order', ...o });
  assert.equal(before.grid.rows.length, limit);
  assert.equal(text(before.count), `(${limit} rows, newest ${limit} kept)`);
  assert.deepEqual(before.grid.rows.map((r) => r.id), placed.slice(-limit).map((o) => o.row.id).reverse(), 'newest first');
  before.flush();
  assert.doesNotMatch(text(before.table), new RegExp(placed[0].row.reference), 'trimmed orders leave the timeline');

  // Reload: the daemon answers with its newest `limit` orders (oldest first) and their events.
  const after = paypalPanel();
  after.panel.setOrders(ordersBody(placed.slice(-limit), limit));
  assert.equal(text(after.count), text(before.count));
  assert.deepEqual(after.grid.rows.map((r) => r.id), before.grid.rows.map((r) => r.id));
  after.flush();
  assert.match(text(after.table), new RegExp(placed[placed.length - 1].row.reference));
});

test('an older daemon without `limit` sending more rows is cut to the newest 20, with their timeline events only', () => {
  const placed = Array.from({ length: 30 }, syntheticOrder);
  const ui = paypalPanel();
  const body = ordersBody(placed);
  ui.panel.setOrders(body);
  assert.equal(ui.grid.rows.length, 20);
  assert.equal(text(ui.count), '(20 rows, newest 20 kept)');
  // A stray event of a dropped order carrying the newest start time must not be drawn.
  const stray = { ...placed[0].events[0], id: 'stray', startDate: new Date(Date.now() + 1e9).toISOString(), endDate: new Date(Date.now() + 1e9).toISOString() };
  ui.panel.setOrders({ ...body, timeline: { ...body.timeline, events: [...body.timeline.events, stray] } });
  ui.flush();
  assert.doesNotMatch(text(ui.table), new RegExp(placed[0].row.reference));
  const few = paypalPanel();
  few.panel.setOrders(ordersBody(placed.slice(0, 3), 20));
  assert.equal(text(few.count), '(3 rows)');
});

// ---- approvals ---------------------------------------------------------------------------------

test('pending and resolved approvals render from GET /api/approvals after a reload', () => {
  const doc = installFakeDom();
  doc.body.append(el('section', { id: 'panel-approvals' },
    ...roles('error:p', 'pending', 'resolved:ul', 'pending-count:span', 'action-status:p', 'approver:input')));
  const panel = approvalsMod.createApprovalsPanel();
  const risk = Math.random() * 100;
  const resultMs = Math.random();
  const now = new Date().toISOString();
  panel.setApprovals({
    pending: [{ id: 'a-000002', run_id: 'r-000002', kind: 'workspace', title: 'Mass delete in demo workspace', summary: 's', risk, files_changed: 3,
      excerpt: 'x', determining_policies: ['forbid-high-impact-without-team-approval'], created_at: now, status: 'pending', resolved_by: null,
      resolved_at: null, result: null, cards: { mattermost: 'disabled', slack: 'disabled' } }],
    resolved: [{ id: 'a-000001', run_id: 'r-000001', kind: 'workspace', title: 'Earlier mass delete', status: 'approved', resolved_by: 'lead',
      resolved_at: now, result: { action: 'commit', ms: resultMs, detail: {} }, cards: {} }],
  });
  assert.equal(text(doc.querySelector('[data-role="pending-count"]')), '(1)');
  const card = doc.querySelector('[data-role="pending"] article');
  assert.match(text(card), /Mass delete in demo workspace/);
  assert.ok(card.querySelectorAll('button').some((b) => text(b) === 'Approve'));
  const resolved = doc.querySelector('[data-role="resolved"]').children;
  assert.equal(resolved.length, 1);
  assert.match(text(resolved[0]), /approved/);
  assert.match(text(resolved[0]), /Earlier mass delete/);
  assert.ok(text(resolved[0]).includes(norm(`commit in ${dom.fmtMs(resultMs)}`)));
});
