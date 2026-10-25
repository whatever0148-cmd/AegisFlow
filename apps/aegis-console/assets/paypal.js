// Panel 2: PayPal Agentic Commerce. AG Grid (vendored Community UMD build) with columnDefs from
// GET /api/paypal/orders, live rows from paypal.order messages, and the built-in SVG timeline.

import { h, replace, badge, riskBar, fmtNum, fmtText, fmtTimeUTC, isNum, DASH, prefersReducedMotion, unitForKey, withUnit } from './dom.js';
import { get, post, describeError } from './api.js';
import { createTimeline } from './timeline.js';

const MAX_ROWS = 1000;

function cssVar(name, fallback) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/** AG Grid theming API (v33+): themeQuartz + colorSchemeDark + console tokens. */
function gridTheme(agGrid) {
  const { themeQuartz, colorSchemeDark } = agGrid;
  return themeQuartz.withPart(colorSchemeDark).withParams({
    backgroundColor: cssVar('--surface', 'black'),
    foregroundColor: cssVar('--text', 'white'),
    chromeBackgroundColor: cssVar('--surface-2', 'black'),
    headerBackgroundColor: cssVar('--surface-2', 'black'),
    headerTextColor: cssVar('--text', 'white'),
    borderColor: cssVar('--border', 'gray'),
    accentColor: cssVar('--accent', 'dodgerblue'),
    rowHoverColor: cssVar('--surface-3', 'black'),
    oddRowBackgroundColor: cssVar('--surface-odd', 'black'),
    rangeSelectionBorderColor: cssVar('--focus', 'gold'),
    focusShadow: { radius: 0, spread: 3, color: cssVar('--focus', 'gold') },
    fontFamily: ['system-ui', '-apple-system', 'Segoe UI', 'Roboto', 'sans-serif'],
    headerFontWeight: 650,
    fontSize: 13,
    wrapperBorderRadius: 8,
    browserColorScheme: 'dark',
  });
}

function signatureParts(sig) {
  if (typeof sig !== 'string' || !sig) return { scheme: null, body: '' };
  const parts = sig.split(':');
  if (parts.length >= 3) return { scheme: parts.slice(0, 2).join(':'), body: parts.slice(2).join(':') };
  return { scheme: null, body: sig };
}

// ---- cell renderers (each returns a DOM node) ----------------------------
function decisionRenderer(p) {
  if (p.value === 'Allow') return badge('Allow', 'ok');
  if (p.value === 'Deny') return badge('Deny', 'fail');
  return h('span', { text: fmtText(p.value) });
}

function receiptRenderer(p) {
  const { scheme } = signatureParts(p.data && p.data.signature);
  const label = scheme || 'receipt';
  if (p.value === true) return h('span', { class: 'cell-receipt cell-receipt--ok' }, h('span', { 'aria-hidden': 'true', text: '✓ ' }), h('span', { class: 'mono', text: label }), h('span', { class: 'sr-only', text: ' verified' }));
  if (p.value === false) return h('span', { class: 'cell-receipt cell-receipt--fail' }, h('span', { 'aria-hidden': 'true', text: '✗ ' }), h('span', { class: 'mono', text: label }), h('span', { text: ' not verified' }));
  return h('span', { class: 'muted', text: DASH });
}

function receiptTooltip(p) {
  const d = p.data || {};
  const { scheme, body } = signatureParts(d.signature);
  const parts = [];
  parts.push(`${scheme || 'signature'} ${body ? `${body.slice(0, 16)}…` : DASH}`);
  if (d.receipt_sha256) parts.push(`receipt sha256 ${String(d.receipt_sha256).slice(0, 16)}…`);
  parts.push(d.receipt_verified === true ? 'signature verified' : d.receipt_verified === false ? 'signature NOT verified' : 'verification unknown');
  return parts.join(' · ');
}

