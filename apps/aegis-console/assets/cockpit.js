// Sticky header hardware strip: ANE placement + online-model latency, warm CoW pool, Cedar,
// Nemotron provider + Tavily, PayPal gateway mode. Every value is read at runtime from
// GET /api/health (model.compute_plan), live stats (hello / telemetry.live), the bench artifact
// (GET /api/telemetry), GET /api/workspace (pool) and GET /api/config (services); each latency is
// labelled with where it came from ("live p50" or "bench p50"). Nothing is typed in here.

import { h, replace, fmtDur, fmtNum, fmtText, isNum, DASH } from './dom.js';

/** Live p50 of a LiveStats sample, in ms (µs samples are converted), or null when it has no samples. */
export function liveP50Ms(live, key) {
  const st = live && live.samples && live.samples[key];
  if (!st || !isNum(st.n) || st.n <= 0 || !isNum(st.p50)) return null;
  return /_us$/.test(key) ? st.p50 / 1000 : st.p50;
}

/** Op placement from MLComputePlan: {ane, total} (ops on the Neural Engine of all dispatched ops). */
export function placementOf(plan) {
  if (!plan || typeof plan !== 'object') return null;
  const nums = ['neural_engine', 'cpu', 'gpu'].map((k) => plan[k]).filter(isNum);
  if (!isNum(plan.neural_engine) || !nums.length) return null;
  return { ane: plan.neural_engine, total: nums.reduce((a, b) => a + b, 0) };
}

function benchRows(bench) {
  if (!bench) return [];
  return [
    ...(Array.isArray(bench.targets_summary) ? bench.targets_summary : []),
    ...(Array.isArray(bench.informational) ? bench.informational : []),
  ].filter((r) => r && typeof r === 'object');
}

/** The bench's online-model latency row: the first ANE-section row with a measured p50 (ms). */
export function benchAneMs(bench) {
  const row = benchRows(bench).find((r) => r.section === 'ane' && isNum(r.measured_p50_ms));
  return row ? { ms: row.measured_p50_ms, metric: row.metric } : null;
}

/** The bench's warm-hit acquire rows (WarmBranchPool, per fixture). */
export function benchWarmRows(bench) {
  return benchRows(bench).filter((r) => typeof r.metric === 'string' && /warm hit/i.test(r.metric) && isNum(r.measured_p50_ms));
}

/** The bench's Cedar is_authorized p50 in ms (from the artifact's cedar section, reported in µs). */
export function benchCedarMs(bench) {
  const st = bench && bench.cedar && bench.cedar.is_authorized_us;
  return st && isNum(st.p50) ? st.p50 / 1000 : null;
}

const PAYPAL_WORDS = { local: 'local gateway', remote: 'PayPal remote', disabled: 'disabled' };

