// Built-in SVG timeline for the PayPal pipeline. Reads the Bryntum Scheduler event-store shape
// ({resources:[{id,name}], events:[{id, resourceId, name, startDate, endDate, durationUs, orderId, cls}]})
// and draws one window per order (newest on the right) with one lane per resource.

import { h, s, replace, fmtUs, fmtText, isNum, DASH } from './dom.js';

const MAX_ORDERS_KEPT = 200;
const MIN_WINDOW_PX = 104;
const MAX_WINDOWS = 10;
const LANE_H = 24;
const HEADER_H = 42;
const FOOT_H = 22;
const GAP = 8;
const CLS_RANK = { ok: 0, blocked: 1, error: 2 };

function parseTime(iso) {
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
}

function clockMs(iso) {
  if (typeof iso !== 'string') return DASH;
  const m = iso.match(/T(\d{2}:\d{2}:\d{2}(?:\.\d+)?)/);
  return m ? m[1] : iso;
}

export function createTimeline({ host, tableHost, noteEl, scaleBtn, labelFor }) {
  let resources = [];
  const orders = new Map(); // orderId -> { id, seq, events: Map }
  let seq = 0;
  let shared = true;
  let frame = 0;
  let lastWidth = 0;

  function ensureResource(id) {
    if (id && !resources.some((r) => r.id === id)) resources.push({ id, name: id });
  }

  function addEvents(events, doRender = true) {
    for (const ev of Array.isArray(events) ? events : []) {
      if (!ev || !ev.orderId) continue;
      ensureResource(ev.resourceId);
      let order = orders.get(ev.orderId);
      if (!order) {
        order = { id: ev.orderId, seq: seq++, events: new Map() };
        orders.set(ev.orderId, order);
      }
      order.events.set(ev.id || `${ev.orderId}:${ev.resourceId}:${order.events.size}`, ev);
    }
    while (orders.size > MAX_ORDERS_KEPT) orders.delete(orders.keys().next().value);
    if (doRender) schedule();
  }

  function setStore(store) {
    resources = Array.isArray(store && store.resources) ? store.resources.map((r) => ({ id: r.id, name: r.name || r.id })) : [];
    orders.clear();
    addEvents(store && store.events, false);
    schedule();
  }

  function orderStart(order) {
    let min = null;
    for (const ev of order.events.values()) {
      const t = parseTime(ev.startDate);
      if (t !== null && (min === null || t < min)) min = t;
    }
    return min;
  }

  function sortedOrders() {
    return [...orders.values()].sort((a, b) => {
      const ta = orderStart(a);
      const tb = orderStart(b);
      if (ta !== null && tb !== null && ta !== tb) return ta - tb;
      return a.seq - b.seq;
    });
  }

  // Start offsets: `startOffsetUs` when the daemon sends it; otherwise startDate (ms resolution) with
  // spans packed sequentially so microsecond stages that share a millisecond do not overlap.
  function layoutOrder(order) {
    const resIndex = (id) => resources.findIndex((r) => r.id === id);
    const evs = [...order.events.values()].sort((a, b) => {
      if (isNum(a.startOffsetUs) && isNum(b.startOffsetUs)) return a.startOffsetUs - b.startOffsetUs;
      return ((parseTime(a.startDate) || 0) - (parseTime(b.startDate) || 0)) || (resIndex(a.resourceId) - resIndex(b.resourceId));
    });
    const t0 = orderStart(order);
    let cursor = 0;
    let exact = true;
    let worst = 'ok';
    const items = evs.map((ev) => {
      const start = parseTime(ev.startDate);
      const end = parseTime(ev.endDate);
      const dur = isNum(ev.durationUs) ? ev.durationUs : (start !== null && end !== null ? Math.max(0, (end - start) * 1000) : 0);
      let off;
      if (isNum(ev.startOffsetUs)) {
        off = ev.startOffsetUs;
      } else {
        exact = false;
        const byDate = start !== null && t0 !== null ? (start - t0) * 1000 : cursor;
        off = Math.max(byDate, cursor);
      }
      cursor = Math.max(cursor, off + dur);
      if ((CLS_RANK[ev.cls] || 0) > (CLS_RANK[worst] || 0)) worst = ev.cls;
      return { ev, off, dur };
    });
    return { order, items, total: cursor, exact, worst };
  }

  function label(order) {
    const fromRow = labelFor ? labelFor(order.id) : null;
    if (fromRow) return fromRow;
    const first = order.events.values().next().value;
    if (first && typeof first.name === 'string' && first.resourceId && first.name.endsWith(` ${first.resourceId}`)) {
      return first.name.slice(0, -(first.resourceId.length + 1));
    }
    return order.id;
  }

  function truncate(text, px) {
    const max = Math.max(4, Math.floor(px / 7));
    return text.length > max ? `${text.slice(0, max - 1)}…` : text;
  }

  function render() {
    frame = 0;
    const width = Math.max(300, Math.floor(host.clientWidth || 600));
    lastWidth = width;
    const gutter = width < 520 ? 60 : 82;
    const n = Math.max(1, Math.min(MAX_WINDOWS, Math.floor((width - gutter + GAP) / (MIN_WINDOW_PX + GAP))));
    const visible = sortedOrders().slice(-n);
    if (!visible.length) {
      replace(host, h('p', { class: 'muted timeline__empty', text: 'No pipeline events yet. Run an order scenario or start the stream.' }));
      replace(tableHost, h('p', { class: 'muted', text: 'No events.' }));
      noteEl.textContent = '';
      return;
    }
    const layouts = visible.map(layoutOrder);
    const maxTotal = Math.max(0, ...layouts.map((l) => l.total));
    const ww = (width - gutter - GAP * (n - 1)) / n;
    const height = HEADER_H + resources.length * LANE_H + FOOT_H;
    const titleId = 'tl-title';
    const descId = 'tl-desc';
    const blocked = layouts.filter((l) => l.worst !== 'ok').length;

    const svg = s('svg', {
      class: 'tl', viewBox: `0 0 ${width} ${height}`, width, height, role: 'img', 'aria-labelledby': `${titleId} ${descId}`,
    },
    s('title', { id: titleId, text: 'PayPal pipeline timeline' }),
    s('desc', { id: descId, text: `Last ${layouts.length} orders, lanes ${resources.map((r) => r.name).join(', ')}; ${blocked} not ok. Full data is in the table below.` }),
    s('defs', null,
      s('pattern', { id: 'tl-hatch', width: 6, height: 6, patternUnits: 'userSpaceOnUse', patternTransform: 'rotate(45)' },
        s('rect', { width: 2.5, height: 6, class: 'tl-hatch' })),
      s('pattern', { id: 'tl-cross', width: 6, height: 6, patternUnits: 'userSpaceOnUse' },
        s('path', { d: 'M0 0 6 6M6 0 0 6', class: 'tl-cross' }))));

    resources.forEach((r, i) => {
      const y = HEADER_H + i * LANE_H;
      svg.append(
        s('rect', { x: 0, y, width, height: LANE_H, class: i % 2 ? 'tl-lane tl-lane--alt' : 'tl-lane' }),
        s('text', { x: gutter - 8, y: y + LANE_H / 2 + 4, 'text-anchor': 'end', class: 'tl-lane-label', text: r.name }));
    });

    layouts.forEach((l, i) => {
      const x0 = gutter + (n - layouts.length + i) * (ww + GAP);
      const scale = (shared ? maxTotal : l.total) || 1;
      const inner = ww - 8;
      const g = s('g', { class: `tl-order tl-order--${l.worst}` });
      g.append(
        s('rect', { x: x0, y: 2, width: ww, height: height - FOOT_H - 2, rx: 4, class: 'tl-window' }),
        s('text', { x: x0 + 4, y: 16, class: 'tl-order-label', text: truncate(label(l.order), ww - 8) },
          s('title', { text: `${label(l.order)} (${l.order.id})` })),
        s('text', { x: x0 + 4, y: 32, class: `tl-order-meta tl-order-meta--${l.worst}`, text: truncate(`${l.worst} · ${fmtUs(l.total)}`, ww - 8) }));
      for (const it of l.items) {
        const lane = resources.findIndex((r) => r.id === it.ev.resourceId);
        if (lane < 0) continue;
        const x = x0 + 4 + (it.off / scale) * inner;
        const w = Math.max(2, (it.dur / scale) * inner);
        const y = HEADER_H + lane * LANE_H + 5;
        const cls = CLS_RANK[it.ev.cls] !== undefined ? it.ev.cls : 'ok';
        const tip = `${fmtText(it.ev.name)} · ${fmtUs(it.dur)} · ${fmtText(it.ev.cls)} · ${clockMs(it.ev.startDate)}`;
        g.append(s('rect', { x, y, width: w, height: LANE_H - 10, rx: 2, class: `tl-span tl-span--${cls}` }, s('title', { text: tip })));
        if (cls === 'blocked') g.append(s('rect', { x, y, width: w, height: LANE_H - 10, class: 'tl-overlay', fill: 'url(#tl-hatch)' }, s('title', { text: tip })));
        if (cls === 'error') g.append(s('rect', { x, y, width: w, height: LANE_H - 10, class: 'tl-overlay', fill: 'url(#tl-cross)' }, s('title', { text: tip })));
      }
      svg.append(g);
    });

    svg.append(s('text', { x: gutter, y: height - 6, class: 'tl-axis', text: shared
      ? `Each window spans 0–${fmtUs(maxTotal)} (shared scale)`
      : 'Each window is scaled to its own order total' }));

    replace(host, svg);
    const exact = layouts.every((l) => l.exact);
    noteEl.textContent = exact
      ? 'Span offsets from startOffsetUs.'
      : 'Span offsets from startDate (millisecond resolution); spans within one millisecond are packed in start order using durationUs.';
    renderTable(layouts);
  }

  function renderTable(layouts) {
    const rows = [];
    for (const l of [...layouts].reverse()) {
      for (const it of l.items) {
        rows.push(h('tr', null,
          h('td', { class: 'mono', text: label(l.order) }),
          h('td', { text: (resources.find((r) => r.id === it.ev.resourceId) || {}).name || fmtText(it.ev.resourceId) }),
          h('td', { class: 'mono', text: clockMs(it.ev.startDate) }),
          h('td', { class: 'mono num', text: fmtUs(it.ev.durationUs) }),
          h('td', { text: fmtText(it.ev.cls) })));
      }
    }
    replace(tableHost, h('table', { class: 'data' },
      h('caption', { class: 'sr-only', text: 'Timeline events for the orders shown above' }),
      h('thead', null, h('tr', null, ['Order', 'Resource', 'Start (UTC)', 'Duration', 'Class'].map((t) => h('th', { scope: 'col', text: t })))),
      h('tbody', null, rows)));
  }

  function schedule() {
    if (!frame) frame = requestAnimationFrame(render);
  }

  scaleBtn.addEventListener('click', () => {
    shared = !shared;
    scaleBtn.setAttribute('aria-pressed', String(shared));
    scaleBtn.textContent = shared ? 'Scale: shared across orders' : 'Scale: per order';
    schedule();
  });

  if ('ResizeObserver' in window) {
    new ResizeObserver(() => {
      if (Math.floor(host.clientWidth) !== lastWidth) schedule();
    }).observe(host);
  }

  return { setStore, addEvents, refresh: schedule };
}
