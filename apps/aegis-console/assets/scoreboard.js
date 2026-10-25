// Panel 3 (right column): benchmark scoreboard from GET /api/telemetry (bench.targets_summary,
// bench.informational, bench.targets_note, bench.preliminary / bench.run_conditions.reasons) and the
// live LiveStats table updated by telemetry.live. A preliminary artifact carries a PRELIMINARY label.
// Failures are always rendered, in red, with the word FAIL; nothing is filtered. A gated row whose
// pass is not a boolean (the bench writes null when the measurement is missing) is not met, exactly
// as the T18 gate in harness/TASKS.json counts it.

import { h, replace, badge, fmtNum, fmtText, fmtDateTime, fmtTime, unitForKey, withUnit, humanize, isNum, DASH } from './dom.js';
import { get, describeError } from './api.js';

const SAMPLE_LABELS = {
  branch_acquire_ms: 'Branch acquire (pool or cold)',
  branch_cold_ms: 'Cold clonefile (pool miss)',
  edit_ms: 'Agent edit',
  diff_ms: 'Diff',
  ane_score_ms: 'CoreML score (context window)',
  cedar_eval_us: 'Cedar is_authorized',
  commit_swap_ms: 'Commit (RENAME_SWAP)',
  rollback_detach_ms: 'Rollback detach',
  reap_ms: 'Reaper, per detached tree',
  paypal_total_us: 'PayPal pipeline total',
  paypal_verify_us: 'Receipt verify (ed25519)',
};
const PLAN_LABELS = { neural_engine: 'ANE', cpu: 'CPU', gpu: 'GPU' };

const COUNT_LABELS = { n: 'n', hits: 'hits', misses: 'misses', dispatched_ops: 'dispatched ops', entries: 'entries' };

/** "measured_p50_ms" -> "p50", "measured_per_entry_us" -> "per entry", "measured" -> "". */
function measuredLabel(key) {
  return key.replace(/^measured_?/, '').replace(/_(ms|us|ns)$/, '').replace(/_/g, ' ').trim();
}

function fmtMeasuredValue(key, v) {
  if (isNum(v)) return withUnit(v, unitForKey(key));
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    return Object.entries(v).map(([pk, pv]) => `${PLAN_LABELS[pk] || pk} ${isNum(pv) ? fmtNum(pv) : fmtText(pv)}`).join(' · ');
  }
  return fmtText(v);
}

/**
 * Measured cell of a summary row: every measured* field labelled from its key (so a median reads
 * "p50 …"), plus the spread and sample counts the row carries (p95_ms, sys_us_p50, n, hits, misses,
 * dispatched_ops, entries).
 */
export function measuredParts(row) {
  const primary = Object.keys(row).filter((k) => k.startsWith('measured')).map((k) => {
    const label = measuredLabel(k);
    const text = fmtMeasuredValue(k, row[k]);
    return label ? `${label} ${text}` : text;
  });
  const extras = [];
  for (const [k, v] of Object.entries(row)) {
    if (k.startsWith('measured')) continue;
    const pct = /^(p\d{2,3})_(ms|us|ns)$/.exec(k);
    const inner = /^(.+)_(ms|us|ns)_(p\d{2,3})$/.exec(k);
    if (pct) extras.push(`${pct[1]} ${withUnit(v, unitForKey(k))}`);
    else if (inner) extras.push(`${inner[1].replace(/_/g, ' ')} ${inner[3]} ${withUnit(v, unitForKey(`_${inner[2]}`))}`);
    else if (Object.prototype.hasOwnProperty.call(COUNT_LABELS, k)) extras.push(`${COUNT_LABELS[k]} ${fmtNum(v)}`);
  }
  return { primary: primary.length ? primary.join('; ') : DASH, extras: extras.join(' · ') };
}

function measuredCell(row) {
  const { primary, extras } = measuredParts(row);
  return h('td', { class: 'mono', 'data-label': 'Measured' }, primary, extras ? h('span', { class: 'small muted block', text: extras }) : null);
}

