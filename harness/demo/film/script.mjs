// The film's timeline, cut to the voiceover. Times are seconds into the voiceover, taken from its
// word timings (voiceover_words.json, produced by transcribe.swift). Every number a callout shows
// is read from the telemetry artifact here, or patched in from the live daemon while rendering.

const SC = (text) => ({ sel: '#panel-workspace .scenario', text });
const PP = (text) => ({ sel: '#panel-paypal .scenario', text });
const NODE = (index) => ({ sel: '#hero .sigpath .node', index });
// Newest order row (the grid lists newest first): its Inspect button.
const INSPECT = { sel: '#panel-paypal .ag-row[row-index="0"] button', text: '{ }Inspect' };
const PATCH = { sel: '#panel-workspace .runs .run details.diffview', contains: 'self-heal patch' };
const PATCH_SUMMARY = { sel: '#panel-workspace .runs .run details.diffview summary', contains: 'self-heal patch' };

// Spoken words the on-device transcriber spells differently from the product names.
const FIXES = [
  [['agis', 'flow'], 'AegisFlow'], [['ages', 'flow'], 'AegisFlow'],
  [['0', 'root'], 'zero-root'], [['copy', 'on', 'right'], 'copy-on-write'],
  [['apple', 'neural', 'engine'], 'Apple Neural Engine'], [['fp', '16'], 'FP16'], [['16', 'core'], '16-core'],
  [['36', 'operation'], '36-operation'], [['in', 'line'], 'inline'], [['ed', '25519'], 'Ed25519'], [['ag', 'grid'], 'AG Grid'],
  [['cedar'], 'Cedar'], [['rust'], 'Rust'], [['nvidia'], 'NVIDIA'], [['nematron'], 'Nemotron'], [['tavoli'], 'Tavily'],
  [['alive'], 'a live'], [['redacted'], '[REDACTED]'],
];
const bare = (w) => w.trim().toLowerCase().replace(/[.,!?]+$/, '');

/** Corrected words with timings: [{w, s, e}]. Merged words span their parts; trailing punctuation of the last part is kept. */
export function fixWords(raw) {
  const words = raw.map((x) => ({ w: x.w.trim(), s: x.s, e: x.e })).filter((x) => x.w);
  const out = [];
  for (let i = 0; i < words.length;) {
    let hit = null;
    for (const [pat, rep] of FIXES) {
      if (pat.every((p, k) => words[i + k] && bare(words[i + k].w) === p)) { hit = [pat.length, rep]; break; }
    }
    if (!hit) { out.push(words[i]); i += 1; continue; }
    const [n, rep] = hit;
    const last = words[i + n - 1].w;
    const tail = /[.,!?]$/.test(last) ? last.slice(-1) : '';
    out.push({ w: rep + tail, s: words[i].s, e: words[i + n - 1].e });
    i += n;
  }
  // Spoken commas the transcriber put after single words inside a phrase.
  out.forEach((x, k) => {
    const next = out[k + 1] ? out[k + 1].w : '';
    if (x.w === 'Apple Neural Engine,' && next === 'risk') x.w = 'Apple Neural Engine';
    if (x.w === 'AegisFlow,' && next === 'scrubs,') x.w = 'AegisFlow';
  });
  for (const x of out) {
    if (x.w === 'Apple,') x.w = 'Apple';
    if (x.w === 'scrubs,') x.w = 'scrubs';
    if (x.w === 'engine,') x.w = 'Engine,';
    if (x.w === 'neural') x.w = 'Neural';
  }
  const v = out.findIndex((x) => x.w === 'verified');
  if (v >= 0) out[v].w = 'verified.';
  const z = out.findIndex((x, k) => k > v && x.w === 'zero-root');
  if (z >= 0) out[z].w = 'Zero root';
  return out;
}

