// Tests for the cockpit redesign (development only; the daemon never serves dev/).
//
//   node --test apps/aegis-console/dev/tests/cockpit.test.mjs
//
// Covers the diff viewer (tokenizer, parser, run-card blocks for the branch diff and the self-heal
// patch), the signed-receipt popover, the crypto pills in the orders grid, the warm/cold latency
// bars, the header hardware strip and the hero signal path. The real assets/*.js modules run against
// fake-dom.mjs with fetch stubbed; every number is synthetic (Math.random) and expectations use the
// console's own formatters, so nothing here is a typed-in measurement.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { installFakeDom, el, roles } from './fake-dom.mjs';

installFakeDom();
const dom = await import('../../assets/dom.js');
const diffview = await import('../../assets/diffview.js');
const workspaceMod = await import('../../assets/workspace.js');
const paypalMod = await import('../../assets/paypal.js');
const receiptMod = await import('../../assets/receipt.js');
const scoreboardMod = await import('../../assets/scoreboard.js');
const cockpitMod = await import('../../assets/cockpit.js');
const pipelineMod = await import('../../assets/pipeline.js');

const text = (node) => (node ? node.textContent.replace(/\s+/g, ' ').trim() : '');
/** Text of an element's children joined by spaces (flex rows separate them visually with a gap). */
const words = (node) => (node ? node.children.map(text).filter(Boolean).join(' ') : '');
const drain = () => new Promise((resolve) => setImmediate(resolve));
const rnd = () => Math.random();
const rndInt = (lo, hi) => lo + Math.floor(Math.random() * (hi - lo + 1));

function stubFetch() {
  const calls = [];
  const waiters = new Map();
  globalThis.fetch = (url, init = {}) => new Promise((resolve) => {
    const entry = { url, method: init.method || 'GET', respond(data, status = 200) { resolve({ ok: status < 400, status, statusText: '', text: async () => JSON.stringify(data) }); } };
    calls.push(entry);
    const w = waiters.get(calls.length - 1);
    if (w) w(entry);
  });
  return { calls, call(i) { return i < calls.length ? Promise.resolve(calls[i]) : new Promise((resolve) => waiters.set(i, resolve)); } };
}

// A unified diff shaped like the daemon's (crates/aegis-daemon/src/diffview.rs), with a redacted value.
const start = rndInt(5, 400);
const DIFF = [
  'diff --git a/src/report.rs b/src/report.rs',
  '--- a/src/report.rs',
  '+++ b/src/report.rs',
  `@@ -${start},3 +${start},3 @@`,
  ' pub fn render(lines: &[Line]) -> String {',
  '-    format!("{}", format_cents(order_amount(lines), CURRENCY))',
  '+    format!("{}", format_cents(order_amount(lines)))',
  ' }',
  'diff --git a/config/app.toml b/config/app.toml',
  '--- a/config/app.toml',
  '+++ b/config/app.toml',
  '@@ -1,1 +1,2 @@',
  ' [upstream]',
  '+[REDACTED]"',
  '',
].join('\n');

const viewOf = (textValue, extra = {}) => ({
  format: 'unified', text: textValue, bytes: textValue.length, bytes_total: textValue.length, cap_bytes: rndInt(1000, 50000),
  truncated: false, files: 2, files_total: 2, files_omitted: 0, redactions: 1, ...extra,
});

// ---- diff viewer ------------------------------------------------------------------------------