/** Gated rows: anything but pass === true is not met; "unmeasured" when pass is not a boolean. */
export function gatedVerdict(pass) {
  if (pass === true) return 'pass';
  if (pass === false) return 'fail';
  return 'unmeasured';
}

export function gatedCounts(gated) {
  const counts = { total: gated.length, passed: 0, missed: 0, unmeasured: 0 };
  for (const r of gated) {
    const v = gatedVerdict(r && r.pass);
    if (v === 'pass') counts.passed += 1;
    else if (v === 'fail') counts.missed += 1;
    else counts.unmeasured += 1;
  }
  return counts;
}

/**
 * P2-R6.5: a preliminary artifact (reduced scale, busy machine, or sources changed during the run)
 * is labelled visibly, with the reasons from run_conditions. Returns null for a decisive run.
 */
export function preliminaryInfo(bench) {
  const run = (bench && bench.run_conditions && typeof bench.run_conditions === 'object') ? bench.run_conditions : {};
  if (!(bench && (bench.preliminary === true || run.preliminary === true))) return null;
  const reasons = Array.isArray(run.reasons) ? run.reasons.filter((r) => typeof r === 'string' && r) : [];
  return { reasons };
}

function preliminaryBanner(bench) {
  const info = preliminaryInfo(bench);
  if (!info) return null;
  const why = info.reasons.length ? info.reasons.join('; ') : 'run_conditions lists no reason';
  return h('p', { class: 'banner banner--warn bench-prelim', role: 'status', 'data-role': 'bench-preliminary' },
    badge('PRELIMINARY', 'warn'),
    h('span', { class: 'banner__text', text: `${why}. Only a quiet full-scale run decides the phase result.` }));
}

function resultCell(pass, gated) {
  if (!gated) {
    return h('td', { class: 'result', 'data-label': 'Result' }, badge('not gated', 'info'),
      pass === true || pass === false ? h('span', { class: 'muted small', text: ` (${pass ? 'would pass' : 'would fail'})` }) : null);
  }
  if (pass === true) return h('td', { class: 'result', 'data-label': 'Result' }, badge('PASS', 'ok'));
  if (pass === false) return h('td', { class: 'result', 'data-label': 'Result' }, badge('FAIL', 'fail'), h('span', { class: 'fail-text small', text: ' target missed' }));
  return h('td', { class: 'result', 'data-label': 'Result' }, badge('FAIL', 'fail'), h('span', { class: 'fail-text small', text: ' not measured (no result in the artifact)' }));
}

