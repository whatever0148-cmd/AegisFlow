// Hero signal path: the five stages of a governed agent step, lit by real events.
//
// Workspace runs (run.stage messages, or GET /api/runs after a reload) light
//   01 APFS CoW branch → 02 ANE score → 03 Cedar policy → 04 RENAME_SWAP commit / rollback
//   → 05 Tavily + Nemotron auto-heal (only when recover stages run)
// with the ms / µs each stage measured in that run, coloured by outcome. PayPal orders (paypal.order,
// or the newest row of GET /api/paypal/orders) map their pipeline onto the same bar:
//   redact → ANE score → Cedar → ed25519 sign + verify → submit.
// Idle nodes show this daemon's live p50 for the stage, or an em dash. Every number is a daemon value.

import { h, s, replace, fmtDur, durParts, fmtNum, fmtText, isNum, DASH, prefersReducedMotion } from './dom.js';
import { liveP50Ms } from './cockpit.js';
import { runNumber } from './workspace.js';

export const WORKSPACE_NODES = [
  { key: 'branch', title: 'APFS CoW branch' },
  { key: 'score', title: 'ANE score · window model' },
  { key: 'policy', title: 'Cedar policy' },
  { key: 'commit', title: 'RENAME_SWAP commit · rollback' },
  { key: 'heal', title: 'Tavily + Nemotron auto-heal' },
];
export const PAYPAL_NODES = [
  { key: 'redact', title: 'Redact' },
  { key: 'score', title: 'ANE score' },
  { key: 'policy', title: 'Cedar policy' },
  { key: 'sign', title: 'ed25519 sign · verify' },
  { key: 'submit', title: 'Submit · Orders v2' },
];
const PAYPAL_CAPTION = 'PayPal order pipeline mapped onto the bar: redact → ANE score → Cedar → ed25519 sign + verify → submit. '
  + 'The middleware runs redact, sign, verify, score, policy, then submit; each node shows that stage\'s measured time.';
const WORKSPACE_CAPTION = 'Workspace run: each node shows the time its stage measured in this run; node 05 lights only when the run enters the recovery tiers.';
const STAGGER_MS = 170;
const MAX_RUNS = 12;

const blank = () => ({ state: 'waiting', ms: null, sub: '', src: '' });

/**
 * Node values for one workspace run from its run.stage events (seq order) and outcome.
 * Returns {nodes:[5], lit:[node index per event, or -1]}; pure, so it is unit-tested.
 */