test('the Rust tokenizer marks keywords, strings, macros, types, numbers and [REDACTED]', () => {
  const tokens = diffview.tokenizeRust('pub fn total(x: u64) -> String { let s = "[REDACTED]"; format!("{}", 42) } // note');
  const of = (t) => tokens.filter((k) => k.t === t).map((k) => k.v);
  assert.deepEqual(of('kw'), ['pub', 'fn', 'let']);
  assert.ok(of('ty').includes('u64') && of('ty').includes('String'));
  assert.deepEqual(of('mac'), ['format!']);
  assert.deepEqual(of('red'), ['[REDACTED]']);
  assert.deepEqual(of('num'), ['42']);
  assert.deepEqual(of('com'), ['// note']);
  assert.deepEqual(of('fn'), ['total']);
  assert.equal(tokens.map((k) => k.v).join(''), 'pub fn total(x: u64) -> String { let s = "[REDACTED]"; format!("{}", 42) } // note', 'tokens cover the line');
  const toml = diffview.tokenizeToml('api_key = "[REDACTED]" # c');
  assert.deepEqual(toml.filter((k) => k.t === 'red').map((k) => k.v), ['[REDACTED]']);
  assert.equal(toml.find((k) => k.t === 'key').v, 'api_key');
});

test('the unified diff parser numbers old and new lines from the hunk header and counts per file', () => {
  const { rows, files } = diffview.parseUnifiedDiff(DIFF);
  assert.deepEqual(files.map((f) => [f.path, f.added, f.removed]), [['src/report.rs', 1, 1], ['config/app.toml', 1, 0]]);
  const del = rows.find((r) => r.kind === 'del');
  const add = rows.find((r) => r.kind === 'add');
  assert.equal(del.oldNo, start + 1);
  assert.equal(add.newNo, start + 1);
  assert.equal(rows.filter((r) => r.kind === 'file').length, 2);
  assert.equal(diffview.languageFor('src/report.rs'), 'rust');
  assert.equal(diffview.languageFor('config/app.toml'), 'toml');
});

