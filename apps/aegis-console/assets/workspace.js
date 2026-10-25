// Panel 1: Live Speculative Workspace & Self-Healing.
// Scenario buttons come from GET /api/workspace; each run's stage pipeline is filled from the daemon's
// run history (GET /api/runs, on load, reconnect and after a lag notice) and live run.stage events,
// merged by run_id + seq so no message is applied twice.

import { h, replace, badge, riskBar, chips, kvList, fmtMs, fmtUs, fmtNum, fmtText, fmtTime, fmtValue, humanize, isNum, DASH, debounce } from './dom.js';
import { get, post, describeError } from './api.js';

const SLOTS = [
  { key: 'branch', label: 'Branch', stages: ['branch'] },
  { key: 'edit', label: 'Edit', stages: ['edit'] },
  { key: 'diff', label: 'Diff', stages: ['diff'] },
  { key: 'features', label: 'Features', stages: ['features'] },
  { key: 'score', label: 'Score', stages: ['score'] },
  { key: 'policy', label: 'Policy', stages: ['policy'] },
  { key: 'verify', label: 'Verify / recover', stages: ['verify', 'recover'] },
  { key: 'decision', label: 'Approval / commit / rollback', stages: ['approval', 'commit', 'rollback'] },
  { key: 'reap', label: 'Reap', stages: ['reap'] },
];
const STAGE_TO_SLOT = new Map(SLOTS.flatMap((slot) => slot.stages.map((st) => [st, slot.key])));

const STATUS = {
  ok: ['OK', 'ok'],
  fail: ['FAIL', 'fail'],
  skip: ['skipped', 'skip'],
  pending: ['pending', 'pending'],
};
const OUTCOME = {
  committed: ['Committed', 'ok'],
  rolled_back: ['Rolled back', 'warn'],
  pending_approval: ['Pending approval', 'pending'],
  recovered: ['Recovered', 'ok'],
  error: ['Error', 'fail'],
};
// Runs shown; GET /api/runs `limit` replaces it (the daemon keeps that many).
const DEFAULT_MAX_RUNS = 10;
const MAX_LOG = 150;

const NOT_OBSERVED = ['not observed', 'neutral'];
const GAP_REASONS = { disconnect: 'the live stream disconnected', lag: 'the daemon dropped live messages for this page (it fell behind)' };

function statusBadge(status) {
  const [label, kind] = STATUS[status] || [fmtText(status), 'neutral'];
  return badge(label, kind);
}

/** Number of a run id (`r-000042` → 42), for newest-first ordering; -1 when it has none. */
export function runNumber(runId) {
  const m = /(\d+)$/.exec(String(runId));
  return m ? Number(m[1]) : -1;
}

/** Newest first: higher run number first, then by id text. */
export function compareRuns(a, b) {
  return (runNumber(b) - runNumber(a)) || (b < a ? -1 : b > a ? 1 : 0);
}

/**
 * True when the run's messages that carry `seq` (API.md) leave no hole: 1 (run.started) up to the
 * highest seq seen or reported by the history. Runs without seq information (older daemons) pass.
 */
export function seqComplete(run) {
  if (!run.seqs || run.seqs.size === 0) return true;
  const top = Math.max(...run.seqs, isNum(run.lastSeq) ? run.lastSeq : 0);
  for (let i = 1; i <= top; i += 1) if (!run.seqs.has(i)) return false;
  return true;
}

/**
 * Adds one run.stage event to `run.events`, kept in seq order. Returns false (and changes nothing)
 * when the event is already there: same seq, or, for events without seq, the same stage, status,
 * ms, ts and detail.
 */
export function mergeStage(run, ev) {
  if (isNum(ev.seq)) {
    if (run.seqs.has(ev.seq)) return false;
    run.seqs.add(ev.seq);
    let i = run.events.length;
    while (i > 0 && isNum(run.events[i - 1].seq) && run.events[i - 1].seq > ev.seq) i -= 1;
    run.events.splice(i, 0, ev);
    return true;
  }
  const detail = JSON.stringify(ev.detail);
  const dup = run.events.some((e) => !isNum(e.seq) && e.stage === ev.stage && e.status === ev.status
    && e.ms === ev.ms && e.ts === ev.ts && JSON.stringify(e.detail) === detail);
  if (dup) return false;
  run.events.push(ev);
  return true;
}

/**
 * Applies one run.finished. The outcome and total come from the highest-seq run.finished (a stale
 * history response cannot undo a newer live one); queued_ms is kept from whichever carries one.
 * Returns false for a run.finished already applied.
 */