export function workspaceNodes(events, outcome) {
  const nodes = WORKSPACE_NODES.map(blank);
  const lit = [];
  const set = (i, v) => { Object.assign(nodes[i], v, { src: 'this run' }); lit.push(i); };
  for (const ev of events) {
    const d = ev.detail && typeof ev.detail === 'object' ? ev.detail : {};
    const ok = ev.status === 'ok';
    switch (ev.stage) {
      case 'branch':
        set(0, { state: ok ? 'ok' : 'fail', ms: ev.ms, sub: d.source === 'pool' ? 'warm pool hit' : d.source === 'cold' ? 'cold clonefile' : fmtText(d.source) });
        break;
      case 'score':
        set(1, { state: ok ? 'ai' : 'fail', ms: ev.ms, sub: `risk ${fmtNum(d.risk)}${d.units ? ` · ${d.units}` : ''}` });
        break;
      case 'policy': {
        const policies = Array.isArray(d.determining_policies) && d.determining_policies.length ? d.determining_policies.join(', ') : 'no permit matched';
        const verdict = d.allowed === true ? (d.team_approval_granted ? 'Allow · team approval' : 'Allow') : 'Deny';
        set(2, { state: d.allowed === true ? 'ok' : 'fail', ms: isNum(d.eval_us) ? d.eval_us / 1000 : ev.ms, sub: `${verdict} · ${policies}` });
        break;
      }
      case 'approval':
        if (ev.status === 'pending') set(2, { state: 'hold', sub: 'team approval hold' });
        else if (ev.status === 'ok') set(2, { state: 'ok', sub: 'team approval granted' });
        else if (ev.status === 'fail') set(2, { state: 'fail', sub: 'team approval rejected' });
        else set(2, { state: 'skip', sub: 'approval withdrawn (reset)' });
        break;
      case 'verify':
        if (nodes[3].state === 'waiting') {
          Object.assign(nodes[3], { state: 'active', sub: `verify ${fmtText(ev.status)}${isNum(ev.ms) ? ` · ${fmtDur(ev.ms)}` : ''}` });
          lit.push(-1);
        } else lit.push(-1);
        break;
      case 'recover': {
        const tier = d.tier;
        if (tier === 0) set(4, { state: ok ? 'ai' : 'fail', ms: ev.ms, sub: 'tier 0 · local triage (CoreML + Cedar)' });
        else if (ev.status === 'skip' && d.enabled === false) {
          Object.assign(nodes[4], { sub: `tier 0 only · remote tiers disabled` });
          lit.push(4);
        } else if (ok) set(4, { state: 'ok', ms: ev.ms, sub: `tier ${fmtText(tier)} · ${fmtText(d.model)} · patch committed` });
        else if (ev.status === 'fail') set(4, { state: 'fail', ms: ev.ms, sub: `tier ${fmtText(tier)} · no verified patch` });
        else lit.push(-1);
        break;
      }
      case 'commit':
        set(3, { state: ok ? 'ok' : 'fail', ms: ev.ms, sub: ok ? 'RENAME_SWAP' : d.conflict ? 'commit conflict' : 'commit failed' });
        break;
      case 'rollback':
        set(3, { state: 'fail', ms: ev.ms, sub: 'rollback · detach' });
        break;
      default:
        lit.push(-1);
    }
  }
  if (outcome) {
    nodes.forEach((n, i) => {
      if (n.state === 'waiting' || n.state === 'active') {
        if (i === 3 && outcome === 'pending_approval') Object.assign(n, { state: 'hold', sub: 'branch held for team approval' });
        else if (i === 4) Object.assign(n, { state: 'skip', sub: 'no build failure: not needed' });
        else Object.assign(n, { state: 'skip', sub: 'not reached' });
      }
    });
  }
  return { nodes, lit };
}

const SUBMISSION_STATE = { sent: 'ok', blocked: 'fail', error: 'fail', not_sent: 'skip' };

/** Node values for one PayPal order row (OrderRow fields, all measured by the middleware). */
export function paypalNodes(row) {
  const us = (v) => (isNum(v) ? v / 1000 : null);
  const allowed = row.decision === 'Allow';
  const signMs = isNum(row.sign_us) && isNum(row.verify_us) ? (row.sign_us + row.verify_us) / 1000 : null;
  const target = row.submission_target ? ` · ${row.submission_target}` : '';
  return [
    { state: 'ok', ms: us(row.redact_us), sub: `${fmtNum(row.redactions)} field${row.redactions === 1 ? '' : 's'} [REDACTED]${row.unformatted_sensitive_fields ? ' · unformatted secret' : ''}`, src: 'this order' },
    { state: 'ai', ms: us(row.score_us), sub: `risk ${fmtNum(row.risk_score)}`, src: 'this order' },
    { state: allowed ? 'ok' : 'fail', ms: us(row.cedar_us), sub: `${fmtText(row.decision)} · ${row.policies || 'no permit matched'}`, src: 'this order' },
    { state: row.receipt_verified === true ? 'ok' : 'fail', ms: signMs, sub: `c2pa:ed25519 · ${row.receipt_verified === true ? 'verified' : 'NOT verified'}`, src: 'this order' },
    { state: SUBMISSION_STATE[row.submission] || 'skip', ms: us(row.submit_us), sub: `${fmtText(row.submission)}${target}${row.submission === 'sent' && row.submission_detail ? ` · ${row.submission_detail}` : ''}`, src: 'this order' },
  ];
}

