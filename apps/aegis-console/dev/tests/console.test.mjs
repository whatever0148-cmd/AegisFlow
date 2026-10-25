// Regression tests for the Aegis Console panels (development only; the daemon never serves dev/).
//
//   node --test apps/aegis-console/dev/tests/console.test.mjs
//
// The real assets/*.js modules run against the minimal DOM in fake-dom.mjs, with fetch stubbed.
// Numeric inputs are synthetic (Math.random) and every expectation is computed with the console's
// own formatters, so no latency or score is typed in here. Waits use explicit barriers (a promise
// resolved by the stubbed fetch or by a panel callback) and node:test mock timers, never sleeps.

import { test, mock } from 'node:test';
import assert from 'node:assert/strict';
import { installFakeDom, FakeEvent, el, roles } from './fake-dom.mjs';

// The orders grid labels its timestamp column UTC; run in a zone where local time differs from UTC.
process.env.TZ = 'America/Los_Angeles';

installFakeDom();
const dom = await import('../../assets/dom.js');
const api = await import('../../assets/api.js');
const workspaceMod = await import('../../assets/workspace.js');
const paypalMod = await import('../../assets/paypal.js');
const scoreboardMod = await import('../../assets/scoreboard.js');
const settingsMod = await import('../../assets/settings.js');

/** Lets every queued microtask (promise continuation) run; not a timed wait. */
const drain = () => new Promise((resolve) => setImmediate(resolve));

