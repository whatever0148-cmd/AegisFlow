// Panel 2 Sandbox badge and Submission cell for the three submission modes (API.md: sandbox.mode /
// services.paypal.mode = "local" | "remote" | "disabled").
//
//   node --test apps/aegis-console/dev/tests/paypal-sandbox.test.mjs
//
// The real assets/paypal.js runs against the minimal DOM in fake-dom.mjs with fetch stubbed. Counts
// are synthetic (Math.random) and expectations use the console's own formatter.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { installFakeDom, el, roles } from './fake-dom.mjs';

installFakeDom();
const dom = await import('../../assets/dom.js');
const paypalMod = await import('../../assets/paypal.js');

const text = (node) => (node ? node.textContent.replace(/\s+/g, ' ').trim() : '');
const drain = () => new Promise((resolve) => setImmediate(resolve));
const synthCount = () => 1 + Math.floor(Math.random() * 40);

/** fetch stub: records calls; call(i) resolves once the i-th request has been made. */
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

function paypalPanel() {
  const doc = installFakeDom();
  doc.body.append(el('section', { id: 'panel-paypal' },
    ...roles('error:p', 'sandbox', 'scenarios', 'action-status:p', 'stream:form', 'stream-toggle:button', 'stream-interval:input',
      'stream-apply:button', 'grid', 'row-count:span', 'timeline', 'timeline-table', 'timeline-note:span', 'scale-toggle:button')));
  const panel = paypalMod.createPaypalPanel();
  return { panel, sandbox: doc.querySelector('[data-role="sandbox"]') };
}

const LOCAL_URL = 'http://127.0.0.1:4173/paypal-sandbox';
const REMOTE_URL = 'https://api-m.sandbox.paypal.com';

function sandboxFor(mode, counts) {
  return {
    enabled: mode !== 'disabled',
    mode,
    reason: mode === 'disabled' ? 'PAYPAL_CLIENT_SECRET not set' : 'mode reason',
    base_url: mode === 'local' ? LOCAL_URL : REMOTE_URL,
    submitted: counts.submitted,
    cap: counts.cap,
    local_submitted: counts.local,
    gateway: null,
  };
}

test('the badge names the local gateway, the remote Sandbox, or the disabled state', () => {
  const counts = { submitted: synthCount(), cap: synthCount() + 40, local: synthCount() };
  const cases = [
    ['local', /🟢 Sandbox Active \(Local v2 Gateway\)/, `local orders ${dom.fmtNum(counts.local)} · no cap`],
    ['remote', /🟢 Sandbox Active \(PayPal Remote\)/, `submitted ${dom.fmtNum(counts.submitted)} / cap ${dom.fmtNum(counts.cap)}`],
    ['disabled', /Sandbox disabled/, `submitted ${dom.fmtNum(counts.submitted)} / cap ${dom.fmtNum(counts.cap)}`],
  ];
  for (const [mode, badge, countText] of cases) {
    stubFetch();
    const { panel, sandbox } = paypalPanel();
    const sb = sandboxFor(mode, counts);
    panel.setScenarios({ scenarios: [], stream: { enabled: false, interval_ms: 60000 }, sandbox: sb });
    panel.setServices({ paypal: sb });
    const shown = text(sandbox);
    assert.match(shown, badge, mode);
    assert.ok(shown.includes(countText), `${mode}: ${shown}`);
    for (const other of cases.filter(([m]) => m !== mode)) assert.doesNotMatch(shown, other[1], `${mode} shows only its own badge`);
    if (mode === 'local') assert.ok(shown.includes(LOCAL_URL), shown);
    if (mode === 'disabled') assert.ok(shown.includes('PAYPAL_CLIENT_SECRET not set'), shown);
    assert.equal(paypalMod.sandboxBadgeText(mode), badge.source.replace(/\\/g, ''));
  }
});

test('a mode change from settings re-reads the scenarios before trusting the badge', async () => {
  const fetch = stubFetch();
  const counts = { submitted: 0, cap: 25, local: synthCount() };
  const { panel, sandbox } = paypalPanel();
  panel.setScenarios({ scenarios: [], stream: { enabled: false, interval_ms: 60000 }, sandbox: sandboxFor('local', counts) });
  panel.setServices({ paypal: sandboxFor('local', counts) });
  assert.equal(fetch.calls.length, 0, 'no refetch while both sources agree');
  panel.setServices({ paypal: sandboxFor('remote', counts) }); // config.changed: real keys saved
  assert.match(text(sandbox), /refreshing/);
  assert.doesNotMatch(text(sandbox), /Local v2 Gateway/);
  const req = await fetch.call(0);
  assert.equal(req.url, '/api/paypal/scenarios');
  req.respond({ scenarios: [], stream: { enabled: false, interval_ms: 60000 }, sandbox: sandboxFor('remote', counts) });
  await drain();
  assert.match(text(sandbox), /Sandbox Active \(PayPal Remote\)/);
});

test('older daemons without mode map enabled to remote / disabled', () => {
  assert.equal(paypalMod.sandboxMode({ enabled: true }), 'remote');
  assert.equal(paypalMod.sandboxMode({ enabled: false }), 'disabled');
  assert.equal(paypalMod.sandboxMode({ enabled: true, mode: 'local' }), 'local');
  assert.equal(paypalMod.sandboxMode(null), null);
});

test('the Submission cell shows the local order id, status and target', () => {
  const id = `LOCAL-${Math.random().toString(16).slice(2, 10).toUpperCase()}`;
  assert.equal(paypalMod.submissionLabel({ submission: 'sent', paypal_order_id: id, submission_detail: 'COMPLETED', submission_target: 'local' }), `${id} · COMPLETED · local`);
  assert.equal(paypalMod.submissionLabel({ submission: 'sent', paypal_order_id: id, submission_detail: 'CREATED', submission_target: 'remote' }), `${id} · CREATED · remote`);
  assert.equal(paypalMod.submissionLabel({ submission: 'blocked', paypal_order_id: null, submission_detail: 'Cedar denied' }), null);
  const [col] = paypalMod.decorate([{ field: 'submission', headerName: 'Submission' }]);
  const cell = col.cellRenderer({ value: 'sent', data: { submission: 'sent', paypal_order_id: id, submission_detail: 'COMPLETED', submission_target: 'local' } });
  assert.match(text(cell), new RegExp(`Sent ${id} · COMPLETED · local`));
});
