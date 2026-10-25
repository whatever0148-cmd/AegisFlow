// Header (connection state, commit, model + MLComputePlan placement) and the global banner stack.

import { h, replace, fmtText, DASH, isNum } from './dom.js';

const banners = document.getElementById('banners');
const current = new Map(); // id -> element

/**
 * Shows (or updates) a banner. kind: 'mock' | 'error' | 'warn' | 'info'.
 * actions: [{label, onClick}]
 */
export function showBanner(id, kind, text, actions = []) {
  let el = current.get(id);
  const body = [h('span', { class: 'banner__text', text })];
  for (const a of actions) body.push(h('button', { type: 'button', class: 'btn btn--small', onclick: a.onClick, text: a.label }));
  if (!el) {
    el = h('div', { class: `banner banner--${kind}`, role: kind === 'error' ? 'alert' : 'status', dataset: { banner: id } });
    current.set(id, el);
    if (kind === 'mock') banners.prepend(el); else banners.append(el);
  }
  el.className = `banner banner--${kind}`;
  replace(el, body);
}

export function hideBanner(id) {
  const el = current.get(id);
  if (el) {
    el.remove();
    current.delete(id);
  }
}

const conn = document.getElementById('conn');
const connLabel = document.getElementById('conn-label');
const connRetry = document.getElementById('conn-retry');
let countdown = null;

export function bindRetry(fn) {
  connRetry.addEventListener('click', fn);
}

/** Renders the WebSocket state pill. */
export function setConnState(state, info = {}) {
  clearInterval(countdown);
  countdown = null;
  conn.dataset.state = state === 'waiting' ? (info.reason || 'closed') : state;
  connRetry.hidden = state !== 'waiting';
  if (state === 'connecting') {
    connLabel.textContent = info.attempt ? `Connecting… (attempt ${info.attempt + 1})` : 'Connecting…';
  } else if (state === 'open') {
    connLabel.textContent = 'Connected · live';
  } else if (state === 'waiting') {
    const prefix = {
      offline: 'Daemon offline',
      refused: 'WebSocket refused',
      closed: 'Disconnected',
    }[info.reason] || 'Disconnected';
    const tick = () => {
      const secs = Math.max(0, Math.ceil((info.until - Date.now()) / 1000));
      connLabel.textContent = `${prefix} · retry in ${secs} s (attempt ${info.attempt})`;
    };
    tick();
    countdown = setInterval(tick, 1000);
  }
}

function placement(plan) {
  if (!plan || typeof plan !== 'object') return DASH;
  const parts = [];
  const label = { neural_engine: 'ANE', cpu: 'CPU', gpu: 'GPU' };
  for (const key of ['neural_engine', 'cpu', 'gpu']) {
    if (key in plan) parts.push(`${label[key]} ${isNum(plan[key]) ? plan[key] : fmtText(plan[key])}`);
  }
  for (const [k, v] of Object.entries(plan)) {
    if (!(k in label)) parts.push(`${k} ${fmtText(v)}`);
  }
  return parts.length ? parts.join(' · ') : DASH;
}

/** Fills the header facts from GET /api/health (or hello.health). */
export function setHealth(health) {
  const model = (health && health.model) || {};
  document.getElementById('hf-commit').textContent = fmtText(health && health.git_commit);
  document.getElementById('hf-version').textContent = fmtText(health && health.version);
  const shape = Array.isArray(model.input_shape) ? ` [${model.input_shape.join('×')}]` : (model.input_shape ? ` ${model.input_shape}` : '');
  replace(document.getElementById('hf-model'),
    h('span', { class: 'mono', text: fmtText(model.name) }),
    shape ? h('span', { class: 'muted mono', text: shape }) : null);
  const units = health && health.compute_units;
  replace(document.getElementById('hf-plan'),
    h('span', { class: 'mono', text: placement(model.compute_plan) }),
    units ? h('span', { class: 'muted', text: ` (${units})` }) : null);
  document.getElementById('hf-plan').title = 'Op placement reported by MLComputePlan for the context-window model';

  if (health && health.mock === true) {
    showBanner('mock', 'mock', 'CONTRACT MOCK — synthetic values, not telemetry');
  } else {
    hideBanner('mock');
  }
}