test('the cap note reports the daemon cap, the bytes shown and the redactions', () => {
  const cap = rndInt(1024, 64 * 1024);
  const total = cap + rndInt(1, 100000);
  const note = diffview.capNote({ truncated: true, bytes: cap, bytes_total: total, cap_bytes: cap, files: 3, files_total: 9, redactions: 2 });
  assert.match(note, /cut at the daemon's .* cap/);
  assert.match(note, /3 of 9 files/);
  assert.match(note, /2 values redacted/);
  assert.equal(diffview.capNote({ truncated: false, redactions: 0 }), '');
});

// ---- run cards: branch diff and self-heal patch ----------------------------------------------

function workspacePanel() {
  const doc = installFakeDom();
  doc.body.append(el('section', { id: 'panel-workspace' },
    ...roles('error', 'summary', 'scenarios', 'action-status', 'pool:dl', 'reaper:dl', 'runs', 'runs-empty:p', 'eventlog:ol', 'reset:button')));
  const panel = workspaceMod.createWorkspacePanel();
  const card = () => doc.querySelector('[data-role="runs"] article');
  return { panel, card, doc };
}

let seq = 0;
const msg = (type, run_id, fields) => ({ type, run_id, ts: new Date().toISOString(), seq: ++seq, ...fields });

test('a run card shows the branch diff from the diff stage, highlighted, and keeps the diff out of the stage tile', () => {
  const { panel, card } = workspacePanel();
  seq = 0;
  panel.onMessage(msg('run.started', 'r-000021', { scenario: 'safe_refactor', title: 'Safe refactor' }));
  panel.onMessage(msg('run.stage', 'r-000021', { stage: 'diff', status: 'ok', ms: rnd(), detail: { files_changed: 2, unified_diff: viewOf(DIFF) } }));
  const block = card().querySelector('details.diffview');
  assert.ok(block, 'a collapsible diff block');
  assert.match(words(block.querySelector('summary')), /^▸ Branch diff 2 files \+2 −1 1 value redacted$/);
  assert.equal(block.querySelector('.dl'), null, 'lines are rendered when the block is opened');
  block.setAttribute('open', '');
  block.dispatchEvent(new (class { constructor() { this.type = 'toggle'; } })());
  assert.ok(block.querySelectorAll('.dl--add').length === 2 && block.querySelectorAll('.dl--del').length === 1);
  assert.ok(block.querySelector('.tok-red'), '[REDACTED] is a pill');
  assert.ok(block.querySelector('.tok-mac'), 'Rust is highlighted');
  const tile = card().querySelector('.stage .kv--stage');
  assert.doesNotMatch(text(card().querySelector('.pipeline')), /diff --git/, 'the stage tile does not dump the diff');
  assert.ok(tile);
  // The open state survives the re-render caused by the next stage.
  panel.onMessage(msg('run.stage', 'r-000021', { stage: 'features', status: 'ok', ms: rnd(), detail: {} }));
  assert.ok(card().querySelector('details.diffview').hasAttribute('open'));
});

test('a truncated diff says it was cut by the daemon cap', () => {
  const { panel, card } = workspacePanel();
  seq = 0;
  const cap = rndInt(1000, 30000);
  panel.onMessage(msg('run.started', 'r-000022', { scenario: 'mass_delete', title: 'Mass delete' }));
  panel.onMessage(msg('run.stage', 'r-000022', { stage: 'diff', status: 'ok', ms: rnd(), detail: { unified_diff: viewOf(DIFF, { truncated: true, cap_bytes: cap, bytes: cap, bytes_total: cap * 3, files: 1, files_total: 40 }) } }));
  const block = card().querySelector('details.diffview');
  assert.match(text(block.querySelector('.diffview__note')), /Truncated by the daemon: showing .* cut at the daemon's .* cap · 1 of 40 files/);
});

test('a recover stage that committed a patch shows the self-heal patch diff with its tier and model', () => {
  const { panel, card } = workspacePanel();
  seq = 0;
  const model = `synthetic/model-${rndInt(1, 999)}`;
  const patch = ['diff --git a/src/report.rs b/src/report.rs', '--- a/src/report.rs', '+++ b/src/report.rs', '@@ -2,1 +2,1 @@',
    '-    format_cents(order_amount(lines), CURRENCY)', '+    format_cents(order_amount(lines))', ''].join('\n');
  panel.onMessage(msg('run.started', 'r-000023', { scenario: 'build_break', title: 'Build break' }));
  panel.onMessage(msg('run.stage', 'r-000023', { stage: 'diff', status: 'ok', ms: rnd(), detail: { unified_diff: viewOf(DIFF) } }));
  panel.onMessage(msg('run.stage', 'r-000023', { stage: 'verify', status: 'fail', ms: rnd(), detail: { errors: 1 } }));
  panel.onMessage(msg('run.stage', 'r-000023', { stage: 'recover', status: 'ok', ms: rnd(), detail: { tier: 0, enabled: true, patch_applied: false } }));
  panel.onMessage(msg('run.stage', 'r-000023', { stage: 'recover', status: 'ok', ms: rnd(), detail: {
    tier: 1, model, enabled: true, patch_applied: true, committed_to_branch: true, patch_diff: viewOf(patch, { files: 1, files_total: 1, redactions: 0 }) } }));
  const blocks = card().querySelectorAll('details.diffview');
  assert.equal(blocks.length, 2, 'branch diff and self-heal patch');
  assert.equal(words(blocks[1].querySelector('summary')), `▸ Self-heal patch (repaired files: before → after) 1 file +1 −1 tier 1 · ${model}`);
  assert.doesNotMatch(text(card().querySelector('.pipeline')), /diff --git/);
});

test('build_break without remote tiers renders a clean "no patch" note with the daemon reason', () => {
  const { panel, card } = workspacePanel();
  seq = 0;
  const reason = `NEBIUS_API_KEY not set ${rndInt(1, 9)}`;
  panel.onMessage(msg('run.started', 'r-000024', { scenario: 'build_break', title: 'Build break' }));
  panel.onMessage(msg('run.stage', 'r-000024', { stage: 'recover', status: 'ok', ms: rnd(), detail: { tier: 0, enabled: true } }));
  panel.onMessage(msg('run.stage', 'r-000024', { stage: 'recover', status: 'skip', ms: null, detail: { tier: 1, enabled: false, reason, model: null } }));
  panel.onMessage(msg('run.stage', 'r-000024', { stage: 'recover', status: 'skip', ms: null, detail: { tier: 2, enabled: false, reason, model: null } }));
  panel.onMessage(msg('run.finished', 'r-000024', { outcome: 'rolled_back', total_ms: rnd(), queued_ms: rnd() }));
  const none = card().querySelector('[data-role="patch-none"]');
  assert.ok(none);
  assert.equal(words(none), `Self-heal patch –none tier 0 triage only; remote tiers disabled: ${reason}`);
  assert.equal(workspaceMod.noPatchReason([], false), 'waiting for the recovery tiers…');
});

// ---- PayPal: crypto pills, inspect column, receipt popover -----------------------------------

test('the receipt cell is a crypto pill with the scheme, a truncated signature and a copy button', () => {
  const body = Array.from({ length: 86 }, () => 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_'[rndInt(0, 63)]).join('');
  const signature = `c2pa:ed25519:${body}`;
  const [col] = paypalMod.decorate([{ field: 'receipt_verified', headerName: 'Receipt verified' }]);
  const cell = col.cellRenderer({ value: true, data: { signature, reference: 'agent-0042' } });
  assert.equal(text(cell.querySelector('.crypto__scheme')), 'c2pa:ed25519');
  assert.equal(text(cell.querySelector('.crypto__body')), `${body.slice(0, 6)}…${body.slice(-4)}`);
  assert.equal(cell.querySelector('button').getAttribute('aria-label'), 'Copy receipt signature of agent-0042');
  assert.ok(cell.querySelector('.crypto--ok'));
  const bad = col.cellRenderer({ value: false, data: { signature, reference: 'x' } });
  assert.ok(bad.querySelector('.crypto--fail'));
  assert.match(text(bad), /not verified/);
});

test('the redactions cell is a [REDACTED] pill with the count', () => {
  const n = rndInt(1, 9);
  const [col] = paypalMod.decorate([{ field: 'redactions', headerName: 'Redactions' }]);
  const cell = col.cellRenderer({ value: n, data: { unformatted_sensitive_fields: false } });
  assert.equal(text(cell.querySelector('.redacted')), `[REDACTED]×${dom.fmtNum(n)}`);
  const none = col.cellRenderer({ value: 0, data: { unformatted_sensitive_fields: true } });
  assert.equal(none.querySelector('.redacted'), null);
  assert.match(text(none), /unformatted secret/);
});

test('columns put the decision and its evidence first, and an Inspect column opens the receipt', () => {
  const fields = ['timestamp', 'reference', 'merchant', 'amount', 'currency', 'risk_score', 'decision', 'policies', 'receipt_verified', 'redactions', 'submission', 'total_us', 'submit_us', 'extra'];
  const ordered = paypalMod.orderColumns(fields.map((field) => ({ field })));
  assert.deepEqual(ordered.map((c) => c.field).slice(5, 8), ['decision', 'receipt_verified', 'redactions']);
  assert.deepEqual(ordered.map((c) => c.field).slice(-3), ['merchant', 'policies', 'extra']);
  const opened = [];
  const col = paypalMod.inspectColumn((row, trigger) => opened.push([row.id, trigger]));
  const btn = col.cellRenderer({ data: { id: 'agent-0001-x', reference: 'agent-0001' } });
  assert.equal(btn.getAttribute('aria-label'), 'Inspect signed receipt JSON of agent-0001');
  btn.click();
  assert.deepEqual(opened.map(([id]) => id), ['agent-0001-x']);
  assert.equal(col.suppressKeyboardEvent({ event: { key: 'Enter' } }), true);
});

test('Inspect fetches GET /api/paypal/orders/{id}/receipt and renders the JSON with pills', async () => {
  const fetch = stubFetch();
  const doc = installFakeDom();
  doc.body.append(
    el('section', { id: 'panel-paypal' }, ...roles('error:p', 'sandbox', 'scenarios', 'action-status:p', 'stream:form', 'stream-toggle:button',
      'stream-interval:input', 'stream-apply:button', 'grid', 'row-count:span', 'timeline', 'timeline-table', 'timeline-note:span', 'scale-toggle:button')),
    el('dialog', { id: 'receipt-dialog' }, el('h2'), ...roles('meta:p', 'status:p', 'body:pre', 'copy:button', 'close:button')));
  const panel = paypalMod.createPaypalPanel();
  const id = `agent-0007-${rndInt(100000, 999999)}`;
  const shown = panel.inspect({ id, reference: 'agent-0007' }, null);
  const req = await fetch.call(0);
  assert.equal(req.url, `/api/paypal/orders/${id}/receipt`);
  assert.equal(req.method, 'GET');
  const signature = `c2pa:ed25519:${rndInt(1e6, 9e6)}`;
  req.respond({ order_id: id, reference: 'agent-0007', receipt: { scheme: 'c2pa:ed25519', signature, payload_sha256: 'ab', verified: true },
    redactions: 2, display_redactions: 0, body: { purchase_units: [{ description: 'note [REDACTED]', custom_id: signature }] } });
  await shown;
  const dialog = doc.getElementById('receipt-dialog');
  assert.ok(dialog.hasAttribute('open'));
  assert.match(text(dialog.querySelector('h2')), /Signed receipt · agent-0007/);
  assert.match(text(dialog.querySelector('[data-role="meta"]')), /signature verified/);
  const body = dialog.querySelector('[data-role="body"]');
  assert.ok(body.querySelectorAll('.tok-red').length === 1, '[REDACTED] pill');
  assert.equal(body.querySelectorAll('.tok-sig').filter((n) => text(n).includes(signature)).length, 2, 'signature pills');
  assert.match(text(body), /"custom_id"/);
});

// ---- scoreboard: warm vs cold bars -------------------------------------------------------------

test('the latency bars pair warm-hit and cold-clonefile rows per fixture and print both values', () => {
  const doc = installFakeDom();
  const table = (role) => el('table', { 'data-role': role }, el('tbody'));
  doc.body.append(el('section', { id: 'panel-approvals' },
    ...roles('bench-error:p', 'bench-meta', 'bench-note:p', 'reload-bench:button', 'counters:dl', 'live-updated:span', 'bench-bars'),
    table('scoreboard'), table('livestats')));
  const board = scoreboardMod.createScoreboard();
  const warm = rnd();
  const cold = warm + rnd();
  const bench = {
    targets_summary: [{ metric: 'branch acquire (WarmBranchPool warm hit) · tiny (9 files)', fixture: 'tiny', measured_p50_ms: warm, pass: true, section: 'apfs' }],
    informational: [
      { metric: 'cold clonefile branch · tiny (9 files)', fixture: 'tiny', measured_p50_ms: cold, pass: true, section: 'apfs' },
      { metric: 'cold clonefile branch · lonely (1 file)', fixture: 'lonely', measured_p50_ms: cold, pass: true, section: 'apfs' },
    ],
  };
  const pairs = scoreboardMod.latencyPairs(bench);
  assert.deepEqual(pairs.map((p) => [p.fixture, p.label, p.warm.p50, p.cold.p50]), [['tiny', 'tiny (9 files)', warm, cold]]);
  board.setTelemetry({ bench_path: 'x.json', bench });
  const bars = doc.querySelector('[data-role="bench-bars"]');
  const vals = bars.querySelectorAll('.bar__val').map(text);
  assert.deepEqual(vals, [dom.fmtDur(warm), dom.fmtDur(cold)]);
  assert.equal(text(bars.querySelector('.bar-group__ratio')), scoreboardMod.ratioText(warm, cold));
  assert.match(scoreboardMod.ratioText(warm, cold), /^warm .*× faster$/);
  // The scoreboard rows and the banner logic are unchanged.
  assert.match(text(doc.querySelector('[data-role="bench-meta"]')), /1 of 1 targets met/);
});

// ---- header hardware strip ---------------------------------------------------------------------

function hwFixture() {
  const doc = installFakeDom();
  for (const id of ['hw-ane', 'hw-pool', 'hw-cedar', 'hw-ai', 'hw-paypal']) {
    doc.body.append(el('span', { id }, el('span', { 'data-role': 'value' }), el('span', { 'data-role': 'source' })));
  }
  const v = (id) => text(doc.getElementById(id).querySelector('[data-role="value"]'));
  const src = (id) => text(doc.getElementById(id).querySelector('[data-role="source"]'));
  return { doc, v, src, cockpit: cockpitMod.createCockpit() };
}

test('the ANE pill is built from MLComputePlan and the live p50, falling back to the bench value with its label', () => {
  const { doc, v, src, cockpit } = hwFixture();
  const ops = rndInt(8, 64);
  const benchMs = rnd();
  cockpit.setHealth({ model: { compute_plan: { neural_engine: ops, cpu: 0, gpu: 0 } } });
  cockpit.setTelemetry({ bench: { targets_summary: [{ section: 'ane', metric: 'online model', measured_p50_ms: benchMs, pass: true }] }, live: { samples: { ane_score_ms: { n: 0, p50: null } } } });
  assert.equal(v('hw-ane'), `${ops}/${ops} OPS · ${dom.fmtDur(benchMs)}`);
  assert.equal(src('hw-ane'), 'bench p50');
  const liveMs = rnd();
  cockpit.setLive({ samples: { ane_score_ms: { n: rndInt(1, 99), p50: liveMs } } });
  assert.equal(v('hw-ane'), `${ops}/${ops} OPS · ${dom.fmtDur(liveMs)}`);
  assert.equal(src('hw-ane'), 'live p50');
  assert.equal(doc.getElementById('hw-ane').dataset.pulse, 'off', 'no pulse while disconnected');
  cockpit.setConnected(true);
  assert.equal(doc.getElementById('hw-ane').dataset.pulse, 'on');
  cockpit.setHealth({ model: { compute_plan: { neural_engine: 0, cpu: ops, gpu: 0 } } });
  assert.equal(doc.getElementById('hw-ane').dataset.pulse, 'off', 'no pulse when the model is not on the ANE');
});

test('the pool, Cedar, Nemotron and PayPal pills come from /api/workspace, live stats and /api/config', () => {
  const { v, src, cockpit } = hwFixture();
  const cap = rndInt(1, 8);
  const ready = rndInt(0, cap);
  const warm = rnd();
  cockpit.setTelemetry({ bench: { targets_summary: [{ metric: 'branch acquire (WarmBranchPool warm hit) · tiny', fixture: 'tiny', measured_p50_ms: warm }] } });
  cockpit.setWorkspace({ pool: { ready, capacity: cap } });
  assert.equal(v('hw-pool'), `${ready}/${cap} ready · ${dom.fmtDur(warm)}`);
  assert.equal(src('hw-pool'), 'bench · tiny');
  const cedarUs = rnd() * 100;
  cockpit.setLive({ samples: { cedar_eval_us: { n: 3, p50: cedarUs } } });
  assert.equal(v('hw-cedar'), dom.fmtDur(cedarUs / 1000));
  cockpit.setServices({ nebius: { enabled: false, provider: 'Provider X', reason: 'NEBIUS_API_KEY not set' }, tavily: { enabled: true }, paypal: { mode: 'local' } });
  assert.equal(v('hw-ai'), 'Provider X · off · Tavily on');
  assert.equal(v('hw-paypal'), 'local gateway');
});

// ---- hero signal path --------------------------------------------------------------------------

test('workspace stages map onto the five nodes with their measured values and outcome colours', () => {
  const branch = rnd();
  const evalUs = rnd() * 100;
  const swap = rnd();
  const { nodes } = pipelineMod.workspaceNodes([
    { stage: 'branch', status: 'ok', ms: branch, detail: { source: 'pool' } },
    { stage: 'score', status: 'ok', ms: rnd(), detail: { risk: rnd() * 100, units: 'cpuAndNeuralEngine' } },
    { stage: 'policy', status: 'ok', ms: rnd(), detail: { allowed: true, determining_policies: ['permit-low-risk'], eval_us: evalUs } },
    { stage: 'commit', status: 'ok', ms: swap, detail: {} },
  ], 'committed');
  assert.deepEqual(nodes.map((n) => n.state), ['ok', 'ai', 'ok', 'ok', 'skip']);
  assert.equal(nodes[0].ms, branch);
  assert.equal(nodes[2].ms, evalUs / 1000, 'Cedar shows the evaluation time');
  assert.equal(nodes[3].ms, swap);
  assert.equal(nodes[4].sub, 'no build failure: not needed', 'stage 5 lights only on recover stages');
  const held = pipelineMod.workspaceNodes([{ stage: 'policy', status: 'fail', ms: rnd(), detail: { allowed: false } }, { stage: 'approval', status: 'pending', ms: null, detail: {} }], 'pending_approval').nodes;
  assert.equal(held[2].state, 'hold');
  assert.equal(held[3].state, 'hold');
  const healed = pipelineMod.workspaceNodes([{ stage: 'recover', status: 'ok', ms: rnd(), detail: { tier: 0 } }, { stage: 'rollback', status: 'ok', ms: rnd(), detail: {} }], 'rolled_back').nodes;
  assert.deepEqual([healed[3].state, healed[4].state], ['fail', 'ai']);
});

test('the hero lights from live run events, idles on live p50, and maps a PayPal order', async () => {
  const doc = installFakeDom();
  doc.body.append(el('section', { id: 'hero' }, el('p', { 'data-role': 'context' }), el('div', { 'data-role': 'path' }), el('p', { 'data-role': 'caption' }),
    el('button', { 'data-mode': 'workspace' }), el('button', { 'data-mode': 'paypal' })));
  const hero = pipelineMod.createPipeline();
  const nodes = () => doc.querySelectorAll('.node');
  const value = (i) => text(nodes()[i].querySelector('.node__val'));
  const acquire = rnd();
  hero.setLive({ samples: { branch_acquire_ms: { n: 4, p50: acquire } } });
  assert.equal(value(0), dom.fmtDur(acquire).replace(' ', ''));
  assert.equal(text(nodes()[0].querySelector('.node__src')), 'live p50');
  assert.equal(value(1), dom.DASH, 'no samples: an em dash');

  const branch = rnd();
  hero.onRunMessage({ type: 'run.started', run_id: 'r-000031', scenario: 'safe_refactor', title: 'Safe refactor', seq: 1 });
  hero.onRunMessage({ type: 'run.stage', run_id: 'r-000031', stage: 'branch', status: 'ok', ms: branch, detail: { source: 'cold' }, seq: 2 });
  assert.equal(value(0), dom.fmtDur(branch).replace(' ', ''));
  assert.equal(nodes()[0].dataset.state, 'ok');
  assert.equal(nodes()[1].dataset.state, 'waiting');
  assert.match(text(doc.querySelector('[data-role="context"]')), /r-000031 · Safe refactor · streaming/);

  const row = { reference: 'agent-0009', amount: '42.00', currency: 'USD', decision: 'Deny', policies: '', redactions: 2, receipt_verified: true,
    submission: 'blocked', redact_us: rnd() * 100, score_us: rnd() * 100, cedar_us: rnd() * 100, sign_us: rnd() * 100, verify_us: rnd() * 100, submit_us: null, total_us: rnd() * 1000 };
  hero.setOrders({ rows: [row] }); // a reload: shown without animation once the PayPal view is selected
  doc.querySelectorAll('[data-mode="paypal"]')[0].click();
  assert.equal(value(2), dom.fmtDur(row.cedar_us / 1000).replace(' ', ''));
  assert.equal(nodes()[2].dataset.state, 'fail', 'Cedar deny is crimson');
  assert.equal(value(3), dom.fmtDur((row.sign_us + row.verify_us) / 1000).replace(' ', ''));
  assert.equal(nodes()[4].dataset.state, 'fail');
  assert.match(text(doc.querySelector('[data-role="caption"]')), /redact → ANE score → Cedar → ed25519 sign \+ verify → submit/);
  await drain();
});
