// Panel 2: PayPal Agentic Commerce. AG Grid (vendored Community UMD build) with columnDefs from
// GET /api/paypal/orders, live rows from paypal.order messages, and the built-in SVG timeline.

import { h, replace, badge, riskBar, fmtNum, fmtText, fmtTimeUTC, isNum, DASH, prefersReducedMotion, unitForKey, withUnit, copyButton } from './dom.js';
import { get, post, describeError } from './api.js';
import { createTimeline } from './timeline.js';
import { createReceiptViewer } from './receipt.js';

// Rows shown; GET /api/paypal/orders `limit` replaces it (the daemon keeps that many), so the grid
// shows the same orders before and after a reload.
const DEFAULT_MAX_ROWS = 20;

function cssVar(name, fallback) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/** AG Grid theming API (v33+): themeQuartz + colorSchemeDark, tuned to the console's cockpit tokens. */
function gridTheme(agGrid) {
  const { themeQuartz, colorSchemeDark } = agGrid;
  const hair = cssVar('--hair', 'rgba(255,255,255,0.08)');
  return themeQuartz.withPart(colorSchemeDark).withParams({
    backgroundColor: cssVar('--inset', 'black'),
    foregroundColor: cssVar('--text', 'white'),
    textColor: cssVar('--text-2', 'white'),
    chromeBackgroundColor: cssVar('--card', 'black'),
    headerBackgroundColor: cssVar('--card', 'black'),
    headerTextColor: cssVar('--text-3', 'gray'),
    headerFontSize: 10.5,
    headerFontWeight: 650,
    headerHeight: 34,
    headerRowBorder: { color: hair },
    headerColumnBorder: false,
    borderColor: hair,
    rowBorder: { color: hair },
    wrapperBorder: { color: hair },
    wrapperBorderRadius: 9,
    accentColor: cssVar('--cyan', 'teal'),
    rowHoverColor: 'rgba(6, 182, 212, 0.06)',
    selectedRowBackgroundColor: 'rgba(6, 182, 212, 0.1)',
    oddRowBackgroundColor: 'rgba(255, 255, 255, 0.014)',
    rangeSelectionBorderColor: cssVar('--focus', 'white'),
    focusShadow: { radius: 0, spread: 2, color: cssVar('--focus', 'white') },
    fontFamily: ['-apple-system', 'SF Pro Text', 'Inter', 'system-ui', 'sans-serif'],
    dataFontSize: 12,
    fontSize: 12,
    cellHorizontalPadding: 10,
    spacing: 6,
    iconSize: 12,
    tooltipBackgroundColor: cssVar('--card-3', 'black'),
    tooltipBorder: { color: cssVar('--hair-2', 'gray') },
    browserColorScheme: 'dark',
  });
}