export function createPipeline() {
  const root = document.getElementById('hero');
  if (!root) return { setLive() {}, setServices() {}, onRunMessage() {}, setRuns() {}, onOrder() {}, setOrders() {} };
  const $ = (role) => root.querySelector(`[data-role="${role}"]`);
  const pathEl = $('path');
  const ctxEl = $('context');
  const captionEl = $('caption');
  const segBtns = [...root.querySelectorAll('[data-mode]')];

  let mode = 'workspace';
  let pinned = false;
  let live = null;
  let services = null;
  let currentRun = null;
  let lastOrder = null;
  const runs = new Map(); // run_id -> {events, seqs, outcome, title, scenario, total_ms}
  const timers = [];

  // ---- static DOM: five nodes, four edges ---------------------------------
  const nodeEls = [];
  const edgeEls = [];
  const parts = [];
  for (let i = 0; i < 5; i += 1) {
    if (i > 0) {
      const edge = h('div', { class: 'edge', 'aria-hidden': 'true' },
        s('svg', { class: 'edge__svg', viewBox: '0 0 100 12', preserveAspectRatio: 'none', focusable: 'false' },
          s('line', { class: 'edge__base', x1: 0, y1: 6, x2: 100, y2: 6, 'vector-effect': 'non-scaling-stroke' }),
          s('line', { class: 'edge__pulse', x1: 0, y1: 6, x2: 100, y2: 6, 'vector-effect': 'non-scaling-stroke' })));
      edgeEls.push(edge);
      parts.push(edge);
    }
    const el = {
      idx: h('span', { class: 'node__idx', text: String(i + 1).padStart(2, '0') }),
      title: h('span', { class: 'node__title' }),
      state: h('span', { class: 'node__state', 'aria-hidden': 'true' }),
      num: h('span', { class: 'node__num' }),
      unit: h('span', { class: 'node__unit' }),
      sub: h('div', { class: 'node__sub' }),
      src: h('div', { class: 'node__src' }),
    };
    el.root = h('div', { class: 'node', role: 'listitem', dataset: { state: 'idle', node: String(i + 1) } },
      h('div', { class: 'node__top' }, el.idx, el.title, el.state),
      h('div', { class: 'node__val' }, el.num, el.unit),
      el.sub, el.src);
    nodeEls.push(el);
    parts.push(el.root);
  }
  pathEl.setAttribute('role', 'list');
  pathEl.setAttribute('aria-label', 'Pipeline stages');
  replace(pathEl, parts);

  function paintNode(i, def, n) {
    const el = nodeEls[i];
    el.title.textContent = def.title;
    el.root.dataset.state = n.state;
    const [num, unit] = durParts(n.ms);
    el.num.textContent = isNum(n.ms) ? num : DASH;
    el.unit.textContent = isNum(n.ms) ? unit : '';
    el.sub.textContent = n.sub || '';
    el.sub.title = n.sub || '';
    el.src.textContent = n.src || '';
    el.root.setAttribute('aria-label', `${def.title}: ${isNum(n.ms) ? fmtDur(n.ms) : 'no value'}${n.sub ? `, ${n.sub}` : ''}${n.src ? ` (${n.src})` : ''}`);
    if (i > 0) {
      const prev = nodeEls[i - 1].root.dataset.state;
      const lit = ['ok', 'hold', 'fail', 'ai'].includes(n.state) && ['ok', 'hold', 'fail', 'ai', 'active'].includes(prev);
      edgeEls[i - 1].dataset.state = lit ? n.state : 'idle';
    }
  }

  function fire(i) {
    if (prefersReducedMotion() || i < 0) return;
    const node = nodeEls[i].root;
    node.classList.remove('is-lit');
    if (typeof node.getBoundingClientRect === 'function') node.getBoundingClientRect();
    node.classList.add('is-lit');
    if (i > 0) {
      const edge = edgeEls[i - 1];
      edge.classList.remove('is-firing');
      if (typeof edge.getBoundingClientRect === 'function') edge.getBoundingClientRect();
      edge.classList.add('is-firing');
    }
    timers.push(setTimeout(() => { node.classList.remove('is-lit'); if (i > 0) edgeEls[i - 1].classList.remove('is-firing'); }, 800));
  }

  function clearTimers() {
    while (timers.length) clearTimeout(timers.pop());
  }

  function paint(defs, nodes) {
    defs.forEach((def, i) => paintNode(i, def, nodes[i]));
  }

  // ---- idle values ------------------------------------------------------------
  function idleWorkspace() {
    const p = (key) => liveP50Ms(live, key);
    const val = (key, sub) => {
      const ms = p(key);
      return { state: 'idle', ms, sub, src: ms === null ? 'no live samples yet' : 'live p50' };
    };
    const rb = p('rollback_detach_ms');
    const neb = services && services.nebius;
    return [
      val('branch_acquire_ms', 'warm pool or cold clonefile'),
      val('ane_score_ms', 'CoreML context-window model'),
      val('cedar_eval_us', 'Authorizer::is_authorized'),
      val('commit_swap_ms', rb === null ? 'renameatx_np RENAME_SWAP' : `rollback detach p50 ${fmtDur(rb)}`),
      { state: 'idle', ms: null, sub: neb ? `Nemotron ${neb.enabled ? `on · ${fmtText(neb.provider)}` : 'disabled'}` : 'recovery tiers', src: 'lights on build failures' },
    ];
  }

  function idlePaypal() {
    const verify = liveP50Ms(live, 'paypal_verify_us');
    const idle = (sub) => ({ state: 'idle', ms: null, sub, src: 'no order yet' });
    return [
      idle('sensitive fields'), idle('CoreML order model'), idle('budget rules'),
      { state: 'idle', ms: verify, sub: 'receipt verify', src: verify === null ? 'no order yet' : 'live p50 · verify only' },
      idle('Orders v2'),
    ];
  }

  // ---- workspace runs ----------------------------------------------------------
  function runRec(id) {
    let r = runs.get(id);
    if (!r) {
      r = { id, events: [], seqs: new Set(), outcome: null, title: null, scenario: null, total_ms: null, outcomeSeq: 0 };
      runs.set(id, r);
      while (runs.size > MAX_RUNS) {
        const oldest = [...runs.keys()].sort((a, b) => runNumber(a) - runNumber(b))[0];
        runs.delete(oldest);
      }
    }
    return r;
  }

  function addEvent(r, ev) {
    if (isNum(ev.seq)) {
      if (r.seqs.has(ev.seq)) return false;
      r.seqs.add(ev.seq);
      let i = r.events.length;
      while (i > 0 && isNum(r.events[i - 1].seq) && r.events[i - 1].seq > ev.seq) i -= 1;
      r.events.splice(i, 0, ev);
      return true;
    }
    r.events.push(ev);
    return true;
  }

  function finishRun(r, f) {
    if (isNum(f.seq)) {
      if (r.seqs.has(f.seq)) return false;
      r.seqs.add(f.seq);
      if (f.seq < r.outcomeSeq) return true;
      r.outcomeSeq = f.seq;
    }
    r.outcome = f.outcome;
    r.total_ms = f.total_ms;
    return true;
  }

  const OUTCOME_WORDS = { committed: 'committed', rolled_back: 'rolled back', pending_approval: 'held for team approval', recovered: 'recovered (patched commit)', error: 'error' };

  function workspaceContext(r) {
    const name = r.title || r.scenario || 'run';
    const status = r.outcome ? OUTCOME_WORDS[r.outcome] || r.outcome : 'streaming…';
    return [h('span', { text: `${r.id} · ${name}` }), h('span', { class: 'dim', text: ` · ${status}${isNum(r.total_ms) ? ` · total ${fmtDur(r.total_ms)}` : ''}` })];
  }

  function showWorkspace(animateLast = false) {
    if (mode !== 'workspace') return;
    captionEl.textContent = WORKSPACE_CAPTION;
    const r = currentRun ? runs.get(currentRun) : null;
    if (!r) {
      paint(WORKSPACE_NODES, idleWorkspace());
      replace(ctxEl, h('span', { class: 'dim', text: 'idle · no run yet · nodes show this daemon\'s live p50' }));
      return;
    }
    const { nodes, lit } = workspaceNodes(r.events, r.outcome);
    paint(WORKSPACE_NODES, nodes);
    replace(ctxEl, workspaceContext(r));
    if (animateLast && lit.length) fire(lit[lit.length - 1]);
  }

  function running(id) {
    const r = id ? runs.get(id) : null;
    return Boolean(r && !r.outcome);
  }

  function follow(id) {
    if (currentRun === id) return true;
    if (currentRun && running(currentRun) && runNumber(id) < runNumber(currentRun)) return false;
    currentRun = id;
    return true;
  }

  function onRunMessage(msg) {
    if (!msg || !msg.run_id) return;
    const r = runRec(msg.run_id);
    let changed = false;
    if (msg.type === 'run.started') {
      r.title = msg.title || r.title;
      r.scenario = msg.scenario || r.scenario;
      if (isNum(msg.seq)) r.seqs.add(msg.seq);
      changed = true;
      pinned = false;
      currentRun = msg.run_id;
      setMode('workspace', false);
    } else if (msg.type === 'run.stage') {
      changed = addEvent(r, { seq: msg.seq, stage: msg.stage, status: msg.status, ms: msg.ms, detail: msg.detail });
    } else if (msg.type === 'run.finished') {
      changed = finishRun(r, msg);
    }
    if (!changed || !follow(msg.run_id)) return;
    showWorkspace(msg.type === 'run.stage');
  }

  /** GET /api/runs records (newest first): stored, and the newest is shown when nothing is streaming. */
  function setRuns(list) {
    for (const rec of Array.isArray(list) ? list : []) {
      if (!rec || typeof rec.run_id !== 'string') continue;
      const r = runRec(rec.run_id);
      r.title = rec.title || r.title;
      r.scenario = rec.scenario || r.scenario;
      r.seqs.add(1);
      for (const m of Array.isArray(rec.stages) ? rec.stages : []) if (m) addEvent(r, { seq: m.seq, stage: m.stage, status: m.status, ms: m.ms, detail: m.detail });
      for (const f of Array.isArray(rec.finished) ? rec.finished : []) if (f) finishRun(r, f);
    }
    const newest = [...runs.keys()].sort((a, b) => runNumber(b) - runNumber(a))[0];
    if (newest && !(currentRun && running(currentRun))) currentRun = newest;
    showWorkspace(false);
  }

  // ---- PayPal orders --------------------------------------------------------------
  function paypalContext(row) {
    return [h('span', { text: `${fmtText(row.reference)} · ${fmtText(row.amount)} ${fmtText(row.currency)}` }),
      h('span', { class: 'dim', text: ` · ${fmtText(row.decision)} · ${fmtText(row.submission)}${isNum(row.total_us) ? ` · pipeline ${fmtDur(row.total_us / 1000)}` : ''}` })];
  }

  function showPaypal(animate = false) {
    if (mode !== 'paypal') return;
    captionEl.textContent = PAYPAL_CAPTION;
    clearTimers();
    if (!lastOrder) {
      paint(PAYPAL_NODES, idlePaypal());
      replace(ctxEl, h('span', { class: 'dim', text: 'idle · no order yet' }));
      return;
    }
    const nodes = paypalNodes(lastOrder);
    replace(ctxEl, paypalContext(lastOrder));
    if (!animate || prefersReducedMotion()) {
      paint(PAYPAL_NODES, nodes);
      return;
    }
    // Light the nodes in pipeline order, one after another, each with its measured value.
    paint(PAYPAL_NODES, nodes.map(() => ({ ...blank(), src: 'this order' })));
    nodes.forEach((n, i) => {
      timers.push(setTimeout(() => {
        if (mode !== 'paypal') return;
        paintNode(i, PAYPAL_NODES[i], n);
        fire(i);
      }, i * STAGGER_MS));
    });
  }

  function onOrder(msg) {
    if (!msg || !msg.row) return;
    lastOrder = msg.row;
    if (!pinned && !running(currentRun)) setMode('paypal', false);
    showPaypal(true);
  }

  function setOrders(data) {
    const rows = Array.isArray(data && data.rows) ? data.rows : [];
    if (rows.length) lastOrder = rows[rows.length - 1];
    showPaypal(false);
  }

  // ---- mode ------------------------------------------------------------------------------
  function setMode(next, render = true) {
    if (next !== 'workspace' && next !== 'paypal') return;
    const changed = mode !== next;
    mode = next;
    for (const b of segBtns) b.setAttribute('aria-pressed', String(b.dataset.mode === mode));
    if (changed) clearTimers();
    if (render || changed) {
      if (mode === 'workspace') showWorkspace(false); else showPaypal(false);
    }
  }

  for (const b of segBtns) {
    b.addEventListener('click', () => {
      pinned = true;
      setMode(b.dataset.mode);
    });
  }

  showWorkspace(false);

  return {
    setLive(l) {
      live = l || null;
      if (mode === 'workspace' && !currentRun) showWorkspace(false);
      if (mode === 'paypal' && !lastOrder) showPaypal(false);
    },
    setServices(svc) {
      services = svc || null;
      if (mode === 'workspace' && !currentRun) showWorkspace(false);
    },
    onRunMessage,
    setRuns,
    onOrder,
    setOrders,
    get mode() { return mode; },
  };
}
