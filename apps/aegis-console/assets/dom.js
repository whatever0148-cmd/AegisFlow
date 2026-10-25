// Small DOM and formatting helpers shared by every panel.
// Every number the console shows is formatted from an API value; missing values render as an em dash.

export const DASH = '—';

const SVG_NS = 'http://www.w3.org/2000/svg';

function appendChildren(el, children) {
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
}

function applyProps(el, props) {
  if (!props) return;
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === 'class') el.setAttribute('class', value);
    else if (key === 'text') el.textContent = String(value);
    else if (key === 'dataset') Object.assign(el.dataset, value);
    else if (key.startsWith('on') && typeof value === 'function') el.addEventListener(key.slice(2), value);
    else if (key === 'props') Object.assign(el, value);
    else if (value === true) el.setAttribute(key, '');
    else el.setAttribute(key, String(value));
  }
}

/** Creates an HTML element: h('div', {class: 'x', onclick: fn}, child, 'text'). */
export function h(tag, props, ...children) {
  const el = document.createElement(tag);
  applyProps(el, props);
  appendChildren(el, children);
  return el;
}

/** Creates an SVG element (same signature as h). */
export function s(tag, props, ...children) {
  const el = document.createElementNS(SVG_NS, tag);
  applyProps(el, props);
  appendChildren(el, children);
  return el;
}

export function clear(el) {
  while (el.firstChild) el.removeChild(el.firstChild);
  return el;
}

export function replace(el, ...children) {
  clear(el);
  appendChildren(el, children);
  return el;
}

export function isNum(v) {
  return typeof v === 'number' && Number.isFinite(v);
}

/** Adaptive precision: integers stay integers, small values keep four decimals. */
export function fmtNum(v) {
  if (!isNum(v)) return DASH;
  if (Number.isInteger(v)) return v.toLocaleString('en-US');
  const a = Math.abs(v);
  const digits = a >= 1000 ? 0 : a >= 100 ? 1 : a >= 10 ? 2 : a >= 1 ? 3 : 4;
  return v.toLocaleString('en-US', { maximumFractionDigits: digits, minimumFractionDigits: 0 });
}

export function withUnit(v, unit) {
  if (!isNum(v)) return DASH;
  return unit ? `${fmtNum(v)} ${unit}` : fmtNum(v);
}

export const fmtMs = (v) => withUnit(v, 'ms');
export const fmtUs = (v) => withUnit(v, 'µs');

export function unitForKey(key) {
  if (/(_ms)$/.test(key)) return 'ms';
  if (/(_us)$/.test(key)) return 'µs';
  if (/(_ns)$/.test(key)) return 'ns';
  return '';
}

export function fmtText(v) {
  if (v === null || v === undefined || v === '') return DASH;
  return String(v);
}

export function fmtBool(v, yes = 'yes', no = 'no') {
  if (v === true) return yes;
  if (v === false) return no;
  return DASH;
}

export function fmtTime(iso) {
  if (!iso) return DASH;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return String(iso);
  return d.toLocaleTimeString([], { hour12: false });
}

/** HH:MM:SS in UTC, for columns whose header says UTC (independent of the browser's time zone). */
export function fmtTimeUTC(iso) {
  if (!iso) return DASH;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return String(iso);
  return d.toISOString().slice(11, 19);
}

export function fmtDateTime(iso) {
  if (!iso) return DASH;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return String(iso);
  return d.toLocaleString([], { hour12: false });
}

/** "neural_engine" -> "Neural engine" */
export function humanize(key) {
  const text = String(key).replace(/_/g, ' ').replace(/\s+/g, ' ').trim();
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Renders any JSON value compactly; numbers get a unit derived from the key suffix. */
export function fmtValue(key, value) {
  if (value === null || value === undefined || value === '') return DASH;
  if (isNum(value)) return withUnit(value, unitForKey(key || ''));
  if (typeof value === 'boolean') return value ? 'yes' : 'no';
  if (Array.isArray(value)) return value.length ? value.map((v) => (typeof v === 'object' ? JSON.stringify(v) : String(v))).join(', ') : DASH;
  if (typeof value === 'object') {
    const parts = Object.entries(value).map(([k, v]) => `${k} ${typeof v === 'object' && v !== null ? JSON.stringify(v) : fmtValue(k, v)}`);
    return parts.length ? parts.join(' · ') : DASH;
  }
  return String(value);
}

const GLYPH = { ok: '✓', fail: '✗', warn: '!', pending: '…', skip: '–', info: 'i', neutral: '·', live: '●' };

/** Text badge; colour is never the only signal (glyph + label). */
export function badge(label, kind = 'neutral', title) {
  return h('span', { class: `badge badge--${kind}`, title },
    h('span', { class: 'badge__glyph', 'aria-hidden': 'true', text: GLYPH[kind] || GLYPH.neutral }),
    h('span', { class: 'badge__label', text: label }));
}

/** Risk bar on the 0-100 scale the scorer reports; the number is always printed next to it. */
export function riskBar(risk) {
  if (!isNum(risk)) return h('span', { class: 'muted', text: DASH });
  const pct = Math.max(0, Math.min(100, risk));
  const fill = h('span', { class: 'riskbar__fill' });
  fill.style.width = `${pct}%`;
  return h('span', { class: 'riskbar' },
    h('span', { class: 'riskbar__track', role: 'img', 'aria-label': `risk ${fmtNum(risk)} of 100` }, fill),
    h('span', { class: 'riskbar__value mono', text: fmtNum(risk) }));
}

export function chips(list) {
  const items = Array.isArray(list) ? list : (typeof list === 'string' && list ? list.split(/,\s*/) : []);
  if (!items.length) return h('span', { class: 'muted', text: DASH });
  return h('ul', { class: 'chips' }, items.map((p) => h('li', { class: 'chip mono', text: p })));
}

/** <dl> from [[label, valueNodeOrText], ...] */
export function kvList(rows, cls = 'kv') {
  return h('dl', { class: cls }, rows.map(([k, v]) => h('div', null, h('dt', { text: k }), h('dd', null, v === undefined ? DASH : v))));
}

export function prefersReducedMotion() {
  return window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

export function storageGet(key) {
  try { return window.localStorage.getItem(key); } catch { return null; }
}

export function storageSet(key, value) {
  try { window.localStorage.setItem(key, value); } catch { /* storage unavailable: keep in memory only */ }
}

export function debounce(fn, wait) {
  let timer = null;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), wait);
  };
}