/** fetch stub: records calls; call(i) resolves once the i-th request has been made. */
function stubFetch() {
  const calls = [];
  const waiters = new Map();
  globalThis.fetch = (url, init = {}) => new Promise((resolve) => {
    const entry = {
      url,
      method: init.method || 'GET',
      body: init.body ? JSON.parse(init.body) : null,
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

const text = (node) => (node ? node.textContent.replace(/\s+/g, ' ').trim() : '');

// ---- workspace runs panel (frontend-1) -------------------------------------------------------

function workspacePanel() {
  const doc = installFakeDom();
  doc.body.append(el('section', { id: 'panel-workspace' },
    ...roles('error', 'summary', 'scenarios', 'action-status', 'pool:dl', 'reaper:dl', 'runs', 'runs-empty:p', 'eventlog:ol', 'reset:button')));
  const panel = workspaceMod.createWorkspacePanel();
  const runsEl = doc.querySelector('[data-role="runs"]');
  const slots = (card) => Object.fromEntries(card.querySelectorAll('li').map((li) => [
    text(li.querySelector('.stage__name')), text(li.querySelector('.stage__head .badge__label'))]));
  const card = () => runsEl.querySelector('article');
  return { panel, runsEl, card, slots };
}

const stage = (run_id, name, status) => ({ type: 'run.stage', run_id, stage: name, status, ms: null, detail: {}, ts: new Date().toISOString() });

test('a run first seen after page load never claims its earlier stages did not run', () => {
  const { panel, card, slots } = workspacePanel();
  // What a page loaded after mass_delete reached pending approval receives when the workspace is reset.
  panel.onMessage(stage('r-000008', 'approval', 'skip'));
  panel.onMessage(stage('r-000008', 'rollback', 'ok'));
  panel.onMessage({ type: 'run.finished', run_id: 'r-000008', outcome: 'rolled_back', total_ms: null, ts: new Date().toISOString() });

  const s = slots(card());
  assert.ok(!Object.values(s).includes('not run'), `no slot may read "not run": ${JSON.stringify(s)}`);
  for (const name of ['Branch', 'Edit', 'Diff', 'Features', 'Score', 'Policy', 'Verify / recover', 'Reap']) {
    assert.equal(s[name], 'not observed', name);
  }
  assert.equal(s['Approval / commit / rollback'], 'OK');
  assert.match(text(card().querySelector('.run__note')), /did not see this run start/);
  assert.equal(text(card().querySelector('.run__title')), 'Run r-000008');
  assert.equal(text(card().querySelector('.run__meta')), 'scenario not observed');
});

test('a partial run takes its title from the approval for the same run_id', () => {
  const { panel, card } = workspacePanel();
  panel.onMessage(stage('r-000009', 'approval', 'ok'));
  panel.onMessage({ type: 'approval.resolved', approval: { id: 'a-000002', run_id: 'r-000009', title: 'Mass delete in demo workspace', status: 'approved' }, ts: new Date().toISOString() });
  assert.equal(text(card().querySelector('.run__title')), 'Mass delete in demo workspace');
});

test('a fully observed run keeps "not run" for stages that did not run, with no partial note', () => {
  const { panel, card, slots } = workspacePanel();
  panel.onMessage({ type: 'run.started', run_id: 'r-000001', scenario: 'safe_refactor', title: 'Safe refactor', ts: new Date().toISOString() });
  for (const name of ['branch', 'edit', 'diff', 'features', 'score', 'policy', 'verify', 'commit']) panel.onMessage(stage('r-000001', name, 'ok'));
  panel.onMessage({ type: 'run.finished', run_id: 'r-000001', outcome: 'committed', total_ms: null, ts: new Date().toISOString() });
  const s = slots(card());
  assert.equal(s.Branch, 'OK');
  assert.equal(s.Reap, 'not run');
  assert.equal(card().querySelector('.run__note'), null);
  assert.equal(text(card().querySelector('.run__head .badge__label')), 'Committed');
});

test('a stream gap marks open runs partial and their outcome unknown until run.finished arrives', () => {
  const { panel, card, slots } = workspacePanel();
  panel.onMessage({ type: 'run.started', run_id: 'r-000002', scenario: 'build_break', title: 'Build break', ts: new Date().toISOString() });
  panel.onMessage(stage('r-000002', 'branch', 'ok'));
  assert.equal(slots(card()).Edit, 'waiting');
  panel.markGap('disconnect');
  assert.equal(text(card().querySelector('.run__head .badge__label')), 'outcome not observed');
  assert.equal(slots(card()).Edit, 'not observed');
  assert.match(text(card().querySelector('.run__note')), /live stream disconnected/);

  panel.onMessage({ type: 'run.finished', run_id: 'r-000002', outcome: 'rolled_back', total_ms: null, ts: new Date().toISOString() });
  assert.equal(text(card().querySelector('.run__head .badge__label')), 'Rolled back');
  assert.equal(slots(card())['Verify / recover'], 'not observed');
  assert.match(text(card().querySelector('.run__note')), /outcome may be out of date/);
});

test('a run first seen through its post-finish reap does not read "running"', () => {
  const { panel, card } = workspacePanel();
  panel.onMessage(stage('r-000003', 'reap', 'ok'));
  assert.equal(text(card().querySelector('.run__head .badge__label')), 'outcome not observed');
});

test('the daemon lag notice is recognised by its dropped field, with the text as a fallback; other log lines are not', () => {
  const dropped = 1 + Math.floor(Math.random() * 50);
  const textOnly = `this client fell behind; ${dropped} messages were dropped`;
  assert.ok(api.isLagNotice({ type: 'log', level: 'warn', message: textOnly, dropped }));
  assert.ok(api.isLagNotice({ type: 'log', level: 'warn', message: 'reworded by a newer daemon', dropped }), 'the field decides, not the text');
  assert.ok(!api.isLagNotice({ type: 'log', level: 'warn', message: textOnly, dropped: 0 }));
  assert.ok(api.isLagNotice({ type: 'log', level: 'warn', message: textOnly }), 'older daemons: text only');
  assert.ok(!api.isLagNotice({ type: 'run.stage', dropped }));
  assert.ok(!api.isLagNotice({ type: 'log', level: 'info', message: 'workspace reset' }));
  assert.ok(!api.isLagNotice({ type: 'run.stage', message: `fell behind; ${dropped} messages were dropped` }));
  assert.ok(!api.isLagNotice(null));
});

// ---- PayPal orders grid (frontend-3, frontend-4) ---------------------------------------------

test('the "Time (UTC)" column renders UTC, not the browser zone', () => {
  const iso = new Date(Date.UTC(2026, 9, 25, 13, 39, 33)).toISOString().replace('.000', '');
  const local = new Date(iso).toLocaleTimeString([], { hour12: false });
  const [col] = paypalMod.decorate([{ field: 'timestamp', headerName: 'Time (UTC)' }]);
  assert.equal(col.headerName, 'Time (UTC)');
  assert.equal(col.valueFormatter({ value: iso }), iso.slice(11, 19));
  assert.notEqual(local, iso.slice(11, 19), 'test zone must differ from UTC to be meaningful');
  const [bare] = paypalMod.decorate([{ field: 'timestamp', headerName: 'Time' }]);
  assert.equal(bare.headerName, 'Time (UTC)');
});

test('the Amount column sorts and filters numerically and displays the daemon string', () => {
  const cents = [4200, 12000, 48000, 2500];
  const rows = cents.map((c) => ({ amount_cents: c, amount: (c / 100).toFixed(2) }));
  const [col] = paypalMod.decorate([{ field: 'amount', headerName: 'Amount', sortable: true, filter: true, type: 'numericColumn' }]);
  assert.equal(col.filter, 'agNumberColumnFilter');
  assert.equal(col.cellDataType, 'number');
  // AG Grid's default comparator compares the column values with < and >, as here.
  const agDefault = (a, b) => (a > b ? 1 : a < b ? -1 : 0);
  assert.equal(typeof col.valueGetter({ data: rows[0] }), 'number');
  const sorted = rows.slice().sort((a, b) => agDefault(col.valueGetter({ data: a }), col.valueGetter({ data: b }))).map((r) => r.amount);
  assert.deepEqual(sorted, cents.slice().sort((a, b) => a - b).map((c) => (c / 100).toFixed(2)));
  assert.equal(col.filterValueGetter({ data: rows[1] }), cents[1] / 100);
  assert.equal(col.valueFormatter({ data: rows[1], value: col.valueGetter({ data: rows[1] }) }), rows[1].amount);
  assert.equal(paypalMod.amountValue({ amount: rows[2].amount }), cents[2] / 100, 'falls back to parsing amount');
});

// ---- PayPal Sandbox status (frontend-2) -------------------------------------------------------

function paypalPanel() {
  const doc = installFakeDom();
  doc.body.append(el('section', { id: 'panel-paypal' },
    ...roles('error:p', 'sandbox', 'scenarios', 'action-status:p', 'stream:form', 'stream-toggle:button', 'stream-interval:input',
      'stream-apply:button', 'grid', 'row-count:span', 'timeline', 'timeline-table', 'timeline-note:span', 'scale-toggle:button')));
  const panel = paypalMod.createPaypalPanel();
  return { panel, sandbox: doc.querySelector('[data-role="sandbox"]') };
}

const scenariosBody = (enabled, submitted) => ({ scenarios: [], stream: { enabled: false, interval_ms: 60000 }, sandbox: { enabled, submitted, cap: 25 } });
const paypalService = (enabled) => ({ paypal: enabled
  ? { enabled: true, reason: null, base_url: 'https://api-m.sandbox.paypal.com' }
  : { enabled: false, reason: 'PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set', base_url: 'https://api-m.sandbox.paypal.com' } });

test('saving PayPal credentials re-reads the Sandbox state instead of showing the stale badge', async () => {
  const fetch = stubFetch();
  const { panel, sandbox } = paypalPanel();
  panel.setScenarios(scenariosBody(false, 0));
  panel.setServices(paypalService(false));
  assert.match(text(sandbox), /Sandbox disabled/);
  assert.equal(fetch.calls.length, 0, 'no refetch while both sources agree');

  panel.setServices(paypalService(true)); // config.changed after POST /api/config
  assert.doesNotMatch(text(sandbox), /Sandbox disabled/);
  assert.match(text(sandbox), /refreshing/);
  const req = await fetch.call(0);
  assert.equal(req.url, '/api/paypal/scenarios');
  req.respond(scenariosBody(true, 0));
  await drain();
  // Without `mode` (older daemons), enabled:true meant the remote Sandbox.
  assert.match(text(sandbox), /Sandbox Active \(PayPal Remote\)/);

  panel.setServices(paypalService(false)); // credentials cleared
  assert.doesNotMatch(text(sandbox), /Sandbox Active/);
  (await fetch.call(1)).respond(scenariosBody(false, 0));
  await drain();
  assert.match(text(sandbox), /Sandbox disabled/);
});

test('a failed Sandbox submission refreshes the submitted counter; unsent orders do not', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const fetch = stubFetch();
  const { panel } = paypalPanel();
  panel.setScenarios(scenariosBody(true, 0));
  panel.setServices(paypalService(true));

  for (const submission of ['not_sent', 'blocked']) {
    panel.onMessage({ type: 'paypal.order', row: { id: `o-${submission}`, reference: submission, submission }, events: [] });
  }
  t.mock.timers.tick(1000);
  assert.equal(fetch.calls.length, 0);

  panel.onMessage({ type: 'paypal.order', row: { id: 'o-error', reference: 'error', submission: 'error' }, events: [] });
  t.mock.timers.tick(1000);
  const req = await fetch.call(0);
  assert.equal(req.url, '/api/paypal/scenarios');
});

// ---- scoreboard (frontend-5, frontend-6) ------------------------------------------------------

function scoreboardPanel() {
  const doc = installFakeDom();
  const table = (role) => el('table', { 'data-role': role }, el('tbody'));
  doc.body.append(el('section', { id: 'panel-approvals' },
    ...roles('bench-error:p', 'bench-meta', 'bench-note:p', 'reload-bench:button', 'counters:dl', 'live-updated:span'),
    table('scoreboard'), table('livestats')));
  return { board: scoreboardMod.createScoreboard(), doc };
}

test('a gated row with pass:null is a red FAIL counted as missed', () => {
  const { board, doc } = scoreboardPanel();
  const measuredMs = Math.random();
  board.setTelemetry({
    bench_path: 'artifacts/aegis_phase1_telemetry.json',
    bench: {
      targets_summary: [
        { metric: 'measured row', target: '< limit', measured_p50_ms: measuredMs, pass: true },
        { metric: 'online model executes on the Neural Engine (MLComputePlan)', target: 'all ops on ANE', measured: { neural_engine: 0, cpu: 0, gpu: 0 }, dispatched_ops: 0, pass: null },
      ],
      informational: [],
    },
  });
  const summary = text(doc.querySelector('[data-role="bench-meta"]'));
  assert.match(summary, /1 of 2 targets met/);
  assert.match(summary, /1 missed \(1 not measured\)/);
  const rows = doc.querySelector('[data-role="scoreboard"]').tBodies[0].children;
  assert.equal(rows[0].className, 'row-pass');
  assert.equal(rows[1].className, 'row-fail');
  assert.match(text(rows[1].querySelector('.result')), /FAIL/);
  assert.match(text(rows[1].querySelector('.result')), /not measured/);
  assert.deepEqual(scoreboardMod.gatedCounts([{ pass: true }, { pass: false }, { pass: null }, {}]), { total: 4, passed: 1, missed: 1, unmeasured: 2 });
});

test('the Measured cell labels the median and shows p95 and hit/miss counts', () => {
  const p50 = Math.random();
  const p95 = p50 + Math.random();
  const hits = 1 + Math.floor(Math.random() * 100);
  const misses = Math.floor(Math.random() * 10);
  const parts = scoreboardMod.measuredParts({ metric: 'pool', target: '< limit', measured_p50_ms: p50, pass: true, fixture: 'small', p95_ms: p95, hits, misses });
  assert.equal(parts.primary, `p50 ${dom.withUnit(p50, 'ms')}`);
  assert.equal(parts.extras, `p95 ${dom.withUnit(p95, 'ms')} · hits ${dom.fmtNum(hits)} · misses ${dom.fmtNum(misses)}`);

  const ops = 1 + Math.floor(Math.random() * 64);
  const placement = scoreboardMod.measuredParts({ measured: { neural_engine: ops, cpu: 0, gpu: 0 }, dispatched_ops: ops, pass: true });
  assert.equal(placement.primary, `ANE ${ops} · CPU 0 · GPU 0`);
  assert.equal(placement.extras, `dispatched ops ${ops}`);

  const perEntry = Math.random();
  const sys = Math.random();
  const kernel = scoreboardMod.measuredParts({ measured_per_entry_us: perEntry, sys_us_p50: sys, entries: hits, pass: null });
  assert.equal(kernel.primary, `per entry ${dom.withUnit(perEntry, 'µs')}`);
  assert.equal(kernel.extras, `sys p50 ${dom.withUnit(sys, 'µs')} · entries ${dom.fmtNum(hits)}`);
});

test('a preliminary artifact shows a PRELIMINARY label with run_conditions.reasons; a decisive one does not', () => {
  const { board, doc } = scoreboardPanel();
  const reasons = [`reduced scale ${Math.random()}: iteration counts scaled down`, `load average unavailable at start ${Math.random()}`];
  const p50 = Math.random();
  const rows = [{ metric: 'pool acquire', target: '< limit', measured_p50_ms: p50, measured_hit_rate: Math.random(), p95_ms: p50 + Math.random(), n: 1 + Math.floor(Math.random() * 100), pass: false }];
  board.setTelemetry({ bench_path: 'x.json', bench: { preliminary: true, run_conditions: { preliminary: true, reasons }, targets_summary: rows, informational: [] } });
  const label = doc.querySelector('[data-role="bench-preliminary"]');
  assert.ok(label, 'PRELIMINARY label rendered');
  assert.match(text(label), /PRELIMINARY/);
  for (const r of reasons) assert.ok(text(label).includes(r), `reason shown: ${r}`);
  const parts = scoreboardMod.measuredParts(rows[0]);
  assert.match(parts.primary, /^p50 .*; hit rate \S/, 'measured_hit_rate is labelled "hit rate"');
  assert.doesNotMatch(parts.primary, /measured/);
  assert.ok(parts.extras.includes(`p95 ${dom.withUnit(rows[0].p95_ms, 'ms')}`), parts.extras);
  assert.ok(parts.extras.includes(`n ${dom.fmtNum(rows[0].n)}`), parts.extras);

  board.setTelemetry({ bench_path: 'x.json', bench: { preliminary: false, run_conditions: { preliminary: false, reasons: [] }, targets_summary: rows, informational: [] } });
  assert.equal(doc.querySelector('[data-role="bench-preliminary"]'), null);
  assert.equal(scoreboardMod.preliminaryInfo({ run_conditions: { preliminary: true } }).reasons.length, 0);
  assert.equal(scoreboardMod.preliminaryInfo({}), null);
});

// ---- settings drawer (frontend-7) -------------------------------------------------------------

function settingsDrawer(onConfig) {
  const doc = installFakeDom();
  const dialog = el('dialog', { id: 'settings' }, ...roles('error:p', 'path:span', 'form:form', 'keys', 'save:button', 'save-status:p', 'services', 'close:button'));
  doc.body.append(el('button', { id: 'open-settings' }), dialog);
  const drawer = settingsMod.createSettings({ onConfig });
  const config = {
    path: '/scratch/.env',
    keys: [{ name: 'TAVILY_API_KEY', set: true, source: 'dotenv', secret: true, hint: 'Tavily key', signup_url: null }],
    services: {},
  };
  drawer.setConfig(config);
  const $ = (sel) => dialog.querySelector(sel);
  const submit = () => $('[data-role="form"]').dispatchEvent(new FakeEvent('submit'));
  return { drawer, config, $, submit };
}

test('secret inputs are masked text fields that browsers and password managers leave alone', () => {
  const { $ } = settingsDrawer(null);
  const input = $('[data-key="TAVILY_API_KEY"]');
  assert.equal(input.getAttribute('type'), 'text', 'type=password makes Chrome offer generated or saved passwords');
  assert.equal(input.getAttribute('autocomplete'), 'off');
  assert.equal(input.getAttribute('class'), 'key__secret');
  assert.equal(input.dataset['1pIgnore'], 'true');
  assert.equal(input.dataset.lpignore, 'true');
  assert.notEqual(input.getAttribute('name'), 'TAVILY_API_KEY');
});

test('a whitespace-only value is refused instead of silently unsetting the saved key', () => {
  const fetch = stubFetch();
  const { $, submit } = settingsDrawer(null);
  $('[data-key="TAVILY_API_KEY"]').value = '   ';
  submit();
  assert.equal(fetch.calls.length, 0, 'nothing may be POSTed');
  const status = text($('[data-role="save-status"]'));
  assert.match(status, /Nothing saved: TAVILY_API_KEY contains only spaces/);
  assert.doesNotMatch(status, /Saved/);
});

test('a typed value and "Clear on save" still post as before', async () => {
  const fetch = stubFetch();
  let saved;
  let configs = 0;
  const { $, submit, config } = settingsDrawer(() => { configs += 1; if (configs > 1 && saved) saved(); });

  $('[data-key="TAVILY_API_KEY"]').value = 'replacement-value';
  let done = new Promise((resolve) => { saved = resolve; });
  submit();
  const req = await fetch.call(0);
  assert.equal(req.method, 'POST');
  assert.deepEqual(req.body, { values: { TAVILY_API_KEY: 'replacement-value' } });
  req.respond(config);
  await done;
  assert.match(text($('[data-role="save-status"]')), /Saved 1 key: TAVILY_API_KEY/);

  $('[data-clear="TAVILY_API_KEY"]').click();
  done = new Promise((resolve) => { saved = resolve; });
  submit();
  const clear = await fetch.call(1);
  assert.deepEqual(clear.body, { values: { TAVILY_API_KEY: '' } });
  clear.respond(config);
  await done;
});

test('a key with console_writable:false renders read-only, keeps "Clear on save" and is never posted with a value', async () => {
  const fetch = stubFetch();
  const doc = installFakeDom();
  const dialog = el('dialog', { id: 'settings' }, ...roles('error:p', 'path:span', 'form:form', 'keys', 'save:button', 'save-status:p', 'services', 'close:button'));
  doc.body.append(el('button', { id: 'open-settings' }), dialog);
  let saved;
  const drawer = settingsMod.createSettings({ onConfig: () => { if (saved) saved(); } });
  const config = {
    path: '/scratch/.env',
    keys: [
      { name: 'NEBIUS_BASE_URL', set: true, source: 'dotenv', secret: false, console_writable: false, hint: 'endpoint', signup_url: null },
      { name: 'TAVILY_API_KEY', set: false, source: null, secret: true, console_writable: true, hint: 'Tavily key', signup_url: null },
    ],
    services: {},
  };
  drawer.setConfig(config);
  const $ = (sel) => dialog.querySelector(sel);
  const base = $('[data-key="NEBIUS_BASE_URL"]');
  assert.ok(base.hasAttribute('readonly'), 'readonly attribute');
  assert.ok(base.hasAttribute('disabled'), 'disabled, so it cannot be typed into or submitted');
  assert.match(text(dialog), /read-only/);
  assert.ok(!$('[data-key="TAVILY_API_KEY"]').hasAttribute('readonly'), 'writable keys stay editable');
  assert.ok($('[data-clear="NEBIUS_BASE_URL"]'), '"Clear on save" stays available');

  $('[data-key="TAVILY_API_KEY"]').value = 'typed-value';
  let done = new Promise((resolve) => { saved = resolve; });
  $('[data-role="form"]').dispatchEvent(new FakeEvent('submit'));
  const req = await fetch.call(0);
  assert.deepEqual(req.body, { values: { TAVILY_API_KEY: 'typed-value' } }, 'the read-only key is not sent');
  req.respond(config);
  await done;

  $('[data-clear="NEBIUS_BASE_URL"]').click();
  done = new Promise((resolve) => { saved = resolve; });
  $('[data-role="form"]').dispatchEvent(new FakeEvent('submit'));
  const clear = await fetch.call(1);
  assert.deepEqual(clear.body, { values: { NEBIUS_BASE_URL: '' } });
  clear.respond(config);
  await done;
});

// Mirrors GET /api/config from aegis-daemon (services::config_json) for the Nemotron provider keys.
const PROVIDERS = [
  { id: 'nebius', label: 'Nebius Token Factory', base_url: 'https://api.tokenfactory.nebius.com/v1', signup_url: 'https://tokenfactory.nebius.com/', key_hint: 'Nebius Token Factory API key', tier1_model: 'nvidia/Llama-3_3-Nemotron-Super-49B-v1', tier2_model: 'nvidia/Llama-3_1-Nemotron-Ultra-253B-v1' },
  { id: 'nvidia-nim', label: 'NVIDIA NIM', base_url: 'https://integrate.api.nvidia.com/v1', signup_url: 'https://build.nvidia.com', key_hint: 'build.nvidia.com API key (nvapi-…)', tier1_model: null, tier2_model: 'nvidia/llama-3.1-nemotron-ultra-253b-v1' },
  { id: 'openrouter', label: 'OpenRouter', base_url: 'https://openrouter.ai/api/v1', signup_url: 'https://openrouter.ai/keys', key_hint: 'OpenRouter API key', tier1_model: null, tier2_model: null },
];

function providerConfig(selected) {
  return {
    path: '/scratch/.env',
    keys: [
      { name: 'NEBIUS_API_KEY', set: true, source: 'dotenv', secret: true, console_writable: true, hint: 'API key for the selected Nemotron provider', signup_url: selected.signup_url },
      { name: 'NEBIUS_BASE_URL', set: true, source: 'dotenv', secret: false, console_writable: true, hint: 'Nemotron provider', signup_url: null,
        choices: PROVIDERS.map((p) => ({ value: p.base_url, label: p.label })) },
      { name: 'NEBIUS_TIER1_MODEL', set: false, source: null, secret: false, console_writable: true, hint: 'Tier-1 model id', signup_url: null },
      { name: 'NEBIUS_TIER2_MODEL', set: false, source: null, secret: false, console_writable: true, hint: 'Tier-2 model id', signup_url: null },
    ],
    nemotron_providers: PROVIDERS,
    services: { nebius: { enabled: false, reason: 'synthetic', provider: selected.label, base_url: selected.base_url, tier1_model: selected.tier1_model, tier2_model: selected.tier2_model } },
  };
}

function providerDrawer(selected, onConfig) {
  const doc = installFakeDom();
  const dialog = el('dialog', { id: 'settings' }, ...roles('error:p', 'path:span', 'form:form', 'keys', 'save:button', 'save-status:p', 'services', 'close:button'));
  doc.body.append(el('button', { id: 'open-settings' }), dialog);
  const drawer = settingsMod.createSettings({ onConfig });
  const config = providerConfig(selected);
  drawer.setConfig(config);
  const $ = (sel) => dialog.querySelector(sel);
  const $$ = (sel) => dialog.querySelectorAll(sel);
  const submit = () => $('[data-role="form"]').dispatchEvent(new FakeEvent('submit'));
  return { drawer, config, dialog, $, $$, submit };
}

test('NEBIUS_BASE_URL renders as a provider selector: keep current, the allowlisted providers, default', () => {
  const [nebius, nim, openrouter] = PROVIDERS;
  const { $, $$, submit } = providerDrawer(nebius, null);
  const fetch = stubFetch();
  const select = $('select[data-key="NEBIUS_BASE_URL"]');
  assert.ok(select, 'a <select> for NEBIUS_BASE_URL');
  assert.equal($('input[data-key="NEBIUS_BASE_URL"]'), null, 'no free-text box');
  assert.ok(!select.hasAttribute('disabled'));
  const options = select.querySelectorAll('option');
  assert.deepEqual(options.map((o) => o.getAttribute('value')), ['', ...PROVIDERS.map((p) => p.base_url), settingsMod.DEFAULT_CHOICE]);
  assert.match(text(options[0]), /^Keep current \(Nebius Token Factory\)$/);
  assert.match(text(options[1]), /Nebius Token Factory \(current\)/);
  assert.doesNotMatch(text(options[2]), /current/);
  assert.match(text(options[options.length - 1]), /^Default \(unset: Nebius Token Factory\)$/);
  assert.ok($('[data-clear="NEBIUS_BASE_URL"]'), '"Clear on save" stays next to the selector');

  // Each provider is listed with its key link and its default models (or which ones are required).
  const notes = $$('[data-role="providers"] li');
  assert.deepEqual(notes.map((li) => li.dataset.provider), PROVIDERS.map((p) => p.id));
  assert.deepEqual(notes.map((li) => li.querySelector('a').getAttribute('href')), PROVIDERS.map((p) => p.signup_url));
  assert.match(text(notes[1]), /tier 1: set NEBIUS_TIER1_MODEL/);
  assert.match(text(notes[1]), new RegExp(`tier 2 ${nim.tier2_model.replace(/[.]/g, '\\.')}`));
  assert.match(text(notes[2]), /tier 1: set NEBIUS_TIER1_MODEL · tier 2: set NEBIUS_TIER2_MODEL/);
  assert.match(text($('[id="cfg-NEBIUS_TIER1_MODEL-hint"]')), /Nebius Token Factory default: nvidia\/Llama-3_3-Nemotron-Super-49B-v1/);

  // Nothing chosen and nothing typed: nothing is posted.
  submit();
  assert.equal(fetch.calls.length, 0);
  assert.match(text($('[data-role="save-status"]')), /Nothing to save/);
  assert.equal(settingsMod.providerFor({ nemotron_providers: PROVIDERS }, `${openrouter.base_url}/`), openrouter);
  assert.equal(settingsMod.providerFor({ nemotron_providers: PROVIDERS }, 'https://openrouter.ai.evil.example/api/v1'), null);
});

test('choosing a provider posts its URL together with the typed key; Default posts "" like Clear on save', async () => {
  const [nebius, nim] = PROVIDERS;
  let saved;
  const { drawer, config, $, submit } = providerDrawer(nebius, () => { if (saved) saved(); });
  const fetch = stubFetch();

  $('select[data-key="NEBIUS_BASE_URL"]').value = nim.base_url;
  $('[data-key="NEBIUS_API_KEY"]').value = 'synthetic-key-for-the-test';
  let done = new Promise((resolve) => { saved = resolve; });
  submit();
  const req = await fetch.call(0);
  assert.equal(req.method, 'POST');
  assert.deepEqual(req.body, { values: { NEBIUS_API_KEY: 'synthetic-key-for-the-test', NEBIUS_BASE_URL: nim.base_url } });
  req.respond(providerConfig(nim));
  await done;
  assert.match(text($('[data-role="save-status"]')), /Saved 2 keys: NEBIUS_API_KEY, NEBIUS_BASE_URL/);
  const select = $('select[data-key="NEBIUS_BASE_URL"]');
  assert.equal(select.value, '', 'back to "keep current" after a save');
  assert.match(text(select.querySelectorAll('option')[0]), /Keep current \(NVIDIA NIM\)/);
  // With NIM selected, the tier hints say which model id is required and which has a default.
  assert.match(text($('[id="cfg-NEBIUS_TIER1_MODEL-hint"]')), /Required with NVIDIA NIM/);
  assert.match(text($('[id="cfg-NEBIUS_TIER2_MODEL-hint"]')), /NVIDIA NIM default: nvidia\/llama-3\.1-nemotron-ultra-253b-v1/);

  select.value = settingsMod.DEFAULT_CHOICE;
  done = new Promise((resolve) => { saved = resolve; });
  submit();
  const unset = await fetch.call(1);
  assert.deepEqual(unset.body, { values: { NEBIUS_BASE_URL: '' } });
  unset.respond(config);
  await done;

  // "Clear on save" disables the selector and posts "" without reading it.
  $('[data-clear="NEBIUS_BASE_URL"]').click();
  assert.ok($('select[data-key="NEBIUS_BASE_URL"]').hasAttribute('disabled'));
  done = new Promise((resolve) => { saved = resolve; });
  submit();
  const clear = await fetch.call(2);
  assert.deepEqual(clear.body, { values: { NEBIUS_BASE_URL: '' } });
  clear.respond(config);
  await done;

  // A refused save (e.g. a provider switch without the key) is reported, not swallowed.
  $('select[data-key="NEBIUS_BASE_URL"]').value = PROVIDERS[2].base_url;
  submit();
  const refused = await fetch.call(3);
  refused.respond({ error: 'switching the Nemotron provider to OpenRouter needs that provider\'s API key in NEBIUS_API_KEY in the same save' }, 400);
  await drain();
  await drain();
  assert.match(text($('[data-role="save-status"]')), /Save failed: .*NEBIUS_API_KEY/);
  drawer.setConfig(config);
});

test('run.finished queued_ms is shown next to total_ms and kept across the post-approval finish', () => {
  const { panel, card } = workspacePanel();
  const norm = (v) => v.replace(/\s+/g, ' ').trim();
  const queued = Math.random();
  const total = Math.random();
  panel.onMessage({ type: 'run.started', run_id: 'r-000010', scenario: 'mass_delete', title: 'Mass delete', ts: new Date().toISOString() });
  assert.equal(text(card().querySelector('.run__queued')), norm(`queued ${dom.fmtMs(null)}`));
  panel.onMessage({ type: 'run.finished', run_id: 'r-000010', outcome: 'pending_approval', total_ms: total, queued_ms: queued, ts: new Date().toISOString() });
  assert.equal(text(card().querySelector('.run__total')), norm(`total ${dom.fmtMs(total)}`));
  assert.equal(text(card().querySelector('.run__queued')), norm(`queued ${dom.fmtMs(queued)}`));
  panel.onMessage({ type: 'run.finished', run_id: 'r-000010', outcome: 'committed', total_ms: total + 1, queued_ms: null, ts: new Date().toISOString() });
  assert.equal(text(card().querySelector('.run__queued')), norm(`queued ${dom.fmtMs(queued)}`));
});

test.after(() => mock.reset());