function redactionsRenderer(p) {
  const d = p.data || {};
  return h('span', { class: 'cell-redactions' },
    h('span', { class: 'mono', text: fmtNum(p.value) }),
    d.unformatted_sensitive_fields === true ? badge('unformatted secret', 'warn') : null);
}

function riskRenderer(p) {
  return riskBar(p.value);
}

const SUBMISSION = { sent: ['Sent', 'ok'], blocked: ['Blocked', 'fail'], not_sent: ['Not sent', 'skip'], error: ['Error', 'fail'] };
function submissionRenderer(p) {
  const [label, kind] = SUBMISSION[p.value] || [fmtText(p.value), 'neutral'];
  const d = p.data || {};
  return h('span', { class: 'cell-submission' }, badge(label, kind), d.paypal_order_id ? h('span', { class: 'mono small', text: ` ${d.paypal_order_id}` }) : null);
}

/** Order amount as a number for sorting and filtering (OrderRow.amount is a display string). */
export function amountValue(row) {
  if (!row) return null;
  if (isNum(row.amount_cents)) return row.amount_cents / 100;
  const n = typeof row.amount === 'string' && row.amount.trim() !== '' ? Number(row.amount) : NaN;
  return Number.isFinite(n) ? n : null;
}

// Submissions that used one of the daemon's Sandbox slots (reserve_submission runs before the call).
const USES_SANDBOX_SLOT = new Set(['sent', 'error']);

export function decorate(defs) {
  return defs.map((def) => {
    const c = { ...def };
    switch (def.field) {
      case 'decision': c.cellRenderer = decisionRenderer; c.minWidth = 110; break;
      case 'receipt_verified': c.cellRenderer = receiptRenderer; c.tooltip = receiptTooltip; c.minWidth = 170; break;
      case 'redactions': c.cellRenderer = redactionsRenderer; c.minWidth = 190; c.tooltip = (p) => `${fmtNum(p.value)} field(s) redacted before signing${p.data && p.data.unformatted_sensitive_fields ? '; unformatted sensitive data detected' : ''}`; break;
      case 'risk_score': c.cellRenderer = riskRenderer; c.minWidth = 150; break;
      case 'submission': c.cellRenderer = submissionRenderer; c.minWidth = 130; c.tooltip = (p) => (p.data && p.data.submission_detail) || null; break;
      case 'timestamp':
        // Rendered in UTC so the cell matches its "(UTC)" header and the timeline's UTC clock.
        if (!/UTC/.test(c.headerName || '')) c.headerName = `${c.headerName || 'Time'} (UTC)`;
        c.valueFormatter = (p) => fmtTimeUTC(p.value); c.tooltip = (p) => p.value || null; c.minWidth = 104; break;
      case 'amount':
        // Sort and filter on the numeric amount; display the daemon's formatted string unchanged.
        c.valueGetter = (p) => amountValue(p.data);
        c.valueFormatter = (p) => (p.data && p.data.amount ? p.data.amount : DASH);
        c.cellDataType = 'number';
        if (def.filter) {
          c.filter = 'agNumberColumnFilter';
          c.filterValueGetter = (p) => amountValue(p.data);
        }
        c.minWidth = 96;
        break;
      case 'currency': c.minWidth = 76; break;
      case 'merchant': c.minWidth = 140; break;
      case 'policies': c.tooltip = (p) => p.value || null; c.minWidth = 180; break;
      default: {
        const unit = unitForKey(def.field || '');
        if (unit) c.valueFormatter = (p) => (isNum(p.value) ? withUnit(p.value, '') : DASH);
        if (!c.valueFormatter) c.valueFormatter = (p) => (p.value === null || p.value === undefined || p.value === '' ? DASH : p.value);
      }
    }
    return c;
  });
}