export function signatureParts(sig) {
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

/** `abcdef…wxyz`: the start and end of a signature body, for a compact pill. */
export function truncateMiddle(text, head = 6, tail = 4) {
  const t = String(text || '');
  return t.length > head + tail + 1 ? `${t.slice(0, head)}…${t.slice(-tail)}` : t;
}

/** Receipt cell: a cryptographic pill (✓/✗, scheme, truncated signature) with a copy button. */
export function receiptRenderer(p) {
  const d = p.data || {};
  const { scheme, body } = signatureParts(d.signature);
  if (p.value !== true && p.value !== false) return h('span', { class: 'muted', text: DASH });
  const ok = p.value === true;
  return h('span', { class: 'cell-receipt' },
    h('span', { class: `crypto crypto--${ok ? 'ok' : 'fail'}` },
      h('span', { class: 'crypto__glyph', 'aria-hidden': 'true', text: ok ? '✓' : '✗' }),
      h('span', { class: 'crypto__scheme', text: scheme || 'receipt' }),
      body ? h('span', { class: 'crypto__body', text: truncateMiddle(body) }) : null,
      h('span', { class: ok ? 'sr-only' : 'crypto__state', text: ok ? ' verified' : ' not verified' }),
      d.signature ? copyButton(() => d.signature, `Copy receipt signature of ${fmtText(d.reference)}`) : null));
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

/** Redactions cell: a `[REDACTED] ×n` pill (or 0), plus a warning for unformatted sensitive data. */
export function redactionsRenderer(p) {
  const d = p.data || {};
  const n = p.value;
  return h('span', { class: 'cell-redactions' },
    isNum(n) && n > 0
      ? h('span', { class: 'redacted', title: `${fmtNum(n)} field(s) replaced by [REDACTED] before signing` }, '[REDACTED]', h('span', { class: 'redacted__n', text: `×${fmtNum(n)}` }))
      : h('span', { class: 'mono muted', text: fmtNum(n) }),
    d.unformatted_sensitive_fields === true ? badge('unformatted secret', 'fail') : null);
}

function riskRenderer(p) {
  return riskBar(p.value);
}

const SUBMISSION = { sent: ['Sent', 'ok'], blocked: ['Blocked', 'fail'], not_sent: ['Not sent', 'skip'], error: ['Error', 'fail'] };

/** Text after the Submission badge: the order id, and for a sent order its status and target
 * (e.g. `LOCAL-… · COMPLETED · local`). Null when no order was created. */
export function submissionLabel(row) {
  if (!row || !row.paypal_order_id) return null;
  const parts = [String(row.paypal_order_id)];
  if (row.submission === 'sent' && row.submission_detail) parts.push(String(row.submission_detail));
  if (row.submission === 'sent' && row.submission_target) parts.push(String(row.submission_target));
  return parts.join(' · ');
}

function submissionRenderer(p) {
  const [label, kind] = SUBMISSION[p.value] || [fmtText(p.value), 'neutral'];
  const extra = submissionLabel(p.data);
  return h('span', { class: 'cell-submission' }, badge(label, kind), extra ? h('span', { class: 'mono small', text: ` ${extra}` }) : null);
}

/** Submission mode from GET /api/paypal/scenarios `sandbox` or GET /api/config `services.paypal`:
 * "local" (the daemon's local Orders v2 gateway), "remote" (PayPal Sandbox) or "disabled". Daemons
 * without `mode` sent only `enabled`, which then meant the remote Sandbox. */
export function sandboxMode(s) {
  if (!s) return null;
  if (s.mode === 'local' || s.mode === 'remote' || s.mode === 'disabled') return s.mode;
  if (typeof s.enabled === 'boolean') return s.enabled ? 'remote' : 'disabled';
  return null;
}

const SANDBOX_BADGE = {
  local: ['🟢 Sandbox Active (Local v2 Gateway)', 'ok'],
  remote: ['🟢 Sandbox Active (PayPal Remote)', 'ok'],
  disabled: ['Sandbox disabled', 'skip'],
};
const MODE_WORDS = { local: 'local gateway', remote: 'PayPal remote', disabled: 'disabled' };

/** The Panel 2 badge text for a mode. */
export function sandboxBadgeText(mode) {
  return (SANDBOX_BADGE[mode] || SANDBOX_BADGE.disabled)[0];
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
      case 'decision': c.cellRenderer = decisionRenderer; c.minWidth = 96; break;
      case 'receipt_verified': c.cellRenderer = receiptRenderer; c.tooltip = receiptTooltip; c.minWidth = 236; break;
      case 'redactions': c.cellRenderer = redactionsRenderer; c.minWidth = 170; c.tooltip = (p) => `${fmtNum(p.value)} field(s) redacted before signing${p.data && p.data.unformatted_sensitive_fields ? '; unformatted sensitive data detected' : ''}`; break;
      case 'risk_score': c.cellRenderer = riskRenderer; c.minWidth = 136; break;
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

// Display order: the decision and its cryptographic evidence first, descriptive columns last.
const COLUMN_ORDER = ['timestamp', 'reference', 'amount', 'currency', 'risk_score', 'decision', 'receipt_verified', 'redactions', 'submission', 'total_us', 'submit_us', 'merchant', 'policies'];

/** Columns reordered by COLUMN_ORDER; fields it does not list keep their order, after the listed ones. */
export function orderColumns(defs) {
  const rank = (d) => { const i = COLUMN_ORDER.indexOf(d.field); return i < 0 ? COLUMN_ORDER.length : i; };
  return defs.map((d, i) => ({ d, i })).sort((a, b) => (rank(a.d) - rank(b.d)) || (a.i - b.i)).map(({ d }) => d);
}

/** Extra column: one-click "Inspect" of the order's signed receipt JSON (Enter on the cell opens it too). */
export function inspectColumn(open) {
  return {
    colId: 'signed_receipt', headerName: 'Receipt JSON', sortable: false, filter: false, resizable: false, minWidth: 112, maxWidth: 128, pinned: 'right',
    cellClass: 'cell-inspect',
    cellRenderer: (p) => (p.data && p.data.id ? h('button', {
      type: 'button', class: 'btn btn--small', 'aria-label': `Inspect signed receipt JSON of ${fmtText(p.data.reference)}`,
      onclick: (ev) => open(p.data, ev.currentTarget),
    }, h('span', { 'aria-hidden': 'true', text: '{ }' }), 'Inspect') : null),
    suppressKeyboardEvent: (p) => p.event && p.event.key === 'Enter',
  };
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
  const receipts = createReceiptViewer();
  const inspect = (row, trigger) => receipts.open(String(row.id), row.reference, trigger);
  let maxRows = DEFAULT_MAX_ROWS;
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
    while (ids.length > maxRows) {
      const id = ids.shift();
      const row = rowsById.get(id);
      rowsById.delete(id);
      if (row && row.reference && byReference.get(row.reference) === row) byReference.delete(row.reference);
      if (row) removed.push(row);
    }
    return removed;
  }

  function updateCount() {
    els.count.textContent = `(${rowsById.size} row${rowsById.size === 1 ? '' : 's'}${rowsById.size >= maxRows ? `, newest ${maxRows} kept` : ''})`;
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
      columnDefs: [...orderColumns(decorate(defs)), inspectColumn(inspect)],
      onCellKeyDown: (e) => {
        if (e.event && e.event.key === 'Enter' && e.column && e.column.getColId && e.column.getColId() === 'signed_receipt' && e.data) inspect(e.data, null);
      },
      rowData: rows,
      getRowId: (p) => String(p.data.id),
      defaultColDef: { resizable: true, sortable: true, flex: 1, minWidth: 96 },
      animateRows: !prefersReducedMotion(),
      ensureDomOrder: true,
      tooltipShowDelay: 250,
      overlayNoRowsTemplate: '<span class="grid-empty">No orders yet. Run a scenario or start the stream.</span>',
      rowHeight: 38,
    };
    if (gridApi) gridApi.destroy();
    replace(els.grid);
    gridApi = agGrid.createGrid(els.grid, options);
  }

  function setOrders(data) {
    const rows = Array.isArray(data && data.rows) ? data.rows.slice() : [];
    if (isNum(data && data.limit) && data.limit > 0) maxRows = data.limit;
    ids.length = 0;
    rowsById.clear();
    byReference.clear();
    for (const r of rows) trackRow(r);
    trimRows();
    const kept = ids.map((id) => rowsById.get(id)).reverse(); // newest first
    buildGrid(data && data.columnDefs, kept);
    const store = (data && data.timeline) || { resources: [], events: [] };
    const events = Array.isArray(store.events) ? store.events.filter((e) => e && rowsById.has(String(e.orderId))) : [];
    timeline.setStore({ ...store, events });
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
      if (removed.length) timeline.removeOrders(removed.map((r) => String(r.id)));
    }
    if (events.length) timeline.addEvents(events);
  }

  // ---- scenarios, stream, sandbox ----------------------------------------
  // GET /api/paypal/scenarios and config.changed both report the submission mode (local gateway,
  // PayPal remote, disabled). When they disagree, one is stale: show that, and re-read the scenarios
  // endpoint.
  function sandboxStale() {
    const a = sandboxMode(scenarios && scenarios.sandbox);
    const b = sandboxMode(services && services.paypal);
    return Boolean(a && b && a !== b);
  }

  // Remote Sandbox orders count against the per-run cap; local gateway orders are uncapped (the
  // stream interval bounds them).
  function counts(sb, mode) {
    if (mode === 'local') return ` local orders ${fmtNum(isNum(sb.local_submitted) ? sb.local_submitted : 0)} · no cap`;
    return ` submitted ${fmtNum(sb.submitted)} / cap ${fmtNum(sb.cap)}`;
  }

  function renderSandbox() {
    const sb = scenarios && scenarios.sandbox;
    const svc = services && services.paypal;
    const parts = [];
    if (sb && sandboxStale()) {
      parts.push(refreshPending()
        ? badge('Sandbox status refreshing', 'pending')
        : badge(`Sandbox status unconfirmed (settings: ${MODE_WORDS[sandboxMode(svc)]})`, 'warn'));
      parts.push(h('span', { class: 'mono', text: counts(sb, sandboxMode(sb)) }));
    } else if (sb) {
      const mode = sandboxMode(sb) || 'disabled';
      const [label, kind] = SANDBOX_BADGE[mode];
      parts.push(badge(label, kind));
      parts.push(h('span', { class: 'mono', text: counts(sb, mode) }));
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
    // Saving or clearing PayPal credentials changes the submission mode and arrives here first.
    if (sandboxStale()) refreshScenarios(0);
    renderSandbox();
  }

  return {
    inspect,
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