export function createCockpit() {
  const pill = (id) => {
    const el = document.getElementById(id);
    return el ? { el, value: el.querySelector('[data-role="value"]'), source: el.querySelector('[data-role="source"]') } : null;
  };
  const pills = { ane: pill('hw-ane'), pool: pill('hw-pool'), cedar: pill('hw-cedar'), ai: pill('hw-ai'), paypal: pill('hw-paypal') };
  const st = { health: null, live: null, bench: null, services: null, pool: null, connected: false };

  function set(p, { state, value, source, title, pulse }) {
    if (!p) return;
    p.el.dataset.state = state || 'idle';
    p.el.dataset.pulse = pulse ? 'on' : 'off';
    replace(p.value, value);
    p.source.textContent = source || '';
    if (title) p.el.setAttribute('title', title);
    p.el.setAttribute('aria-label', title || `${p.el.textContent.replace(/\s+/g, ' ').trim()}`);
  }

  const sep = () => h('span', { class: 'sep', 'aria-hidden': 'true', text: ' · ' });

  function renderAne() {
    const model = (st.health && st.health.model) || {};
    const place = placementOf(model.compute_plan);
    const live = liveP50Ms(st.live, 'ane_score_ms');
    const bench = benchAneMs(st.bench);
    const ms = live !== null ? live : bench && bench.ms;
    const source = live !== null ? 'live p50' : bench ? 'bench p50' : '';
    const placeText = place ? `${fmtNum(place.ane)}/${fmtNum(place.total)} OPS` : DASH;
    const state = !place ? 'idle' : place.ane === place.total && place.total > 0 ? 'ok' : place.ane > 0 ? 'warn' : 'off';
    set(pills.ane, {
      state,
      pulse: st.connected && state === 'ok',
      value: [placeText, sep(), isNum(ms) ? fmtDur(ms) : DASH],
      source,
      title: `Neural Engine placement from MLComputePlan (${place ? `${place.ane} of ${place.total} ops on the ANE` : 'not reported'}); `
        + `online window model latency ${isNum(ms) ? fmtDur(ms) : 'not measured yet'}${source ? ` (${source}${source === 'bench p50' && bench ? `: ${bench.metric}` : ''})` : ''}`,
    });
  }

  function renderPool() {
    const pool = st.pool || {};
    const ready = isNum(pool.ready) && isNum(pool.capacity) ? `${fmtNum(pool.ready)}/${fmtNum(pool.capacity)} ready` : DASH;
    const rows = benchWarmRows(st.bench);
    const warm = rows[0] || null;
    const state = !isNum(pool.ready) ? 'idle' : pool.ready > 0 ? 'ok' : 'warn';
    set(pills.pool, {
      state,
      value: [ready, sep(), warm ? fmtDur(warm.measured_p50_ms) : DASH],
      source: warm ? `bench${warm.fixture ? ` · ${warm.fixture}` : ''}` : '',
      title: `Warm copy-on-write branch pool: ${ready}; bench warm-hit acquire p50 ${warm ? `${fmtDur(warm.measured_p50_ms)} (${warm.metric})` : 'not in the artifact'}`,
    });
  }

  function renderCedar() {
    const live = liveP50Ms(st.live, 'cedar_eval_us');
    const bench = benchCedarMs(st.bench);
    const ms = live !== null ? live : bench;
    const source = live !== null ? 'live p50' : bench !== null ? 'bench p50' : '';
    set(pills.cedar, {
      state: isNum(ms) ? 'ok' : 'idle',
      value: isNum(ms) ? fmtDur(ms) : DASH,
      source,
      title: `Cedar is_authorized ${isNum(ms) ? `p50 ${fmtDur(ms)} (${source})` : 'not measured yet'}`,
    });
  }

  function renderAi() {
    const svc = st.services || {};
    const neb = svc.nebius;
    const tav = svc.tavily;
    if (!neb && !tav) { set(pills.ai, { state: 'idle', value: DASH }); return; }
    const on = neb && neb.enabled === true;
    const word = (s) => (s && s.enabled === true ? 'on' : 'off');
    set(pills.ai, {
      state: on ? 'ai' : 'off',
      value: [h('span', { text: neb && neb.provider ? neb.provider : DASH }), sep(), h('span', { class: on ? '' : 'dim', text: word(neb) }), sep(),
        h('span', { class: 'dim', text: 'Tavily ' }), h('span', { class: tav && tav.enabled ? '' : 'dim', text: word(tav) })],
      source: '',
      title: `Nemotron provider ${fmtText(neb && neb.provider)}: ${on ? 'enabled' : `disabled${neb && neb.reason ? ` (${neb.reason})` : ''}`}. `
        + `Tavily: ${tav && tav.enabled ? 'enabled' : `disabled${tav && tav.reason ? ` (${tav.reason})` : ''}`}.`,
    });
  }

  function renderPaypal() {
    const pp = st.services && st.services.paypal;
    const mode = pp && (pp.mode || (typeof pp.enabled === 'boolean' ? (pp.enabled ? 'remote' : 'disabled') : null));
    set(pills.paypal, {
      state: mode === 'local' ? 'ai' : mode === 'remote' ? 'ok' : mode === 'disabled' ? 'off' : 'idle',
      value: mode ? PAYPAL_WORDS[mode] || mode : DASH,
      source: '',
      title: `PayPal submission mode: ${mode || 'unknown'}${pp && pp.reason ? ` (${pp.reason})` : ''}`,
    });
  }

  function renderAll() {
    renderAne();
    renderPool();
    renderCedar();
    renderAi();
    renderPaypal();
  }
  renderAll();

  return {
    setHealth(health) { st.health = health || null; renderAne(); },
    setLive(live) { st.live = live || null; renderAne(); renderCedar(); },
    setTelemetry(t) { st.bench = (t && t.bench) || null; if (t && t.live) st.live = t.live; renderAll(); },
    setServices(services) { st.services = services || null; renderAi(); renderPaypal(); },
    setWorkspace(ws) { st.pool = (ws && ws.pool) || null; renderPool(); },
    setConnected(on) { st.connected = Boolean(on); renderAne(); },
  };
}