export function applyFinished(run, f) {
  if (isNum(f.seq)) {
    if (run.seqs.has(f.seq)) return false;
    run.seqs.add(f.seq);
  }
  if (isNum(f.queued_ms)) run.queued_ms = f.queued_ms;
  if (isNum(f.seq) && f.seq < run.outcomeSeq) return true;
  if (isNum(f.seq)) run.outcomeSeq = f.seq;
  run.outcome = f.outcome;
  run.total_ms = f.total_ms;
  return true;
}

const stageEvent = (m) => ({ seq: m.seq, stage: m.stage, status: m.status, ms: m.ms, detail: m.detail, ts: m.ts });

/**
 * True when this page has every event of the run: it saw run.started (live, or through the daemon's
 * run history), no stream gap (disconnect or dropped messages) is still unresolved by a later history
 * load, and no seq is missing. A run older than the history whose first event seen live is mid-way is
 * only partially observed.
 */
export function runComplete(run) {
  return Boolean(run.observedStart) && !run.gap && seqComplete(run);
}

/**
 * Slot status. A slot shows its last event's status, except that an unrecovered failure keeps the
 * slot red. A slot without events reads "not run" (finished) or "waiting" only when the page saw the
 * whole run; otherwise the stage may have run unseen, so it reads "not observed".
 */
export function slotStatus(events, finished, complete = true) {
  if (!events.length) {
    if (!complete) return NOT_OBSERVED;
    return finished ? ['not run', 'skip'] : ['waiting', 'neutral'];
  }
  const last = events[events.length - 1];
  if (last.status !== 'ok' && events.some((e) => e.status === 'fail')) return STATUS.fail;
  return STATUS[last.status] || [fmtText(last.status), 'neutral'];
}

/**
 * Run badge. Without run.finished the run reads "running" only while the page can trust it saw the
 * whole stream; after a gap, or when the first event seen was the post-finish reap, the outcome is
 * unknown to this page.
 */
export function runOutcome(run) {
  if (run.outcome) return OUTCOME[run.outcome] || [fmtText(run.outcome), 'neutral'];
  const last = run.events[run.events.length - 1];
  if (run.gap || (!run.observedStart && last && last.stage === 'reap')) return ['outcome not observed', 'neutral'];
  return ['running', 'pending'];
}