export function createScoreboard() {
  const root = document.getElementById('panel-approvals');
  const $ = (role) => root.querySelector(`[data-role="${role}"]`);
  const els = {
    error: $('bench-error'), meta: $('bench-meta'), table: $('scoreboard'), note: $('bench-note'), reload: $('reload-bench'),
    live: $('livestats'), counters: $('counters'), updated: $('live-updated'),
  };

  function setBenchError(msg) {
    els.error.hidden = !msg;
    els.error.textContent = msg || '';
  }

  function setTelemetry(t) {
    setBenchError('');
    const bench = t && t.bench;
    const tbody = els.table.tBodies[0];
    if (!bench) {
      replace(els.meta, h('p', { class: 'muted', text: `No benchmark artifact${t && t.bench_path ? ` at ${t.bench_path}` : ''}. Run the benchmark to populate the scoreboard.` }));
      replace(tbody, h('tr', null, h('td', { colspan: 4, class: 'muted', text: 'No bench data.' })));
      els.note.hidden = true;
    } else {
      const gated = Array.isArray(bench.targets_summary) ? bench.targets_summary : [];
      const info = Array.isArray(bench.informational) ? bench.informational : [];
      const { passed, missed, unmeasured } = gatedCounts(gated);
      const notMet = missed + unmeasured;
      const commit = bench.git && bench.git.commit;
      const host = bench.host ? [bench.host.cpu, bench.host.model, bench.host.macos && `macOS ${bench.host.macos}`].filter(Boolean).join(' · ') : null;
      replace(els.meta,
        preliminaryBanner(bench),
        h('p', { class: 'bench-summary' },
          h('strong', { text: `${passed} of ${gated.length} targets met` }),
          notMet ? h('span', { class: 'fail-text', text: ` · ${notMet} missed${unmeasured ? ` (${unmeasured} not measured)` : ''}` }) : null,
          info.length ? h('span', { class: 'muted', text: ` · ${info.length} informational` }) : null),
        h('p', { class: 'small muted' },
          `Artifact ${fmtText(t.bench_path)} · generated ${fmtDateTime(t.bench_generated_at || bench.generated_at)}`,
          commit ? ` · commit ${commit}${bench.git.dirty ? ' (dirty)' : ''}` : '',
          host ? ` · ${host}` : ''),
        bench.ane && bench.ane.placement_note ? h('p', { class: 'small note-inline', text: `Placement: ${bench.ane.placement_note}` }) : null);
      const rows = [
        ...gated.map((r) => ({ r, gated: true })),
        ...info.map((r) => ({ r, gated: false })),
      ];
      if (!rows.length) {
        replace(tbody, h('tr', null, h('td', { colspan: 4, class: 'muted', text: 'The artifact has no targets_summary rows.' })));
      } else {
        replace(tbody, rows.map(({ r, gated: g }) => h('tr', { class: g ? (r.pass === true ? 'row-pass' : 'row-fail') : 'row-info' },
          h('th', { scope: 'row', text: fmtText(r.metric) }),
          h('td', { class: 'mono', 'data-label': 'Target', text: fmtText(r.target) }),
          measuredCell(r),
          resultCell(r.pass, g))));
      }
      if (bench.targets_note) {
        els.note.hidden = false;
        els.note.textContent = typeof bench.targets_note === 'string' ? bench.targets_note : JSON.stringify(bench.targets_note);
      } else {
        els.note.hidden = true;
      }
    }
    if (t && t.live) setLive(t.live);
  }

  function setLive(live) {
    const samples = (live && live.samples) || {};
    const tbody = els.live.tBodies[0];
    const entries = Object.entries(samples);
    if (!entries.length) {
      replace(tbody, h('tr', null, h('td', { colspan: 5, class: 'muted', text: 'No live samples reported.' })));
    } else {
      replace(tbody, entries.map(([key, st]) => {
        const unit = unitForKey(key);
        const stat = st || {};
        return h('tr', null,
          h('th', { scope: 'row' }, h('span', { text: SAMPLE_LABELS[key] || humanize(key) }), h('span', { class: 'mono small muted block', text: key })),
          h('td', { class: 'mono num', 'data-label': 'n', text: fmtNum(stat.n) }),
          h('td', { class: 'mono num', 'data-label': 'p50', text: withUnit(stat.p50, unit) }),
          h('td', { class: 'mono num', 'data-label': 'p95', text: withUnit(stat.p95, unit) }),
          h('td', { class: 'mono num', 'data-label': 'last', text: withUnit(stat.last, unit) }));
      }));
    }
    const counters = (live && live.counters) || {};
    replace(els.counters, Object.entries(counters).map(([k, v]) => h('div', { class: 'counter' },
      h('dt', { text: humanize(k) }), h('dd', { class: 'mono', text: fmtNum(v) }))));
    els.updated.textContent = `updated ${fmtTime(new Date().toISOString())}`;
  }

  async function reload() {
    els.reload.disabled = true;
    try {
      setTelemetry(await get('/api/telemetry'));
    } catch (err) {
      fail(err);
    } finally {
      els.reload.disabled = false;
      if (!document.activeElement || document.activeElement === document.body) els.reload.focus();
    }
  }
  els.reload.addEventListener('click', reload);

  function fail(err) {
    setBenchError(`GET /api/telemetry: ${describeError(err)}`);
  }

  return { setTelemetry, setLive, fail, reload };
}