export function createPaypalPanel() {
  const root = document.getElementById('panel-paypal');
  const $ = (role) => root.querySelector(`[data-role="${role}"]`);
  const els = {
    error: $('error'), sandbox: $('sandbox'), scenarios: $('scenarios'), status: $('action-status'),
    stream: $('stream'), toggle: $('stream-toggle'), interval: $('stream-interval'), apply: $('stream-apply'),
    grid: $('grid'), count: $('row-count'), timeline: $('timeline'), timelineTable: $('timeline-table'),
    note: $('timeline-note'), scale: $('scale-toggle'),
  };

  let gridApi = null;
  let scenarios = null;
  let services = null;
  let stream = null;
  const ids = [];
  const rowsById = new Map();
  const byReference = new Map();
  const busy = new Set();
  const errors = new Map();

  function setError(key, msg) {
    if (msg) errors.set(key, msg); else errors.delete(key);
    els.error.hidden = errors.size === 0;
    els.error.textContent = [...errors.values()].join(' · ');
  }

  const timeline = createTimeline({
    host: els.timeline,
    tableHost: els.timelineTable,
    noteEl: els.note,
    scaleBtn: els.scale,
    labelFor: (orderId) => {
      const row = rowsById.get(orderId) || byReference.get(orderId);
      return row ? row.reference : null;
    },
  });

  function trackRow(row) {
    if (!row || row.id === undefined || row.id === null) return false;
    const id = String(row.id);
    const isNew = !rowsById.has(id);
    rowsById.set(id, row);
    if (row.reference) byReference.set(row.reference, row);
    if (isNew) ids.push(id);
    return isNew;
  }

  function trimRows() {
    const removed = [];
    while (ids.length > MAX_ROWS) {
      const id = ids.shift();
      const row = rowsById.get(id);
      rowsById.delete(id);
      if (row && row.reference && byReference.get(row.reference) === row) byReference.delete(row.reference);
      if (row) removed.push(row);
    }
    return removed;
  }

  function updateCount() {
    els.count.textContent = `(${rowsById.size} row${rowsById.size === 1 ? '' : 's'}${rowsById.size >= MAX_ROWS ? `, newest ${MAX_ROWS} kept` : ''})`;
  }

  function buildGrid(columnDefs, rows) {
    const agGrid = window.agGrid;
    if (!agGrid || typeof agGrid.createGrid !== 'function') {
      replace(els.grid, h('p', { class: 'error-text', text: 'AG Grid bundle did not load from /vendor/ag-grid-community/ag-grid-community.min.noStyle.js; orders are listed in the timeline only.' }));
      return;
    }
    let defs = Array.isArray(columnDefs) && columnDefs.length ? columnDefs : null;
    if (!defs) {
      setError('cols', 'GET /api/paypal/orders returned no columnDefs; showing raw fields.');
      defs = Object.keys(rows[0] || { id: null }).map((field) => ({ field, headerName: field }));
    } else {
      setError('cols', null);
    }
    const options = {
      theme: gridTheme(agGrid),
      loadThemeGoogleFonts: false,
      columnDefs: decorate(defs),
      rowData: rows,
      getRowId: (p) => String(p.data.id),
      defaultColDef: { resizable: true, sortable: true, flex: 1, minWidth: 96 },
      animateRows: !prefersReducedMotion(),
      ensureDomOrder: true,
      tooltipShowDelay: 250,
      overlayNoRowsTemplate: '<span class="grid-empty">No orders yet. Run a scenario or start the stream.</span>',
      rowHeight: 40,
    };
    if (gridApi) gridApi.destroy();
    replace(els.grid);
    gridApi = agGrid.createGrid(els.grid, options);
  }

  function setOrders(data) {
    const rows = Array.isArray(data && data.rows) ? data.rows.slice() : [];
    ids.length = 0;
    rowsById.clear();
    byReference.clear();
    for (const r of rows) trackRow(r);
    trimRows();
    const kept = ids.map((id) => rowsById.get(id)).reverse(); // newest first
    buildGrid(data && data.columnDefs, kept);
    timeline.setStore((data && data.timeline) || { resources: [], events: [] });
    setError('orders', null);
    updateCount();
  }

  function onOrder(msg) {
    const row = msg.row;
    const events = Array.isArray(msg.events) ? msg.events : [];
    if (row) {
      const isNew = trackRow(row);
      const removed = trimRows();
      if (gridApi) {
        const tx = isNew ? { add: [row], addIndex: 0 } : { update: [row] };
        if (removed.length) tx.remove = removed;
        gridApi.applyTransactionAsync(tx);
      }
      updateCount();
      if (USES_SANDBOX_SLOT.has(row.submission)) refreshScenarios();
    }
    if (events.length) timeline.addEvents(events);
  }

  // ---- scenarios, stream, sandbox ----------------------------------------
  // GET /api/paypal/scenarios and config.changed both report whether PayPal credentials are set.
  // When they disagree, one is stale: show that, and re-read the scenarios endpoint.
  function sandboxStale() {
    const sb = scenarios && scenarios.sandbox;
    const svc = services && services.paypal;
    return Boolean(sb && svc && typeof svc.enabled === 'boolean' && typeof sb.enabled === 'boolean' && svc.enabled !== sb.enabled);
  }

  function renderSandbox() {
    const sb = scenarios && scenarios.sandbox;
    const svc = services && services.paypal;
    const parts = [];
    if (sb && sandboxStale()) {
      parts.push(refreshPending()
        ? badge('Sandbox status refreshing', 'pending')
        : badge(`Sandbox status unconfirmed (settings: credentials ${svc.enabled ? 'set' : 'not set'})`, 'warn'));
      parts.push(h('span', { class: 'mono', text: ` submitted ${fmtNum(sb.submitted)} / cap ${fmtNum(sb.cap)}` }));
    } else if (sb) {
      parts.push(sb.enabled === true ? badge('Sandbox enabled', 'ok') : badge('Sandbox disabled', 'skip'));
      parts.push(h('span', { class: 'mono', text: ` submitted ${fmtNum(sb.submitted)} / cap ${fmtNum(sb.cap)}` }));
    } else {
      parts.push(h('span', { class: 'muted', text: 'Sandbox status unavailable' }));
    }
    if (svc && svc.enabled !== true && svc.reason) parts.push(h('span', { class: 'reason small', text: ` · ${svc.reason}` }));
    if (svc && svc.enabled === true && svc.base_url) parts.push(h('span', { class: 'muted small mono', text: ` · ${svc.base_url}` }));
    replace(els.sandbox, parts);
  }

  function renderStream() {
    const enabled = stream && stream.enabled === true;
    els.toggle.setAttribute('aria-pressed', String(enabled));
    els.toggle.textContent = stream ? `Stream: ${enabled ? 'on' : 'off'}` : 'Stream: —';
    els.toggle.classList.toggle('btn--on', enabled);
    if (stream && isNum(stream.interval_ms) && document.activeElement !== els.interval) els.interval.value = String(stream.interval_ms);
  }

  function renderScenarios() {
    const list = scenarios && Array.isArray(scenarios.scenarios) ? scenarios.scenarios : [];
    if (!list.length) {
      replace(els.scenarios, h('p', { class: 'muted', text: 'The daemon returned no order scenarios.' }));
      return;
    }
    const focused = document.activeElement && els.scenarios.contains(document.activeElement) ? document.activeElement.dataset.scenario : null;
    replace(els.scenarios, list.map((sc) => h('button', {
      type: 'button', class: 'scenario scenario--compact', disabled: busy.has(sc.id), dataset: { scenario: sc.id },
      'aria-labelledby': `pp-sc-${sc.id}-title`, 'aria-describedby': `pp-sc-${sc.id}-desc`,
      onclick: (ev) => placeOrder(sc, ev.currentTarget),
    },
    h('span', { class: 'scenario__title', id: `pp-sc-${sc.id}-title`, text: fmtText(sc.title || sc.id) }),
    h('span', { class: 'scenario__desc', id: `pp-sc-${sc.id}-desc`, text: fmtText(sc.description) }),
    h('span', { class: 'scenario__id mono', text: sc.id }))));
    if (focused) {
      const again = els.scenarios.querySelector(`[data-scenario="${CSS.escape(focused)}"]`);
      if (again && !again.disabled) again.focus();
    }
  }

  async function placeOrder(sc, btn) {
    busy.add(sc.id);
    btn.disabled = true;
    els.status.textContent = `Submitting ${sc.id}…`;
    try {
      const res = await post('/api/paypal/order', { scenario: sc.id });
      els.status.textContent = `Order ${fmtText(res && res.order_id)} accepted (${sc.id}); the row arrives on the live stream.`;
    } catch (err) {
      els.status.textContent = `Order ${sc.id} failed: ${describeError(err)}`;
    } finally {
      busy.delete(sc.id);
      const current = els.scenarios.querySelector(`[data-scenario="${CSS.escape(sc.id)}"]`) || btn;
      current.disabled = false;
      if (current.isConnected && (!document.activeElement || document.activeElement === document.body)) current.focus();
    }
  }

  async function setStream(enabled, trigger) {
    const interval = Number(els.interval.value);
    if (!Number.isFinite(interval) || interval <= 0) {
      els.status.textContent = 'Enter a positive interval in milliseconds.';
      els.interval.focus();
      return;
    }
    els.toggle.disabled = true;
    els.apply.disabled = true;
    try {
      const res = await post('/api/paypal/stream', { enabled, interval_ms: Math.round(interval) });
      stream = res;
      if (scenarios) scenarios.stream = res;
      renderStream();
      els.status.textContent = `Stream ${res && res.enabled ? 'on' : 'off'} · interval ${fmtNum(res && res.interval_ms)} ms`;
    } catch (err) {
      els.status.textContent = `Stream update failed: ${describeError(err)}`;
    } finally {
      els.toggle.disabled = false;
      els.apply.disabled = false;
      if (trigger && (!document.activeElement || document.activeElement === document.body)) trigger.focus();
    }
  }

  els.toggle.addEventListener('click', () => setStream(!(stream && stream.enabled === true), els.toggle));
  els.stream.addEventListener('submit', (ev) => {
    ev.preventDefault();
    setStream(Boolean(stream && stream.enabled === true), els.apply);
  });

  function setScenarios(data) {
    scenarios = data || null;
    stream = (data && data.stream) || null;
    setError('scenarios', null);
    renderScenarios();
    renderStream();
    renderSandbox();
  }

  let refreshTimer = null;
  let refreshesInFlight = 0;
  function refreshPending() {
    return refreshTimer !== null || refreshesInFlight > 0;
  }
  function refreshScenarios(delayMs = 1000) {
    if (refreshTimer) return;
    refreshTimer = setTimeout(async () => {
      refreshTimer = null;
      refreshesInFlight += 1;
      try {
        const data = await get('/api/paypal/scenarios');
        refreshesInFlight -= 1;
        setScenarios(data);
      } catch (err) {
        refreshesInFlight -= 1;
        setError('scenarios', `GET /api/paypal/scenarios: ${describeError(err)}`);
        renderSandbox();
      }
    }, delayMs);
  }

  function setServices(svc) {
    services = svc || null;
    // Saving or clearing PayPal credentials changes the Sandbox state and arrives here first.
    if (sandboxStale()) refreshScenarios(0);
    renderSandbox();
  }

  return {
    setOrders,
    setScenarios,
    setServices,
    onMessage(msg) { if (msg.type === 'paypal.order') onOrder(msg); },
    failOrders(err) {
      setError('orders', `GET /api/paypal/orders: ${describeError(err)}`);
      if (!gridApi) replace(els.grid, h('p', { class: 'muted grid-placeholder', text: 'Orders unavailable until the daemon answers.' }));
    },
    failScenarios(err) {
      setError('scenarios', `GET /api/paypal/scenarios: ${describeError(err)}`);
      if (!scenarios) replace(els.scenarios, h('p', { class: 'muted', text: 'Scenarios unavailable until the daemon answers.' }));
      renderSandbox();
    },
  };
}