/** Caption chunks: phrases of at most ~40 characters, broken at punctuation where possible. */
export function captionChunks(words) {
  const chunks = [];
  let cur = [];
  const flush = () => { if (cur.length) chunks.push(cur); cur = []; };
  for (const w of words) {
    if (cur.length && w.s - cur[cur.length - 1].e > 0.55) flush();
    const len = cur.reduce((a, x) => a + x.w.length + 1, 0);
    const ends = /[.,!?]$/.test(w.w);
    if (cur.length && len + w.w.length > (ends ? 52 : 40)) flush();
    cur.push(w);
    const n = cur.reduce((a, x) => a + x.w.length + 1, 0);
    if (/[.!?]$/.test(w.w) || (/,$/.test(w.w) && n > 16)) flush();
  }
  flush();
  return chunks.map((ws, i) => {
    const next = chunks[i + 1];
    const end = Math.min(ws[ws.length - 1].e + 0.5, next ? next[0].s - 0.04 : Infinity);
    return { start: ws[0].s, end, words: ws };
  });
}

const us = (ms) => {
  const v = ms * 1000;
  return `${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} µs`;
};
const row = (a, prefix) => a.targets_summary.find((r) => r.metric.startsWith(prefix)) || a.informational.find((r) => r.metric.startsWith(prefix));