export function createWorkspacePanel() {
  const root = document.getElementById('panel-workspace');
  const $ = (role) => root.querySelector(`[data-role="${role}"]`);
  const els = {
    error: $('error'), summary: $('summary'), scenarios: $('scenarios'), status: $('action-status'),
    pool: $('pool'), reaper: $('reaper'), runs: $('runs'), runsEmpty: $('runs-empty'), log: $('eventlog'), reset: $('reset'),
  };

  let workspace = null;
  let services = null;
  let counters = null;
  let maxRuns = DEFAULT_MAX_RUNS;
  let gapGen = 0; // bumped by every stream gap; a history load clears gaps from before it started
  const runs = new Map();
  const order = [];
  const busy = new Set();

  function showError(msg) {
    els.error.hidden = !msg;
    els.error.textContent = msg || '';
  }

  // ---- event log -------------------------------------------------------
  function log(level, text, ts) {
    const li = h('li', { class: `log log--${level}` },
      h('time', { class: 'log__time mono', datetime: ts || new Date().toISOString(), text: fmtTime(ts || new Date().toISOString()) }),
      h('span', { class: `log__level log__level--${level}`, text: level.toUpperCase() }),
      h('span', { class: 'log__text', text }));
    els.log.prepend(li);
    while (els.log.children.length > MAX_LOG) els.log.lastElementChild.remove();
  }

  // ---- workspace summary, pool, reaper ---------------------------------
  function renderSummary() {
    if (!workspace) return;
    const versions = Array.isArray(workspace.versions) ? workspace.versions : [];
    const latest = versions.reduce((best, v) => (best === null || (isNum(v.number) && v.number > best.number) ? v : best), null);
    replace(els.summary, kvList([
      ['Live tree', h('span', { class: 'mono break', text: fmtText(workspace.live) })],
      ['Files', fmtNum(workspace.files)],
      ['Versions', versions.length ? `${versions.length} (latest #${fmtText(latest && latest.number)} at ${fmtTime(latest && latest.created_at)})` : DASH],
    ], 'kv kv--inline'));
  }

  function renderPool() {
    const pool = (workspace && workspace.pool) || {};
    const reaper = (workspace && workspace.reaper) || {};
    const ready = isNum(pool.ready) && isNum(pool.capacity) ? `${pool.ready} / ${pool.capacity}` : DASH;
    const poolRows = [
      ['Ready / capacity', ready],
      ['Hits', fmtNum(pool.hits)],
      ['Misses', fmtNum(pool.misses)],
      ['Stale discards', fmtNum(pool.stale_discards)],
      ['Last replenish', fmtMs(pool.last_replenish_ms)],
    ];
    if (counters) poolRows.push(['Live hits / misses', `${fmtNum(counters.pool_hits)} / ${fmtNum(counters.pool_misses)}`]);
    replace(els.pool, ...kvList(poolRows).children);
    replace(els.reaper, ...kvList([
      ['Queued', fmtNum(reaper.queued)],
      ['Reaped', fmtNum(reaper.reaped)],
      ['Last reap', fmtMs(reaper.last_reap_ms)],
    ]).children);
  }

  function renderScenarios() {
    const list = (workspace && Array.isArray(workspace.scenarios)) ? workspace.scenarios : [];
    if (!list.length) {
      replace(els.scenarios, h('p', { class: 'muted', text: 'The daemon returned no workspace scenarios.' }));
      return;
    }
    const focused = document.activeElement && els.scenarios.contains(document.activeElement) ? document.activeElement.dataset.scenario : null;
    replace(els.scenarios, list.map((sc) => h('button', {
      type: 'button', class: 'scenario', dataset: { scenario: sc.id }, disabled: busy.has(sc.id),
      'aria-labelledby': `ws-sc-${sc.id}-title`, 'aria-describedby': `ws-sc-${sc.id}-desc${sc.expected ? ` ws-sc-${sc.id}-exp` : ''}`,
      onclick: (ev) => runScenario(sc, ev.currentTarget),
    },
    h('span', { class: 'scenario__title', id: `ws-sc-${sc.id}-title`, text: fmtText(sc.title || sc.id) }),
    h('span', { class: 'scenario__desc', id: `ws-sc-${sc.id}-desc`, text: fmtText(sc.description) }),
    sc.expected ? h('span', { class: 'scenario__expected', id: `ws-sc-${sc.id}-exp`, text: `Expected: ${sc.expected}` }) : null,
    h('span', { class: 'scenario__id mono', text: sc.id }))));
    if (focused) {
      const again = els.scenarios.querySelector(`[data-scenario="${CSS.escape(focused)}"]`);
      if (again && !again.disabled) again.focus();
    }
  }

  async function runScenario(sc, btn) {
    busy.add(sc.id);
    btn.disabled = true;
    els.status.textContent = `Starting ${sc.title || sc.id}…`;
    try {
      const res = await post('/api/workspace/run', { scenario: sc.id });
      els.status.textContent = `Run ${fmtText(res && res.run_id)} started (${sc.id}); stages stream in below.`;
      if (res && res.run_id) ensureRun(res.run_id, { scenario: sc.id, title: sc.title });
    } catch (err) {
      els.status.textContent = `Could not start ${sc.id}: ${describeError(err)}`;
      log('error', `run ${sc.id} failed to start: ${describeError(err)}`);
    } finally {
      busy.delete(sc.id);
      const current = els.scenarios.querySelector(`[data-scenario="${CSS.escape(sc.id)}"]`) || btn;
      current.disabled = false;
      if (current.isConnected && (!document.activeElement || document.activeElement === document.body)) current.focus();
    }
  }

  async function reset() {
    els.reset.disabled = true;
    els.status.textContent = 'Resetting workspace…';
    try {
      setWorkspace(await post('/api/workspace/reset', {}));
      els.status.textContent = 'Workspace reset.';
      log('info', 'workspace reset');
    } catch (err) {
      els.status.textContent = `Reset failed: ${describeError(err)}`;
    } finally {
      els.reset.disabled = false;
    }
  }
  els.reset.addEventListener('click', reset);

  const refresh = debounce(async () => {
    try {
      setWorkspace(await get('/api/workspace'));
    } catch (err) {
      showError(`GET /api/workspace: ${describeError(err)}`);
    }
  }, 300);

  // ---- runs ------------------------------------------------------------
  /** Sorts `order` newest first, drops runs beyond `maxRuns` and puts the cards in that order. */
  function sortAndTrim() {
    order.sort(compareRuns);
    while (order.length > maxRuns) {
      const old = order.pop();
      const r = runs.get(old);
      if (r && r.el) r.el.remove();
      runs.delete(old);
    }
    for (const id of order) {
      const r = runs.get(id);
      if (r && r.el) els.runs.appendChild(r.el);
    }
    els.runsEmpty.hidden = runs.size > 0;
  }

  function ensureRun(runId, init = {}) {
    let run = runs.get(runId);
    if (!run) {
      run = {
        run_id: runId, scenario: init.scenario || null, title: init.title || null, events: [], outcome: null, total_ms: null, queued_ms: null,
        started: init.ts || null, observedStart: false, gap: null, el: null, seqs: new Set(), outcomeSeq: 0, lastSeq: null,
      };
      runs.set(runId, run);
      order.push(runId);
      sortAndTrim();
    } else {
      if (init.scenario && !run.scenario) run.scenario = init.scenario;
      if (init.title && !run.title) run.title = init.title;
      if (init.ts && !run.started) run.started = init.ts;
    }
    return run;
  }

  function stageDetail(stage, detail) {
    const d = detail && typeof detail === 'object' ? detail : {};
    const rows = [];
    const used = new Set();
    const use = (k) => { used.add(k); return d[k]; };
    if (stage === 'branch' && 'source' in d) {
      const src = use('source');
      rows.push(['Source', src === 'pool' ? badge('warm pool', 'ok') : src === 'cold' ? badge('cold clonefile', 'warn') : fmtText(src)]);
    }
    if (stage === 'score') {
      if ('risk' in d) rows.push(['Risk', riskBar(use('risk'))]);
      if ('units' in d) rows.push(['Compute units', h('span', { class: 'mono', text: fmtText(use('units')) })]);
    }
    if (stage === 'policy') {
      if ('allowed' in d) {
        const allowed = use('allowed');
        rows.push(['Cedar', allowed === true ? badge('Allow', 'ok') : allowed === false ? badge('Deny', 'fail') : DASH]);
      }
      if ('determining_policies' in d) rows.push(['Determining', chips(use('determining_policies'))]);
      if ('eval_us' in d) rows.push(['Eval', fmtUs(use('eval_us'))]);
    }
    if (stage === 'recover') {
      const tier = use('tier');
      if (tier !== undefined) rows.push(['Tier', isNum(tier) ? `tier ${tier}` : fmtText(tier)]);
      if ('model' in d) rows.push(['Model', h('span', { class: 'mono break', text: fmtText(use('model')) })]);
      if ('enabled' in d || 'reason' in d) {
        const enabled = use('enabled');
        const reason = use('reason');
        const label = isNum(tier) && tier >= 1 ? 'Nemotron' : 'Service';
        rows.push([label, enabled === true ? badge('enabled', 'ok', reason || undefined)
          : h('span', null, badge('disabled', 'skip'), reason ? h('span', { class: 'reason', text: ` ${reason}` }) : null)]);
        if (enabled === true && reason) rows.push(['Reason', fmtText(reason)]);
      }
      // Tavily backs tier-2 retrieval: show the run's own report if present, else the service status for remote tiers.
      const tav = d.tavily && typeof d.tavily === 'object' ? use('tavily') : (isNum(tier) && tier >= 1 && services ? services.tavily : null);
      if (tav) {
        rows.push([d.tavily ? 'Tavily' : 'Tavily (service)', tav.enabled === true ? badge('enabled', 'ok')
          : h('span', null, badge('disabled', 'skip'), tav.reason ? h('span', { class: 'reason', text: ` ${tav.reason}` }) : null)]);
      }
      if ('patch_applied' in d) {
        const p = use('patch_applied');
        rows.push(['Patch', p === true ? badge('applied', 'ok') : p === false ? badge('not applied', 'skip') : DASH]);
      }
    }
    for (const [k, v] of Object.entries(d)) {
      if (!used.has(k)) rows.push([humanize(k), fmtValue(k, v)]);
    }
    return rows.length ? kvList(rows, 'kv kv--stage') : null;
  }

  function partialNote(run) {
    const parts = [];
    if (!run.observedStart) parts.push('This page did not see this run start (it began before the page loaded or while the live stream was down), and it is not in the daemon\'s run history (GET /api/runs keeps only the newest runs).');
    if (run.gap) parts.push(`At ${fmtTime(run.gap.ts)} ${GAP_REASONS[run.gap.reason] || fmtText(run.gap.reason)}, so later events may be missing${run.outcome ? ' and the outcome may be out of date' : ''} until the run history is read again.`);
    else if (run.observedStart && !seqComplete(run)) parts.push('Some of this run\'s messages were not received (its seq numbers have gaps).');
    parts.push('Stages without events read “not observed”, not “not run”.');
    return h('p', { class: 'run__note small muted', role: 'note', text: parts.join(' ') });
  }

  function renderRun(run) {
    if (runs.get(run.run_id) !== run) return; // trimmed: older than every run shown
    const [outLabel, outKind] = runOutcome(run);
    const finished = Boolean(run.outcome);
    const complete = runComplete(run);
    const titleId = `run-${run.run_id}-title`;
    const named = run.title || run.scenario;
    const pipeline = h('ol', { class: 'pipeline', 'aria-label': `Stages of ${run.run_id}` }, SLOTS.map((slot) => {
      const evs = run.events.filter((e) => STAGE_TO_SLOT.get(e.stage) === slot.key);
      const [label, kind] = slotStatus(evs, finished, complete);
      const multi = slot.stages.length > 1;
      return h('li', { class: `stage stage--${kind}` },
        h('div', { class: 'stage__head' }, h('span', { class: 'stage__name', text: slot.label }), badge(label, kind)),
        evs.length ? evs.map((e) => h('div', { class: 'stage__event' },
          multi || evs.length > 1 ? h('div', { class: 'stage__sub' }, h('span', { class: 'mono', text: e.stage }), statusBadge(e.status)) : null,
          h('div', { class: 'stage__ms mono', text: isNum(e.ms) ? fmtMs(e.ms) : DASH }),
          stageDetail(e.stage, e.detail))) : h('div', { class: 'stage__ms mono muted', text: DASH }));
    }));
    const el = h('article', { class: `run run--${outKind}`, 'aria-labelledby': titleId },
      h('header', { class: 'run__head' },
        h('h4', { id: titleId, class: 'run__title', text: named ? fmtText(named) : `Run ${run.run_id}` }),
        h('span', { class: 'run__meta mono', text: named ? [run.run_id, run.scenario].filter(Boolean).join(' · ') : 'scenario not observed' }),
        badge(outLabel, outKind),
        h('span', { class: 'run__total mono', text: `total ${fmtMs(run.total_ms)}` }),
        h('span', { class: 'run__queued mono muted', title: 'time the run waited behind earlier runs or a reset before it started (not part of total)', text: `queued ${fmtMs(run.queued_ms)}` })),
      complete ? null : partialNote(run),
      pipeline);
    if (run.el) run.el.replaceWith(el);
    else {
      const idx = order.indexOf(run.run_id);
      const nextId = order.slice(0, idx).reverse().find((id) => runs.get(id) && runs.get(id).el);
      const anchor = nextId ? runs.get(nextId).el.nextSibling : els.runs.firstChild;
      els.runs.insertBefore(el, anchor);
    }
    run.el = el;
    els.runsEmpty.hidden = runs.size > 0;
  }

  // ---- WebSocket messages ----------------------------------------------
  function onMessage(msg) {
    switch (msg.type) {
      case 'run.started': {
        const run = ensureRun(msg.run_id, { scenario: msg.scenario, title: msg.title, ts: msg.ts });
        const seen = isNum(msg.seq) && run.seqs.has(msg.seq);
        if (isNum(msg.seq)) run.seqs.add(msg.seq);
        run.observedStart = true;
        renderRun(run);
        if (!seen) log('info', `run ${msg.run_id} started: ${fmtText(msg.title || msg.scenario)}`, msg.ts);
        break;
      }
      case 'run.stage': {
        const run = ensureRun(msg.run_id, { ts: msg.ts });
        if (!mergeStage(run, stageEvent(msg))) break; // already loaded from GET /api/runs
        renderRun(run);
        const level = msg.status === 'fail' ? 'warn' : 'info';
        log(level, `${msg.run_id} ${msg.stage}: ${fmtText(msg.status)}${isNum(msg.ms) ? ` in ${fmtMs(msg.ms)}` : ''}`, msg.ts);
        if (msg.stage === 'reap') refresh();
        break;
      }
      case 'run.finished': {
        const run = ensureRun(msg.run_id, { ts: msg.ts });
        // The second run.finished after an approval resolution carries queued_ms: null; the wait
        // reported by the first one still describes this run, so it is kept (applyFinished).
        if (!applyFinished(run, msg)) break; // already loaded from GET /api/runs
        renderRun(run);
        const [label] = OUTCOME[msg.outcome] || [msg.outcome];
        log(msg.outcome === 'error' ? 'error' : 'info', `run ${msg.run_id} finished: ${fmtText(label)} (total ${fmtMs(msg.total_ms)}${isNum(msg.queued_ms) ? `, queued ${fmtMs(msg.queued_ms)}` : ''})`, msg.ts);
        refresh();
        break;
      }
      case 'approval.created':
        if (msg.approval) {
          nameFromApproval(msg.approval);
          log('warn', `approval ${msg.approval.id} requested: ${fmtText(msg.approval.title)}`, msg.ts);
        }
        break;
      case 'approval.resolved':
        if (msg.approval) {
          const a = msg.approval;
          nameFromApproval(a);
          const result = a.result ? ` → ${fmtText(a.result.action)} in ${fmtMs(a.result.ms)}` : '';
          log('info', `approval ${a.id} ${fmtText(a.status)} by ${fmtText(a.resolved_by)}${result}`, msg.ts);
          refresh();
        }
        break;
      case 'config.changed':
        log('info', 'configuration changed', msg.ts);
        break;
      case 'log':
        log(['info', 'warn', 'error'].includes(msg.level) ? msg.level : 'info', fmtText(msg.message), msg.ts);
        break;
      default:
        break;
    }
  }

  // A run first seen mid-way has no title; its approval (same run_id) carries the daemon's description.
  function nameFromApproval(a) {
    const run = a && a.run_id ? runs.get(a.run_id) : null;
    if (!run || run.title || run.scenario || !a.title) return;
    run.title = a.title;
    if (run.el) renderRun(run);
  }

  /**
   * The live stream had a gap (socket closed, or the daemon dropped messages for this page): every
   * tracked run may have missed events, and the daemon offers no replay, so mark them partial.
   */
  function markGap(reason, ts) {
    const at = ts || new Date().toISOString();
    gapGen += 1;
    for (const run of runs.values()) {
      if (!run.gap) run.gap = { reason, ts: at, gen: gapGen };
      else run.gap.gen = gapGen;
      if (run.el) renderRun(run);
    }
  }

  /**
   * Merges GET /api/runs into the panel. `gen` is the gap generation when the request was made: the
   * daemon's history covers every message broadcast before it answered, so gaps up to `gen` are
   * filled for the runs it lists; a gap marked while the request was in flight stays.
   */
  function setRuns(data, gen = gapGen) {
    const list = Array.isArray(data && data.runs) ? data.runs : [];
    if (isNum(data && data.limit) && data.limit > 0) maxRuns = data.limit;
    const touched = [];
    for (const rec of list) {
      if (!rec || typeof rec.run_id !== 'string') continue;
      const run = ensureRun(rec.run_id, { scenario: rec.scenario, title: rec.title, ts: rec.started_at });
      run.observedStart = true;
      run.seqs.add(1); // a history record starts with the run's run.started (seq 1)
      if (isNum(rec.last_seq)) run.lastSeq = Math.max(isNum(run.lastSeq) ? run.lastSeq : 0, rec.last_seq);
      for (const m of Array.isArray(rec.stages) ? rec.stages : []) if (m) mergeStage(run, stageEvent(m));
      for (const f of Array.isArray(rec.finished) ? rec.finished : []) if (f) applyFinished(run, f);
      if (run.gap && run.gap.gen <= gen) run.gap = null;
      touched.push(run);
    }
    sortAndTrim();
    for (const run of touched) renderRun(run);
    els.runsEmpty.hidden = runs.size > 0;
  }

  function failRuns(err) {
    log('warn', `GET /api/runs: ${describeError(err)}; runs that started before this page loaded are not shown`);
  }

  /** GET /api/runs → setRuns, remembering which stream gaps the answer can fill. */
  async function loadRuns() {
    const gen = gapGen;
    try {
      setRuns(await get('/api/runs'), gen);
    } catch (err) {
      failRuns(err);
    }
  }

  function setWorkspace(ws) {
    workspace = ws;
    showError('');
    renderSummary();
    renderScenarios();
    renderPool();
  }

  function setServices(svc) {
    services = svc || null;
  }

  function setCounters(c) {
    counters = c || null;
    renderPool();
  }

  function fail(err) {
    showError(`GET /api/workspace: ${describeError(err)}`);
    if (!workspace) replace(els.scenarios, h('p', { class: 'muted', text: 'Scenarios unavailable until the daemon answers.' }));
  }

  return { setWorkspace, setServices, setCounters, onMessage, markGap, setRuns, loadRuns, failRuns, fail, log, refresh };
}