export function buildFilm(artifact, rawWords) {
  const a = artifact;
  const words = fixWords(rawWords);
  const captions = captionChunks(words);
  const voiceEnd = words[words.length - 1].e;

  const warm = row(a, 'branch acquire (WarmBranchPool warm hit) · aegisflow_src');
  const swap = row(a, 'renameatx_np RENAME_SWAP commit');
  const rollback = row(a, 'rollback · caller-visible');
  const aneRow = row(a, 'CoreML FP16 online window model · cpuAndNeuralEngine · back-to-back');
  const plan = row(a, 'online model executes on the Neural Engine');
  const speed = row(a, 'online model, back-to-back calls');
  const cedar = a.cedar.is_authorized_us;
  const pr = a.paypal_receipt;
  const passed = a.targets_summary.filter((r) => r.pass === true).length;
  const total = a.targets_summary.length;
  const host = a.host || {};
  const cores = a.ane.neural_engine_cores;
  const date = String(a.generated_at || '').slice(0, 10);
  const ane = `${plan.measured.neural_engine}/${plan.dispatched_ops}`;

  const duration = Math.ceil((voiceEnd + 4.2) * 10) / 10;
  const LIVE = 'right: 56px; top: 56px;';

  const spec = {
    duration,
    windowTitle: 'Aegis Console',
    windowUrl: '127.0.0.1:4173',
    windowIn: 17.95,
    captions,
    hook: {
      end: 13.85, turn: 6.24, threatsAt: 9.06, shakes: [6.96, 8.34],
      line1: [{ text: 'Autonomous', at: 0.12 }, { text: 'AI', at: 1.02 }, { text: 'agents', at: 1.5 }],
      pills: [{ icon: '</>', text: 'write code', at: 2.46 }, { icon: '$_', text: 'run terminal commands', at: 3.36 }, { icon: '$', text: 'execute payments', at: 4.98 }],
      line2: [{ text: 'One hallucination.', at: 6.72 }, { text: 'One prompt injection.', at: 7.98 }],
      threats: [
        { kicker: 'rm -rf', title: 'Wipe a production repo', code: '$ rm -rf ./production', at: 9.06 },
        { kicker: 'PII leak', title: 'Leak customer PII', code: 'note: jane@example.com · +1 415 555 0134', at: 10.56 },
        { kicker: 'Wallet drain', title: 'Drain a company wallet', code: 'pay $480.00 → new payee, high-risk country', at: 12.3 },
      ],
    },
    brand: {
      t: 13.98, wordAt: 14.4, end: 18.4, word: 'AegisFlow',
      chips: [{ text: 'Zero root', at: 15.66, color: 'emerald' }, { text: 'Speculative copy-on-write sandbox', at: 16.26, color: 'cyan' }],
    },
    finale: {
      t: 89.3, word: 'AegisFlow',
      tagline: 'Governed autonomy for AI agents, built for Apple silicon.',
      metrics: [
        { value: us(warm.measured_p50_ms), label: 'warm CoW branch', color: 'emerald' },
        { value: ane, label: 'ops on the Neural Engine', color: 'cyan' },
        { value: us(cedar.p50 / 1000), label: 'Cedar decision', color: 'violet' },
        { value: `${passed}/${total}`, label: 'targets met', color: 'emerald' },
      ],
      url: 'github.com/whatever0148-cmd/AegisFlow',
      license: 'Open source · MIT license · runs on one MacBook',
    },
    shots: [
      { t: 0, wide: true, s: 0.84, cy: 620, scroll: 0 },
      { t: 19.25, target: { union: [{ sel: '#hw-ane' }, { sel: '#hw-pool' }] }, zoom: 2.0, anchor: 'window', scroll: 0, dur: 1.3 },
      { t: 20.85, target: { union: [{ sel: '#hw-cedar' }, { sel: '#hw-ai' }] }, zoom: 2.0, anchor: 'window', scroll: 0, dur: 1.0 },
      { t: 22.85, target: { sel: '#hwstrip' }, zoom: 1.3, anchor: 'window', scroll: 0, dur: 1.0 },
      { t: 24.75, target: { union: [SC('Safe multi-file refactor'), SC('Unformatted secret')] }, spotTarget: SC('Safe multi-file refactor'), zoom: 1.9, scroll: 0 },
      { t: 26.75, target: NODE(0), zoom: 2.2, anchor: 'window', scroll: 0 },
      { t: 33.2, target: { union: [NODE(1), { sel: '#hf-plan' }] }, zoom: 2.0, anchor: 'window', scroll: 0 },
      { t: 42.5, target: NODE(2), zoom: 2.2, anchor: 'window', scroll: 0 },
      { t: 47.7, target: SC('Build break and recovery'), zoom: 1.8, scroll: 0 },
      { t: 49.75, target: { union: [NODE(2), NODE(4)] }, zoom: 1.65, anchor: 'window', scroll: 0 },
      { t: 54.6, target: { sel: '#panel-workspace .runs .run li.stage', text: 'Verify / recover' }, zoom: 1.8, anchor: 'top', cut: true, reveal: true },
      { t: 58.0, target: PATCH, zoom: 1.7, anchor: 'top', cut: true, reveal: true },
      { t: 63.9, target: SC('Mass delete'), zoom: 1.8, scroll: 0, cut: true },
      { t: 66.4, target: { sel: '#panel-approvals .approvals .approval' }, zoom: 1.75, anchor: 'top' },
      { t: 69.0, target: { sel: '#panel-approvals .approvals .approval .excerpt' }, zoom: 2.1 },
      { t: 70.9, target: { union: [{ sel: '#panel-approvals .approvals .approval .excerpt' }, { sel: '#panel-approvals .approvals .approval .approval__actions' }] }, zoom: 1.9 },
      { t: 72.05, target: { sel: '#panel-approvals .resolved' }, zoom: 2.0, anchor: 'top' },
      { t: 74.3, target: { union: [PP('$42.00'), PP('$480.00')] }, zoom: 1.8, cut: true, spotPad: 18 },
      { t: 77.3, target: { sel: '#panel-paypal .grid-wrap' }, zoom: 1.55, anchor: 'top' },
      { t: 83.1, target: { union: [{ sel: '#panel-paypal [data-role=stream-toggle]' }, { sel: '#panel-paypal .grid-wrap' }] }, zoom: 1.3, anchor: 'top', dur: 0.9 },
      { t: 86.8, target: { union: [{ sel: '#panel-approvals [data-role=bench-meta]' }, { sel: '#panel-approvals .scoreboard tbody tr', index: 3 }] }, zoom: 1.9, anchor: 'top', cut: true },
    ],
    cursor: [
      { t: 25.0, target: SC('Safe multi-file refactor'), dur: 0.9 },
      { t: 26.7, hide: true },
      { t: 48.3, target: SC('Build break and recovery'), dur: 0.8 },
      { t: 49.9, hide: true },
      { t: 58.3, target: PATCH_SUMMARY, dur: 0.5, ax: 0.2, ay: 0.5 },
      { t: 59.6, hide: true },
      { t: 64.6, target: SC('Mass delete'), dur: 0.8, ay: 0.12 },
      { t: 66.3, hide: true },
      { t: 71.0, target: { sel: '#panel-approvals .approvals .approval button', text: 'Reject' }, dur: 0.7, ax: 0.4, ay: 0.5 },
      { t: 72.05, hide: true },
      { t: 74.85, target: PP('$480.00'), dur: 0.55 },
      { t: 75.75, target: PP('$42.00'), dur: 0.45 },
      { t: 76.7, hide: true },
      { t: 79.75, target: INSPECT, dur: 0.6, ax: 0.5, ay: 0.5 },
      { t: 82.35, target: { sel: '#receipt-dialog button', text: 'Close' }, dur: 0.55, ax: 0.5, ay: 0.5 },
      { t: 83.3, target: { sel: '#panel-paypal [data-role=stream-toggle]' }, dur: 0.6, ax: 0.5, ay: 0.5 },
      { t: 84.8, hide: true },
    ],
    highlights: [
      { t: 19.6, dur: 1.5, target: { sel: '#hw-ane' }, color: 'cyan' },
      { t: 21.0, dur: 1.9, target: { sel: '#hw-cedar' }, color: 'violet' },
      { t: 23.0, dur: 1.6, target: { sel: '#hwstrip' }, color: 'emerald', pad: 6 },
      { t: 31.0, dur: 1.9, target: NODE(0), color: 'emerald' },
      { t: 37.4, dur: 2.8, target: { sel: '#hf-plan' }, color: 'cyan', pad: 6 },
      { t: 45.6, dur: 1.9, target: NODE(2), color: 'violet' },
      { t: 51.4, dur: 3.0, target: NODE(4), color: 'cyan' },
      { t: 68.9, dur: 1.9, target: { sel: '#panel-approvals .approvals .approval .excerpt' }, color: 'amber' },
      { t: 78.0, dur: 2.0, target: { sel: '#panel-paypal .ag-row[row-index="0"] .redacted' }, color: 'crimson', pad: 6 },
      { t: 87.5, dur: 1.8, target: { sel: '#panel-approvals [data-role=bench-meta]' }, color: 'emerald', pad: 6 },
    ],
    badges: [
      { t: 19.2, dur: 69.9, text: 'LIVE · real daemon on 127.0.0.1, nothing staged', color: 'emerald', pos: LIVE, blink: true },
      { id: 'lapse', t: 49.7, dur: 4.6, text: 'Live Nemotron call, shortened', color: 'amber', pos: 'right: 56px; top: 116px;' },
      { t: 84.3, dur: 2.6, text: 'Order stream sped up for the video', color: 'amber', pos: 'right: 56px; top: 116px;' },
    ],
    lowers: [
      { t: 19.6, dur: 5.0, icon: '', color: 'cyan', kicker: 'One local runtime',
        title: 'Neural Engine risk model · Cedar policy · APFS sandbox', sub: `${host.cpu || 'Apple silicon'} · ${cores}-core Neural Engine · Rust, Swift and CoreML` },
      { t: 28.0, dur: 4.9, icon: '⚡', color: 'emerald', kicker: 'Warm APFS copy-on-write branch', title: 'Pre-warmed clonefile pool, zero root',
        value: us(warm.measured_p50_ms), sub: `benchmark p50 · n=${warm.n} · AegisFlow source tree` },
      { t: 34.0, dur: 7.9, icon: '🧠', color: 'cyan', kicker: 'Apple Neural Engine', title: `${ane} ops on the ANE (MLComputePlan)`,
        value: us(aneRow.measured_p50_ms), sub: `CoreML FP16 window model · benchmark p50 · ${speed.measured.speedup.toFixed(2)}× faster than cpuOnly` },
      { t: 43.0, dur: 4.6, icon: '🛡️', color: 'violet', kicker: 'AWS Cedar policy engine', title: 'Deterministic allow or deny',
        value: us(cedar.p50 / 1000), sub: `is_authorized p50 · n=${cedar.n} · ${a.cedar.policies} policies, strict schema` },
      { id: 'heal', t: 54.6, dur: 3.6, icon: '🔧', color: 'cyan', kicker: 'Tiered recovery', title: 'Tavily search + NVIDIA Nemotron',
        value: '', sub: 'patch re-checked by rustc and Cedar before commit' },
      { t: 60.0, dur: 3.8, icon: '⚛️', color: 'emerald', kicker: 'Atomic commit', title: 'renameatx_np RENAME_SWAP',
        value: us(swap.measured_p50_ms), sub: `benchmark p50 · n=${swap.n} · the live tree swaps in one syscall` },
      { id: 'hold', t: 67.0, dur: 3.8, icon: '⏸', color: 'amber', kicker: 'Cedar holds the commit', title: 'Team approval required',
        value: '', sub: 'high-impact change, the branch waits for a human' },
      { t: 72.4, dur: 1.95, icon: '↩️', color: 'crimson', kicker: 'Reject · rollback', title: 'Branch detached, live tree untouched',
        value: us(rollback.measured_p50_ms), sub: `benchmark p50 · n=${rollback.n} · reaped in the background` },
      { t: 77.6, dur: 2.6, icon: '🔒', color: 'crimson', kicker: 'Inline redaction', title: 'E-mail and phone become [REDACTED] before signing',
        value: us(pr.redact_us.p50 / 1000), sub: `redaction p50 · n=${pr.redact_us.n}` },
      { t: 80.9, dur: 2.6, icon: '✍️', color: 'cyan', pos: 'left: auto; right: 56px;', kicker: 'Signed receipt', title: 'c2pa:ed25519 on every order',
        value: us(pr.ed25519_sign_us.p50 / 1000), sub: `Ed25519 sign p50 · verify ${us(pr.ed25519_verify_us.p50 / 1000)}` },
      { t: 84.2, dur: 2.4, icon: '📊', color: 'emerald', kicker: 'Live audit stream', title: 'Every order lands in AG Grid',
        sub: 'decision · risk · redactions · receipt, over a local WebSocket' },
      { t: 87.2, dur: 2.1, icon: '✅', color: 'emerald', kicker: 'Verified benchmark run', title: `${passed} of ${total} gated targets met`,
        value: `${passed}/${total}`, sub: `${host.cpu || ''} · ${date} · zero root` },
    ],
  };

  // Clicks on the live console, and what the renderer waits for after each one.
  const actions = [
    { t: 26.1, click: SC('Safe multi-file refactor'), wait: 'run' },
    { t: 49.15, click: SC('Build break and recovery'), wait: 'run', patch: 'heal' },
    { t: 58.9, click: PATCH_SUMMARY },
    { t: 65.6, click: SC('Mass delete'), wait: 'run', patch: 'hold' },
    { t: 71.75, click: { sel: '#panel-approvals .approvals .approval button', text: 'Reject' }, wait: 'resolved' },
    { t: 75.4, click: PP('$480.00'), wait: 'order' },
    { t: 76.2, click: PP('$42.00'), wait: 'order' },
    { t: 80.4, click: INSPECT, wait: 'receipt' },
    { t: 82.95, click: { sel: '#receipt-dialog button', text: 'Close' } },
    { t: 83.95, click: { sel: '#panel-paypal [data-role=stream-toggle]' }, wait: 'stream', pace: '#panel-paypal [data-role=stream-interval]' },
    { t: 87.0, quiet: { sel: '#panel-paypal [data-role=stream-toggle]' } },
  ];
  return { spec, actions, words };
}
