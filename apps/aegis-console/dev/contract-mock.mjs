#!/usr/bin/env node
// Aegis Console contract mock (development only; aegis-daemon never serves dev/).
//
//   node apps/aegis-console/dev/contract-mock.mjs [--port 4174] [--no-offsets]
//
// Serves the console on 127.0.0.1:<port>, substitutes a random per-process token into index.html,
// and implements every endpoint and WebSocket message in ../API.md. Every value it produces is
// SYNTHETIC: numbers come from a seeded PRNG, names say "mock", and GET /api/health reports
// mock: true, which makes the console show a permanent "CONTRACT MOCK" banner. Nothing here is
// telemetry, and the mock never calls an external service (Nebius, Tavily, PayPal, chat webhooks).
//
// The WebSocket handshake and framing (RFC 6455) are implemented by hand with node:http and node:crypto.
// Dev-only extra routes (not in the contract, token-protected): POST /__mock/rotate-token issues a new token
// and drops all sockets, which simulates a daemon restart so the console's 401 / "socket refused" states can be
// exercised; POST /__mock/emit {type, ...fields} broadcasts one message so rare shapes can be rendered;
// POST /__mock/lag {dropped} sends every socket the broadcast-lag notice (log with `dropped`), as aegis-daemon
// does for a client that fell behind. Run messages carry the per-run `seq` and are kept for GET /api/runs
// (newest 10, pending approvals kept), and GET /api/paypal/orders keeps the newest 20 orders, as in API.md.
// --no-offsets omits the optional TimelineEvent.startOffsetUs field.

import http from 'node:http';
import crypto from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const argv = process.argv.slice(2);
const argValue = (name) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
};
const PORT = Number(argValue('--port') || 4174);
const HOST = '127.0.0.1';
const WITH_OFFSETS = !argv.includes('--no-offsets');
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const STARTED_AT = new Date().toISOString();
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const ALLOWED_HOSTS = new Set([`localhost:${PORT}`, `127.0.0.1:${PORT}`, `[::1]:${PORT}`]);
const ALLOWED_ORIGINS = new Set([`http://localhost:${PORT}`, `http://127.0.0.1:${PORT}`, `http://[::1]:${PORT}`]);
const MAX_BODY = 256 * 1024;
let TOKEN = crypto.randomBytes(24).toString('hex');

// ---------------------------------------------------------------------------------------------
// Synthetic value source: a seeded PRNG (mulberry32). Never mistaken for a measurement.
function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rand = mulberry32(0x5eed);
const synth = (lo, hi) => Math.round((lo + rand() * (hi - lo)) * 1000) / 1000;
const synthInt = (lo, hi) => Math.floor(lo + rand() * (hi - lo + 1));
const pick = (list) => list[Math.floor(rand() * list.length)];
const iso = (ms = Date.now()) => new Date(ms).toISOString();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pad = (n, w = 6) => String(n).padStart(w, '0');

// ---------------------------------------------------------------------------------------------
// LiveStats over synthetic samples (rolling last 512, nearest-rank percentiles).
const SAMPLE_KEYS = [
  'branch_acquire_ms', 'branch_cold_ms', 'edit_ms', 'diff_ms', 'ane_score_ms', 'cedar_eval_us',
  'commit_swap_ms', 'rollback_detach_ms', 'reap_ms', 'paypal_total_us', 'paypal_verify_us',
];
const samples = Object.fromEntries(SAMPLE_KEYS.map((k) => [k, []]));
const counters = { runs: 0, commits: 0, rollbacks: 0, approvals_pending: 0, orders: 0, orders_blocked: 0, orders_sent: 0, pool_hits: 0, pool_misses: 0 };

function record(key, value) {
  const list = samples[key];
  list.push(value);
  if (list.length > 512) list.shift();
  return value;
}

function stat(list) {
  if (!list.length) return { n: 0, p50: null, p95: null, last: null };
  const sorted = [...list].sort((a, b) => a - b);
  const rank = (p) => sorted[Math.max(0, Math.ceil(p * sorted.length) - 1)];
  return { n: list.length, p50: rank(0.5), p95: rank(0.95), last: list[list.length - 1] };
}

function liveStats() {
  counters.approvals_pending = [...approvals.values()].filter((a) => a.status === 'pending').length;
  return { samples: Object.fromEntries(SAMPLE_KEYS.map((k) => [k, stat(samples[k])])), counters: { ...counters } };
}

// ---------------------------------------------------------------------------------------------
// Health and the synthetic bench artifact.
const PLAN = (() => {
  const total = synthInt(5, 9);
  const ane = synthInt(0, total);
  return { neural_engine: ane, cpu: total - ane, gpu: 0 };
})();

// health.ane_keepwarm: same shape as aegis-daemon's keep-warm report (synthetic ping counts and
// durations; the mock runs no model). A ping is counted once per keep-warm interval per model.
const KEEPWARM_INTERVAL_MS = 1000;
const keepwarm = { workspace: { pings: 0, failures: 0, last_ping_ms: null }, paypal: { pings: 0, failures: 0, last_ping_ms: null } };

function keepwarmTick() {
  for (const model of Object.values(keepwarm)) {
    model.pings += 1;
    model.last_ping_ms = synth(0.02, 0.2);
  }
}

function health() {
  const models = Object.fromEntries(Object.entries(keepwarm).map(([name, m]) => [name, { ...m }]));
  return {
    ok: true,
    mock: true,
    version: '0.0.0-contract-mock',
    git_commit: 'mock000',
    started_at: STARTED_AT,
    workspace: '/synthetic/mock-workspace',
    compute_units: 'syntheticUnits',
    model: {
      name: 'SyntheticMockModel (not a real model)', input_shape: [1, 64], compute_plan: { ...PLAN },
      path: '/synthetic/mock-model.mlmodelc', layout: 'synthetic layout', operations_dispatched: PLAN.neural_engine + PLAN.cpu + PLAN.gpu,
    },
    ane_keepwarm: {
      enabled: true,
      interval_ms: KEEPWARM_INTERVAL_MS,
      pings: Object.values(keepwarm).reduce((sum, m) => sum + m.pings, 0),
      models,
    },
  };
}

const BENCH = (() => {
  const bound = 1; // synthetic bound, used only to label rows as passing or failing
  const gated = ['alpha', 'beta', 'gamma', 'delta', 'epsilon'].map((name, i) => {
    const value = i % 2 === 0 ? synth(0.05, 0.9) : synth(1.1, 4);
    return { metric: `mock metric ${name}`, target: `< ${bound} ms (synthetic)`, measured_p50_ms: value, p95_ms: value + synth(0, 0.5), n: synthInt(20, 200), pass: value < bound };
  });
  const acquires = synthInt(20, 200);
  const hits = synthInt(Math.floor(acquires / 2), acquires);
  gated.push({
    metric: 'mock pool acquire (synthetic, hit-rate row)', target: `< ${bound} ms and every settled acquire a warm hit (synthetic)`,
    measured_p50_ms: synth(0.02, 0.3), measured_hit_rate: Math.round((hits / acquires) * 1000) / 1000,
    p95_ms: synth(0.3, 0.9), n: acquires, hits, misses: acquires - hits, pass: hits === acquires,
  });
  gated.push({
    metric: 'mock placement (MLComputePlan-shaped, synthetic)',
    target: 'all ops on ANE',
    measured: { ...PLAN },
    pass: PLAN.cpu === 0 && PLAN.gpu === 0,
  });
  return {
    schema: 'aegisflow.contract-mock.telemetry',
    generated_at: STARTED_AT,
    git: { commit: 'mock000', dirty: false },
    host: { cpu: 'synthetic CPU', model: 'mock host', macos: '0.0' },
    ane: { placement_note: 'Synthetic placement note from the contract mock.' },
    targets_summary: gated,
    informational: [
      { metric: 'mock informational metric (with pass)', target: `< ${bound} ms (synthetic)`, measured_p50_ms: synth(0.2, 2), pass: rand() < 0.5 },
      { metric: 'mock informational metric (no pass field)', target: 'context only', measured_p50_us: synth(1, 80) },
    ],
    targets_note: 'Contract-mock note: these synthetic rows exist to exercise PASS, FAIL and not-gated rendering.',
    // Same shape as aegis-bench's run_conditions; the mock artifact is always preliminary.
    preliminary: true,
    run_conditions: {
      preliminary: true,
      reasons: ['contract mock: synthetic values, nothing was measured'],
      rule: 'synthetic: the contract mock labels every artifact preliminary',
    },
  };
})();

// ---------------------------------------------------------------------------------------------
// Configuration: names mirror crates/aegis-config KEYS; the mock stores only presence, never values.
const KEYS = [
  { name: 'NEBIUS_API_KEY', secret: true, hint: 'API key for the selected Nemotron provider (mock)', signup_url: 'https://tokenfactory.nebius.com/' },
  { name: 'NEBIUS_BASE_URL', secret: false, hint: 'Nemotron provider: one of the allowlisted OpenAI-compatible base URLs (mock)', signup_url: null },
  { name: 'NEBIUS_TIER1_MODEL', secret: false, hint: 'Tier-1 (Nemotron Super) model id; required where the provider has no default (mock)', signup_url: null },
  { name: 'NEBIUS_TIER2_MODEL', secret: false, hint: 'Tier-2 (Nemotron Ultra) model id; required where the provider has no default (mock)', signup_url: null },
  { name: 'TAVILY_API_KEY', secret: true, hint: 'Tavily search API key (mock)', signup_url: 'https://app.tavily.com/' },
  { name: 'PAYPAL_CLIENT_ID', secret: true, hint: 'PayPal Sandbox REST app client id (mock)', signup_url: 'https://developer.paypal.com/dashboard/applications/sandbox' },
  { name: 'PAYPAL_CLIENT_SECRET', secret: true, hint: 'PayPal Sandbox REST app secret (mock)', signup_url: 'https://developer.paypal.com/dashboard/applications/sandbox' },
  { name: 'AEGIS_MATTERMOST_WEBHOOK_URL', secret: true, hint: 'Mattermost incoming-webhook URL (mock)', signup_url: null },
  { name: 'AEGIS_SLACK_WEBHOOK_URL', secret: true, hint: 'Slack incoming-webhook URL (mock)', signup_url: null },
  { name: 'AEGIS_APPROVAL_CALLBACK_URL', secret: false, hint: 'Mattermost interactive-action callback URL (mock)', signup_url: null },
];
const configured = new Map([['NEBIUS_TIER1_MODEL', 'env']]); // name -> source

// Mirrors aegis_config::NEMOTRON_PROVIDERS, the allowlist NEBIUS_BASE_URL is checked against. These
// are contract values (where a key may be sent), not telemetry; the mock never contacts them.
const NEMOTRON_PROVIDERS = [
  { id: 'nebius', label: 'Nebius Token Factory', base_url: 'https://api.tokenfactory.nebius.com/v1', signup_url: 'https://tokenfactory.nebius.com/', key_hint: 'Nebius Token Factory API key', tier1_model: 'nvidia/Llama-3_3-Nemotron-Super-49B-v1', tier2_model: 'nvidia/Llama-3_1-Nemotron-Ultra-253B-v1' },
  { id: 'nvidia-nim', label: 'NVIDIA NIM', base_url: 'https://integrate.api.nvidia.com/v1', signup_url: 'https://build.nvidia.com', key_hint: 'build.nvidia.com API key (nvapi-…)', tier1_model: null, tier2_model: 'nvidia/llama-3.1-nemotron-ultra-253b-v1' },
  { id: 'openrouter', label: 'OpenRouter', base_url: 'https://openrouter.ai/api/v1', signup_url: 'https://openrouter.ai/keys', key_hint: 'OpenRouter API key', tier1_model: null, tier2_model: null },
];
const providerFor = (url) => NEMOTRON_PROVIDERS.find((p) => p.base_url === String(url).trim().replace(/\/$/, '')) || null;
// NEBIUS_BASE_URL is public, so the mock keeps which provider is selected (never a secret value).
let nemotronProvider = NEMOTRON_PROVIDERS[0];

/** Same rule as aegis-daemon's services::console_writable: every key; NEBIUS_BASE_URL only to the allowlist. */
const consoleWritable = (name) => KEYS.some((k) => k.name === name);

/** Same checks as aegis-daemon's services::console_update_problem; returns the 400 message or null. */
function consoleUpdateProblem(values) {
  if (!Object.hasOwn(values, 'NEBIUS_BASE_URL')) return null;
  const value = values.NEBIUS_BASE_URL;
  const next = value === '' ? NEMOTRON_PROVIDERS[0] : providerFor(value);
  if (!next) {
    return `NEBIUS_BASE_URL from the console must be one of the Nemotron providers: ${NEMOTRON_PROVIDERS.map((p) => `${p.label} (${p.base_url})`).join(', ')} (mock)`;
  }
  if (next.id === nemotronProvider.id || !configured.has('NEBIUS_API_KEY')) return null;
  if (configured.get('NEBIUS_API_KEY') === 'env') return `NEBIUS_API_KEY comes from the environment, so the console cannot switch the Nemotron provider to ${next.label} (mock)`;
  if (!Object.hasOwn(values, 'NEBIUS_API_KEY')) {
    return `switching the Nemotron provider to ${next.label} needs that provider's API key in NEBIUS_API_KEY in the same save (or clear NEBIUS_API_KEY) (mock)`;
  }
  return null;
}

function serviceState(names, label) {
  const missing = names.filter((n) => !configured.has(n));
  return missing.length
    ? { enabled: false, reason: `${missing.join(' / ')} not set` }
    : { enabled: true, reason: `${label}: marked set in the mock; the mock never calls out` };
}

function nebiusService() {
  const p = nemotronProvider;
  const tier1 = configured.has('NEBIUS_TIER1_MODEL') ? 'mock-tier1-model' : p.tier1_model;
  const tier2 = configured.has('NEBIUS_TIER2_MODEL') ? 'mock-tier2-model' : p.tier2_model;
  const missing = [['NEBIUS_TIER1_MODEL', tier1], ['NEBIUS_TIER2_MODEL', tier2]].filter(([, m]) => !m).map(([n]) => n);
  let state = serviceState(['NEBIUS_API_KEY'], 'nebius');
  if (state.enabled && missing.length) state = { enabled: false, reason: `${missing.join(' / ')} not set (${p.label} has no confirmed default model id for it)` };
  return { ...state, provider: p.label, base_url: p.base_url, tier1_model: tier1, tier2_model: tier2 };
}

/** Same fields as aegis-daemon's services.paypal / scenarios `sandbox` (API.md): both PayPal keys
 * unset → "local" (the daemon's local Orders v2 gateway; the mock runs none and sends nothing), both
 * marked set → "remote", one of them → "disabled". */
function paypalService() {
  const set = ['PAYPAL_CLIENT_ID', 'PAYPAL_CLIENT_SECRET'].filter((n) => configured.has(n));
  const mode = set.length === 0 ? 'local' : set.length === 2 ? 'remote' : 'disabled';
  const state = mode === 'local'
    ? { enabled: true, reason: 'PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set: local Orders v2 gateway (mock: none runs; nothing is sent)' }
    : serviceState(['PAYPAL_CLIENT_ID', 'PAYPAL_CLIENT_SECRET'], 'paypal');
  return {
    ...state,
    mode,
    base_url: mode === 'local' ? `http://127.0.0.1:${PORT}/paypal-sandbox` : 'https://paypal.synthetic.invalid',
    submitted: sandbox.submitted,
    cap: sandbox.cap,
    local_submitted: 0,
    gateway: null,
  };
}

function services() {
  return {
    nebius: nebiusService(),
    tavily: serviceState(['TAVILY_API_KEY'], 'tavily'),
    paypal: paypalService(),
    mattermost: serviceState(['AEGIS_MATTERMOST_WEBHOOK_URL'], 'mattermost'),
    slack: serviceState(['AEGIS_SLACK_WEBHOOK_URL'], 'slack'),
  };
}

function configState() {
  return {
    path: '/synthetic/mock-workspace/.env (mock: nothing is written)',
    keys: KEYS.map((k) => ({
      name: k.name, set: configured.has(k.name), source: configured.get(k.name) || null, secret: k.secret,
      console_writable: consoleWritable(k.name), hint: k.hint,
      signup_url: k.name === 'NEBIUS_API_KEY' ? nemotronProvider.signup_url : k.signup_url,
      ...(k.name === 'NEBIUS_BASE_URL' ? { choices: NEMOTRON_PROVIDERS.map((p) => ({ value: p.base_url, label: p.label })) } : {}),
    })),
    nemotron_providers: NEMOTRON_PROVIDERS,
    services: services(),
  };
}

// ---------------------------------------------------------------------------------------------
// Workspace, runs and approvals.
const POOL_CAPACITY = 3;
const pool = {
  ready: POOL_CAPACITY, capacity: POOL_CAPACITY, hits: 0, misses: 0, stale_discards: 0, last_replenish_ms: null,
  // Optional FSEvents diagnostics fields (API.md); the mock watches nothing.
  replenishing: false, replenished: 0, fs_events: 0, fs_events_ignored: 0, watching_fs_events: false, recent_fs_changes: [], last_error: null,
};
const reaper = { queued: 0, reaped: 0, last_reap_ms: null, failed: 0, background: true };
const versions = [{ number: 1, created_at: STARTED_AT }];
const WORKSPACE_FILES = synthInt(40, 140);
const WS_SCENARIOS = [
  { id: 'safe_refactor', title: 'Safe multi-file refactor', description: 'Mock: renames a helper across several files.', expected: 'low risk → commit' },
  { id: 'secret_leak', title: 'Unformatted secret', description: 'Mock: writes a credential-shaped string into a config file.', expected: 'policy deny → rollback' },
  { id: 'mass_delete', title: 'Mass delete', description: 'Mock: deletes many files at once. (aegis-daemon\'s description discloses its tuning: fixture count and per-tool-call delete events.)', expected: 'high impact (window of per-tool-call events, see description) → team approval' },
  { id: 'build_break', title: 'Build break', description: 'Mock: introduces a compile error.', expected: 'verify fails → recovery tiers → patched commit or rollback' },
];

function workspaceState() {
  return { live: '/synthetic/mock-workspace/live', files: WORKSPACE_FILES, pool: { ...pool }, reaper: { ...reaper }, versions: versions.map((v) => ({ ...v })), scenarios: WS_SCENARIOS };
}

let runSeq = 0;
let approvalSeq = 0;

// Run history (API.md "Run history"): run messages exactly as broadcast, newest RUNS_KEPT runs.
const RUNS_KEPT = 10;
const RUN_TYPES = new Set(['run.started', 'run.stage', 'run.finished']);
const runHistory = []; // oldest run id first
const runNumber = (id) => {
  const m = /(\d+)$/.exec(String(id));
  return m ? Number(m[1]) : 0;
};

/** Adds `seq` to a run message and records a copy, like aegis-daemon's RunHistory::observe. */
function recordRunMessage(msg) {
  if (msg.type === 'run.started') {
    msg.seq = 1;
    const rec = {
      run_id: msg.run_id, scenario: msg.scenario ?? null, title: msg.title ?? null, started_at: msg.ts,
      outcome: null, total_ms: null, queued_ms: null, stages: [], finished: [], last_seq: 1,
    };
    const at = runHistory.findIndex((r) => runNumber(r.run_id) > runNumber(msg.run_id));
    runHistory.splice(at < 0 ? runHistory.length : at, 0, rec);
    while (runHistory.length > RUNS_KEPT) {
      const idx = runHistory.findIndex((r) => r.outcome !== 'pending_approval');
      runHistory.splice(idx < 0 ? 0 : idx, 1);
    }
    return;
  }
  const rec = runHistory.find((r) => r.run_id === msg.run_id);
  if (!rec) return; // no longer kept: broadcast without seq
  rec.last_seq += 1;
  msg.seq = rec.last_seq;
  if (msg.type === 'run.stage') {
    rec.stages.push({ ...msg });
  } else {
    rec.outcome = msg.outcome;
    rec.total_ms = msg.total_ms;
    if (msg.queued_ms !== null && msg.queued_ms !== undefined) rec.queued_ms = msg.queued_ms;
    rec.finished.push({ ...msg });
  }
}
const approvals = new Map();
const runWaiters = new Map(); // run_id -> resolve(approval)

function cardsStatus() {
  const card = (name) => (configured.has(name) ? 'error: the contract mock does not post webhooks' : 'disabled');
  return { mattermost: card('AEGIS_MATTERMOST_WEBHOOK_URL'), slack: card('AEGIS_SLACK_WEBHOOK_URL') };
}

function createApproval(fields) {
  const a = {
    id: `a-${pad(++approvalSeq)}`, run_id: null, kind: 'workspace', title: '', summary: '', risk: null, files_changed: 0,
    excerpt: '', determining_policies: [], created_at: iso(), status: 'pending', resolved_by: null, resolved_at: null,
    result: null, cards: cardsStatus(), ...fields,
  };
  approvals.set(a.id, a);
  return a;
}

function resolveApproval(a, approve, by) {
  const action = approve ? 'commit' : 'rollback';
  const ms = record(approve ? 'commit_swap_ms' : 'rollback_detach_ms', synth(0.05, 0.4));
  Object.assign(a, { status: approve ? 'approved' : 'rejected', resolved_by: by, resolved_at: iso(), result: { action, ms, detail: { mock: true } } });
  return a;
}

function replenishSoon() {
  setTimeout(() => {
    if (pool.ready < pool.capacity) {
      pool.ready += 1;
      pool.replenished += 1;
      pool.last_replenish_ms = synth(0.5, 6);
      broadcast('log', { level: 'info', message: `mock: warm pool replenished (${pool.ready}/${pool.capacity})` });
    }
  }, 1500);
}

function reapSoon() {
  reaper.queued += 1;
  setTimeout(() => {
    reaper.queued = Math.max(0, reaper.queued - 1);
    reaper.reaped += 1;
    reaper.last_reap_ms = record('reap_ms', synth(1, 12));
  }, 900);
}

// UnifiedDiff (API.md): a synthetic diff text in the daemon's shape, labelled as mock data. The real
// daemon renders the branch against live, redacted and capped at cap_bytes.
const MOCK_DIFF_CAP = 24 * 1024;
function mockDiff(scenario, patch = false) {
  const file = patch || scenario === 'build_break' ? 'src/report.rs' : scenario === 'secret_leak' ? 'config/app.toml' : 'src/mock.rs';
  const body = patch
    ? ['@@ -1,3 +1,3 @@', ' // contract mock: synthetic repaired file', '-    format_cents(order_amount(lines), CURRENCY)', '+    format_cents(order_amount(lines))', ' }']
    : scenario === 'secret_leak'
      ? ['@@ -1,1 +1,3 @@', ' # contract mock', '+[upstream]', '+[REDACTED]"']
      : ['@@ -1,3 +1,3 @@', ' // contract mock: synthetic diff, not a real branch', `-pub fn mock_${synthInt(1, 99)}() {}`, `+pub fn mock_${synthInt(100, 199)}() {}`, ' fn unchanged() {}'];
  const text = [`diff --git a/${file} b/${file}`, `--- a/${file}`, `+++ b/${file}`, ...body, ''].join('\n');
  return { format: 'unified', text, bytes: text.length, bytes_total: text.length, cap_bytes: MOCK_DIFF_CAP, truncated: false, files: 1, files_total: 1, files_omitted: 0, redactions: text.includes('[REDACTED]') ? 1 : 0 };
}

async function runScenario(run_id, scenario, requestedAt) {
  const sc = WS_SCENARIOS.find((s) => s.id === scenario);
  const t0 = Date.now();
  let queuedMs = t0 - requestedAt; // the first run.finished carries it; after an approval it is null
  const stage = async (name, status, ms, detail = {}) => {
    await sleep(260);
    broadcast('run.stage', { run_id, stage: name, status, ms, detail });
  };
  const finish = (outcome) => {
    broadcast('run.finished', { run_id, outcome, total_ms: Date.now() - t0, queued_ms: queuedMs });
    queuedMs = null;
  };
  broadcast('run.started', { run_id, scenario, title: sc.title });

  const fromPool = pool.ready > 0;
  if (fromPool) {
    pool.ready -= 1;
    pool.hits += 1;
    counters.pool_hits += 1;
    replenishSoon();
  } else {
    pool.misses += 1;
    counters.pool_misses += 1;
  }
  const branchMs = record('branch_acquire_ms', fromPool ? synth(0.02, 0.2) : synth(0.8, 3));
  if (!fromPool) record('branch_cold_ms', branchMs);
  await stage('branch', 'ok', branchMs, { source: fromPool ? 'pool' : 'cold' });
  await stage('edit', 'ok', record('edit_ms', synth(0.05, 0.6)), { files: synthInt(1, 12) });
  await stage('diff', 'ok', record('diff_ms', synth(0.5, 4)), { files_changed: scenario === 'mass_delete' ? synthInt(30, 60) : synthInt(1, 8), unified_diff: mockDiff(scenario) });
  await stage('features', 'ok', synth(0.002, 0.02), { count: 64 });
  const riskRange = { safe_refactor: [5, 25], secret_leak: [30, 60], mass_delete: [65, 95], build_break: [10, 30] }[scenario];
  const risk = synth(riskRange[0], riskRange[1]);
  const scoreDetail = { risk, units: 'syntheticUnits' };
  if (scenario === 'mass_delete') {
    // aegis-daemon also scores the same change as one event with no history, for comparison.
    scoreDetail.single_event_risk = synth(5, 40);
    scoreDetail.single_event_score_ms = synth(0.02, 0.09);
  }
  await stage('score', 'ok', record('ane_score_ms', synth(0.02, 0.09)), scoreDetail);
  const policy = {
    safe_refactor: [true, ['mock-permit-low-risk']],
    secret_leak: [false, ['mock-forbid-unformatted-secret']],
    mass_delete: [false, ['mock-forbid-high-impact-without-team-approval']],
    build_break: [true, ['mock-permit-low-risk']],
  }[scenario];
  await stage('policy', policy[0] ? 'ok' : 'fail', synth(0.003, 0.02), { allowed: policy[0], determining_policies: policy[1], eval_us: record('cedar_eval_us', synth(2, 9)) });

  const commit = async () => {
    const version = versions.length + 1;
    versions.push({ number: version, created_at: iso() });
    counters.commits += 1;
    await stage('commit', 'ok', record('commit_swap_ms', synth(0.05, 0.4)), { version });
    reapSoon();
    await stage('reap', 'ok', null, { queued: reaper.queued });
  };
  const rollback = async () => {
    counters.rollbacks += 1;
    await stage('rollback', 'ok', record('rollback_detach_ms', synth(0.03, 0.3)), { detached: true });
    reapSoon();
    await stage('reap', 'ok', null, { queued: reaper.queued });
  };

  if (scenario === 'safe_refactor') {
    await stage('verify', 'ok', synth(5, 40), { checks: synthInt(2, 6) });
    await commit();
    finish('committed');
  } else if (scenario === 'secret_leak') {
    await stage('verify', 'skip', null, { reason: 'policy denied' });
    await rollback();
    finish('rolled_back');
  } else if (scenario === 'mass_delete') {
    await stage('verify', 'ok', synth(5, 40), { checks: synthInt(2, 6) });
    const a = createApproval({
      run_id, kind: 'workspace', title: 'Mass delete in mock workspace', summary: 'Synthetic approval raised by the contract mock.',
      risk, files_changed: synthInt(30, 60), excerpt: '- mock/file_001.rs\n- mock/file_002.rs\n- api_key = [REDACTED]\n…',
      determining_policies: policy[1],
    });
    broadcast('approval.created', { approval: a });
    await stage('approval', 'pending', null, { approval_id: a.id });
    finish('pending_approval');
    const resolved = await new Promise((resolve) => runWaiters.set(run_id, resolve));
    if (resolved.status === 'approved') {
      await commit();
      finish('committed');
    } else {
      await rollback();
      finish('rolled_back');
    }
  } else if (scenario === 'build_break') {
    await stage('verify', 'fail', synth(5, 40), { errors: synthInt(1, 3) });
    const patched = rand() < 0.5;
    await stage('recover', patched ? 'ok' : 'fail', synth(0.1, 2), {
      tier: 0, model: null, enabled: true, reason: 'local deterministic patch (mock)', patch_applied: patched, committed_to_branch: patched,
      ...(patched ? { patch_diff: mockDiff(scenario, true) } : {}),
    });
    if (patched) {
      await stage('verify', 'ok', synth(5, 40), { checks: synthInt(2, 6) });
      await commit();
      finish('recovered');
    } else {
      const nebius = services().nebius;
      await stage('recover', 'skip', null, {
        tier: 1, model: nebius.tier1_model, enabled: nebius.enabled,
        reason: nebius.enabled ? 'contract mock: no external call is made' : nebius.reason, patch_applied: false,
      });
      await rollback();
      finish('rolled_back');
    }
  }
}

// Seed one pending and one resolved approval so panel 3 has content on first load.
createApproval({
  kind: 'paypal', title: 'Mock order to a new payee', summary: 'Synthetic PayPal approval seeded at mock start.',
  risk: synth(60, 90), files_changed: 0, excerpt: '{"payee":"[REDACTED]","amount":"mock"}', determining_policies: ['mock-forbid-new-payee-high-amount'],
});
resolveApproval(createApproval({
  kind: 'workspace', title: 'Mock earlier approval', summary: 'Synthetic, resolved at mock start.', risk: synth(60, 90),
  files_changed: synthInt(10, 40), excerpt: 'mock excerpt', determining_policies: ['mock-forbid-high-impact-without-team-approval'],
}), false, 'mock-seed');

// ---------------------------------------------------------------------------------------------
// PayPal orders, timeline and stream.
const COLUMN_DEFS = [
  ['timestamp', 'Time (UTC)', false], ['reference', 'Order', false], ['merchant', 'Merchant', false], ['amount', 'Amount', true],
  ['currency', 'Ccy', false], ['risk_score', 'ANE risk', true], ['decision', 'Cedar', false], ['policies', 'Determining policies', false],
  ['receipt_verified', 'Receipt verified', false], ['redactions', 'Redactions', true], ['submission', 'Submission', false], ['total_us', 'Pipeline µs', true],
  ['submit_us', 'Submit µs', true],
].map(([field, headerName, numeric]) => ({ field, headerName, sortable: true, filter: true, ...(numeric ? { type: 'numericColumn' } : {}) }));
const RESOURCES = ['redact', 'score', 'policy', 'sign', 'verify', 'submit'].map((id) => ({ id, name: id }));
const PP_SCENARIOS = [
  { id: 'known_payee_small', title: 'Small order, known payee', description: 'Mock: low amount to a known merchant.' },
  { id: 'new_payee_large', title: 'Large order, new payee', description: 'Mock: high amount to a new payee.' },
  { id: 'secret_in_note', title: 'Secret in customer note', description: 'Mock: the note carries a credential-shaped string.' },
  { id: 'over_budget', title: 'Over budget', description: 'Mock: exceeds the agent budget.' },
];
const STREAM_MIN_MS = 1000;
const STREAM_MAX_MS = 600000;
const stream = { enabled: false, interval_ms: 2000 };
const SANDBOX_CAP = 25;
const sandbox = { submitted: 0, cap: SANDBOX_CAP };
const ORDERS_KEPT = 20; // newest orders kept, with their timeline events (API.md)
const orders = [];
const timelineEvents = [];
let orderSeq = 0;
let streamTimer = null;

// GET /api/paypal/orders/{id}/receipt documents (synthetic), kept with their rows.
const receipts = new Map();

function makeOrder(scenario) {
  const n = ++orderSeq;
  const reference = `mock-${pad(n, 4)}`;
  const sha = crypto.createHash('sha256').update(`synthetic-receipt-${n}-${rand()}`).digest('hex');
  const amountRange = { known_payee_small: [500, 5000], new_payee_large: [15000, 90000], secret_in_note: [1000, 9000], over_budget: [25000, 60000] }[scenario];
  const cents = synthInt(amountRange[0], amountRange[1]);
  const riskRange = { known_payee_small: [5, 25], new_payee_large: [55, 90], secret_in_note: [30, 60], over_budget: [40, 80] }[scenario];
  const allowed = scenario === 'known_payee_small';
  const policies = {
    known_payee_small: 'mock-permit-low-risk',
    new_payee_large: 'mock-forbid-new-payee-high-amount',
    secret_in_note: 'mock-forbid-unformatted-secret',
    over_budget: 'mock-forbid-over-budget',
  }[scenario];
  const timing = { redact_us: synth(5, 20), score_us: synth(25, 60), cedar_us: synth(5, 25), sign_us: synth(15, 30), verify_us: synth(30, 60) };
  const submitUs = allowed ? synth(1, 4) : 0;
  const total = Math.round((timing.redact_us + timing.score_us + timing.cedar_us + timing.sign_us + timing.verify_us + submitUs + synth(2, 10)) * 100) / 100;
  record('paypal_total_us', total);
  record('paypal_verify_us', timing.verify_us);
  counters.orders += 1;
  if (!allowed) counters.orders_blocked += 1;
  const pp = paypalService();
  const sandboxOn = pp.enabled;
  const row = {
    id: `${reference}-${sha.slice(0, 12)}`,
    timestamp: iso().replace(/\.\d{3}Z$/, 'Z'),
    reference,
    merchant: scenario === 'new_payee_large' ? 'mock-new-payee' : 'mock-lab-supplies',
    agent: 'mock-procurement-agent',
    amount_cents: cents,
    amount: (cents / 100).toFixed(2),
    currency: 'USD',
    risk_score: synth(riskRange[0], riskRange[1]),
    decision: allowed ? 'Allow' : 'Deny',
    policies,
    receipt_sha256: sha,
    signature: `c2pa:ed25519:${crypto.randomBytes(64).toString('base64url')}`,
    receipt_verified: rand() >= 0.1,
    redactions: scenario === 'secret_in_note' ? synthInt(2, 4) : synthInt(0, 2),
    unformatted_sensitive_fields: scenario === 'secret_in_note',
    submission: allowed ? 'not_sent' : 'blocked',
    submission_detail: allowed
      ? (sandboxOn ? 'contract mock: nothing is sent to PayPal' : pp.reason)
      : `denied by ${policies}`,
    paypal_order_id: null,
    submission_target: null,
    paypal_capture_id: null,
    submit_us: null,
    submit_calls: [],
    ...timing,
    total_us: total,
  };
  // Sequential spans, microsecond offsets from the order start.
  const t0 = Date.now();
  const spans = [['redact', timing.redact_us], ['score', timing.score_us], ['policy', timing.cedar_us], ['sign', timing.sign_us], ['verify', timing.verify_us]];
  if (allowed) spans.push(['submit', submitUs]);
  let offset = 0;
  const events = spans.map(([resourceId, dur], i) => {
    const cls = !allowed && i >= 2 ? 'blocked' : 'ok';
    const ev = {
      id: `${row.id}:${resourceId}`, resourceId, name: `${reference} ${resourceId}`,
      startDate: iso(t0 + offset / 1000), endDate: iso(t0 + (offset + dur) / 1000), durationUs: dur, orderId: row.id, cls,
    };
    if (WITH_OFFSETS) ev.startOffsetUs = Math.round(offset * 1000) / 1000;
    offset += dur;
    return ev;
  });
  orders.push(row);
  receipts.set(row.id, {
    order_id: row.id, reference,
    receipt: { scheme: 'c2pa:ed25519', signature: row.signature, payload_sha256: sha, public_key: crypto.randomBytes(32).toString('hex'), verified: row.receipt_verified, attached_at: 'purchase_units[0].custom_id' },
    redactions: row.redactions, display_redactions: 0, unformatted_sensitive_fields: row.unformatted_sensitive_fields,
    body: { intent: 'CAPTURE', purchase_units: [{ reference_id: reference, description: 'contract mock order | note [REDACTED]',
      amount: { currency_code: 'USD', value: row.amount }, custom_id: row.signature }] },
  });
  timelineEvents.push(...events);
  while (orders.length > ORDERS_KEPT) {
    const dropped = orders.shift();
    receipts.delete(dropped.id);
    for (let i = timelineEvents.length - 1; i >= 0; i -= 1) if (timelineEvents[i].orderId === dropped.id) timelineEvents.splice(i, 1);
  }
  if (scenario === 'new_payee_large' && rand() < 0.5) {
    const a = createApproval({
      kind: 'paypal', title: `Mock order ${reference} to a new payee`, summary: 'Synthetic approval raised by a mock order.',
      risk: row.risk_score, files_changed: 0, excerpt: `{"reference":"${reference}","payee":"[REDACTED]"}`, determining_policies: [policies],
    });
    setTimeout(() => broadcast('approval.created', { approval: a }), 50);
  }
  return { row, events };
}

for (const sc of ['known_payee_small', 'over_budget', 'secret_in_note']) makeOrder(sc);

function setStream(enabled, intervalMs) {
  stream.enabled = enabled;
  stream.interval_ms = intervalMs;
  clearInterval(streamTimer);
  streamTimer = null;
  if (enabled) {
    streamTimer = setInterval(() => {
      const { row, events } = makeOrder(pick(PP_SCENARIOS).id);
      broadcast('paypal.order', { row, events });
    }, intervalMs);
  }
}

// ---------------------------------------------------------------------------------------------
// HTTP plumbing.
const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.json': 'application/json', '.txt': 'text/plain; charset=utf-8',
  '.md': 'text/markdown; charset=utf-8', '.png': 'image/png', '.woff2': 'font/woff2',
};
const CSP = [
  "default-src 'self'", "script-src 'self'", "style-src 'self' 'unsafe-inline'", "img-src 'self' data:",
  `connect-src 'self' ws://127.0.0.1:${PORT} ws://localhost:${PORT}`, "font-src 'self'", "object-src 'none'",
  "base-uri 'none'", "form-action 'none'", "frame-ancestors 'none'",
].join('; ');

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

function sendJson(res, status, body) {
  const text = JSON.stringify(body);
  res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(text), 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
  res.end(text);
}

function hostAllowed(req) {
  if (!ALLOWED_HOSTS.has(String(req.headers.host || '').toLowerCase())) return false;
  const origin = req.headers.origin;
  return origin === undefined || ALLOWED_ORIGINS.has(origin);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks = [];
    let tooLarge = false;
    req.on('data', (c) => {
      size += c.length;
      if (size > MAX_BODY) tooLarge = true; // keep draining so a 413 response can still be written
      else chunks.push(c);
    });
    req.on('end', () => (tooLarge ? reject(new HttpError(413, 'body too large')) : resolve(Buffer.concat(chunks).toString('utf8'))));
    req.on('error', reject);
  });
}

async function jsonBody(req) {
  if (req.headers['x-aegis-token'] !== TOKEN) throw new HttpError(401, 'missing or wrong X-Aegis-Token');
  if (!String(req.headers['content-type'] || '').toLowerCase().startsWith('application/json')) throw new HttpError(415, 'Content-Type must be application/json');
  const text = await readBody(req);
  try {
    const body = text ? JSON.parse(text) : {};
    if (!body || typeof body !== 'object' || Array.isArray(body)) throw new Error('not an object');
    return body;
  } catch {
    throw new HttpError(400, 'body must be a JSON object');
  }
}

async function serveStatic(res, rel) {
  const base = rel.startsWith('assets/') ? path.join(ROOT, 'assets') : path.join(ROOT, 'vendor');
  const file = path.resolve(ROOT, rel);
  if (!file.startsWith(base + path.sep)) throw new HttpError(404, 'not found');
  let data;
  try {
    data = await readFile(file);
  } catch {
    throw new HttpError(404, 'not found');
  }
  res.writeHead(200, { 'Content-Type': MIME[path.extname(file)] || 'application/octet-stream', 'Content-Length': data.length, 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
  res.end(data);
}

async function serveIndex(res) {
  const html = (await readFile(path.join(ROOT, 'index.html'), 'utf8')).split('%%AEGIS_TOKEN%%').join(TOKEN);
  res.writeHead(200, {
    'Content-Type': 'text/html; charset=utf-8', 'Content-Length': Buffer.byteLength(html), 'Cache-Control': 'no-store',
    'Content-Security-Policy': CSP, 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer',
  });
  res.end(html);
}

async function route(req, res) {
  const url = new URL(req.url, `http://${HOST}:${PORT}`);
  const p = url.pathname;
  const m = req.method;
  if (!hostAllowed(req)) throw new HttpError(403, 'Host/Origin not allowed');

  if (m === 'GET' && (p === '/' || p === '/index.html')) return serveIndex(res);
  if (m === 'GET' && (p.startsWith('/assets/') || p.startsWith('/vendor/'))) return serveStatic(res, decodeURIComponent(p.slice(1)));
  if (m === 'GET' && p === '/favicon.ico') return serveStatic(res, 'assets/favicon.svg');

  if (m === 'GET') {
    switch (p) {
      case '/api/health': return sendJson(res, 200, health());
      case '/api/telemetry': return sendJson(res, 200, { bench: BENCH, bench_path: 'artifacts/contract-mock (synthetic, no file)', bench_generated_at: STARTED_AT, live: liveStats() });
      case '/api/config': return sendJson(res, 200, configState());
      case '/api/workspace': return sendJson(res, 200, workspaceState());
      case '/api/approvals': {
        const all = [...approvals.values()];
        return sendJson(res, 200, {
          pending: all.filter((a) => a.status === 'pending'),
          resolved: all.filter((a) => a.status !== 'pending').sort((a, b) => String(b.resolved_at).localeCompare(String(a.resolved_at))).slice(0, 50),
        });
      }
      case '/api/runs': return sendJson(res, 200, { runs: [...runHistory].reverse(), limit: RUNS_KEPT });
      case '/api/paypal/orders': return sendJson(res, 200, { rows: orders, columnDefs: COLUMN_DEFS, timeline: { resources: RESOURCES, events: timelineEvents }, limit: ORDERS_KEPT });
      case '/api/paypal/scenarios': return sendJson(res, 200, { scenarios: PP_SCENARIOS, stream: { ...stream }, sandbox: paypalService() });
      default: {
        const rm = /^\/api\/paypal\/orders\/([^/]+)\/receipt$/.exec(p);
        if (rm) {
          const doc = receipts.get(decodeURIComponent(rm[1]));
          if (!doc) throw new HttpError(404, `order ${decodeURIComponent(rm[1]).slice(0, 64)} is not among the newest ${ORDERS_KEPT} orders`);
          return sendJson(res, 200, doc);
        }
        throw new HttpError(404, 'not found');
      }
    }
  }

  if (m !== 'POST') throw new HttpError(405, 'method not allowed');
  const body = await jsonBody(req);

  if (p === '/api/config') {
    const values = body.values;
    if (!values || typeof values !== 'object' || Array.isArray(values)) throw new HttpError(400, 'values must be an object');
    for (const [name, value] of Object.entries(values)) {
      if (!KEYS.some((k) => k.name === name)) throw new HttpError(400, `unknown key ${name}`);
      if (typeof value !== 'string' || /[\r\n]/.test(value)) throw new HttpError(400, `invalid value for ${name}`);
      // Only "" unsets; a whitespace-only value is refused, as in aegis-daemon.
      if (value !== '' && value.trim() === '') throw new HttpError(400, `${name}: value is only whitespace; send "" to unset`);
      if (value !== '' && !consoleWritable(name)) throw new HttpError(400, `${name} cannot be set from the console (mock)`);
    }
    const problem = consoleUpdateProblem(values);
    if (problem) throw new HttpError(400, problem);
    for (const [name, value] of Object.entries(values)) {
      if (value === '') configured.delete(name); else configured.set(name, 'dotenv');
      if (name === 'NEBIUS_BASE_URL') nemotronProvider = value === '' ? NEMOTRON_PROVIDERS[0] : providerFor(value);
    }
    const cfg = configState();
    broadcast('config.changed', { config: cfg });
    broadcast('log', { level: 'info', message: `mock: configuration updated (${Object.keys(values).join(', ')})` });
    return sendJson(res, 200, cfg);
  }
  if (p === '/api/config/test') {
    const service = body.service;
    if (!['nebius', 'tavily', 'paypal'].includes(service)) throw new HttpError(400, 'service must be nebius, tavily or paypal');
    await sleep(300);
    const enabled = services()[service].enabled;
    return sendJson(res, 200, { service, ok: false, latency_ms: null, detail: enabled ? 'contract mock: no external call is made' : 'not configured' });
  }
  if (p === '/api/workspace/run') {
    if (!WS_SCENARIOS.some((s) => s.id === body.scenario)) throw new HttpError(400, 'unknown scenario');
    const run_id = `r-${pad(++runSeq)}`;
    counters.runs += 1;
    const requestedAt = Date.now();
    setTimeout(() => runScenario(run_id, body.scenario, requestedAt).catch((err) => broadcast('log', { level: 'error', message: `mock run failed: ${err.message}` })), 0);
    return sendJson(res, 200, { run_id });
  }
  if (p === '/api/workspace/reset') {
    versions.splice(1);
    pool.ready = pool.capacity;
    broadcast('log', { level: 'info', message: 'mock: workspace reset' });
    return sendJson(res, 200, workspaceState());
  }
  const ap = p.match(/^\/api\/approvals\/([^/]+)\/(approve|reject)$/);
  if (ap) {
    const a = approvals.get(decodeURIComponent(ap[1]));
    if (!a) throw new HttpError(404, 'unknown approval');
    if (typeof body.by !== 'string' || !body.by.trim()) throw new HttpError(400, 'by is required');
    if (a.status !== 'pending') throw new HttpError(409, `approval already ${a.status}`);
    await sleep(400);
    resolveApproval(a, ap[2] === 'approve', body.by.trim());
    broadcast('approval.resolved', { approval: a });
    const waiter = a.run_id && runWaiters.get(a.run_id);
    if (waiter) {
      runWaiters.delete(a.run_id);
      waiter(a);
    }
    return sendJson(res, 200, a);
  }
  if (p === '/api/paypal/order') {
    if (!PP_SCENARIOS.some((s) => s.id === body.scenario)) throw new HttpError(400, 'unknown scenario');
    const { row, events } = makeOrder(body.scenario);
    setTimeout(() => broadcast('paypal.order', { row, events }), 120);
    return sendJson(res, 200, { order_id: row.id });
  }
  if (p === '/api/paypal/stream') {
    const { enabled, interval_ms: intervalMs } = body;
    if (typeof enabled !== 'boolean') throw new HttpError(400, 'enabled must be a boolean');
    // Same as aegis-daemon: a missing or non-positive interval keeps the current one, and the value
    // is clamped to STREAM_MIN_MS..STREAM_MAX_MS; the response carries the interval in effect.
    const requested = typeof intervalMs === 'number' && Number.isFinite(intervalMs) && intervalMs > 0 ? Math.round(intervalMs) : stream.interval_ms;
    setStream(enabled, Math.min(STREAM_MAX_MS, Math.max(STREAM_MIN_MS, requested)));
    return sendJson(res, 200, { ...stream });
  }
  if (p === '/__mock/emit') {
    // Dev-only: broadcast an arbitrary contract message to exercise rare shapes (e.g. submission "sent").
    if (typeof body.type !== 'string' || !body.type) throw new HttpError(400, 'type is required');
    const { type, ...fields } = body;
    broadcast(type, fields);
    return sendJson(res, 200, { emitted: type, clients: clients.size });
  }
  if (p === '/__mock/lag') {
    // Dev-only: the broadcast-lag notice aegis-daemon sends a client that fell behind.
    const dropped = Number.isInteger(body.dropped) && body.dropped > 0 ? body.dropped : 1;
    broadcast('log', { level: 'warn', message: `this client fell behind; ${dropped} messages were dropped`, dropped });
    return sendJson(res, 200, { emitted: 'log', dropped, clients: clients.size });
  }
  if (p === '/__mock/rotate-token') {
    TOKEN = crypto.randomBytes(24).toString('hex');
    for (const c of [...clients]) closeClient(c, 1012);
    return sendJson(res, 200, { rotated: true, note: 'dev-only mock route; reload the page to pick up the new token' });
  }
  throw new HttpError(404, 'not found');
}

const server = http.createServer((req, res) => {
  route(req, res).catch((err) => {
    const status = err instanceof HttpError ? err.status : 500;
    if (!res.headersSent) sendJson(res, status, { error: err instanceof HttpError ? err.message : 'internal mock error' });
    else res.end();
    if (status === 500) console.error('[mock] internal error:', err);
  }).finally(() => {
    console.log(`[mock] ${req.method} ${req.url.split('?')[0]} -> ${res.statusCode}`);
  });
});

// ---------------------------------------------------------------------------------------------
// WebSocket (RFC 6455), by hand: handshake, server->client text frames, close/ping/pong handling.
const clients = new Set();

function encodeFrame(opcode, payload) {
  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.alloc(2);
    header[1] = len;
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  header[0] = 0x80 | opcode; // FIN + opcode; server frames are never masked
  return Buffer.concat([header, payload]);
}

function sendFrame(client, opcode, payload) {
  if (!client.closed && client.socket.writable) client.socket.write(encodeFrame(opcode, payload));
}

function send(client, msg) {
  sendFrame(client, 0x1, Buffer.from(JSON.stringify(msg), 'utf8'));
}

function broadcast(type, fields = {}) {
  const msg = { type, ts: iso(), ...fields };
  if (RUN_TYPES.has(type)) recordRunMessage(msg);
  for (const c of clients) send(c, msg);
}

function closeClient(client, code = 1000) {
  if (client.closed) return;
  const payload = Buffer.alloc(2);
  payload.writeUInt16BE(code, 0);
  sendFrame(client, 0x8, payload);
  client.closed = true;
  clients.delete(client);
  client.socket.end();
  setTimeout(() => client.socket.destroy(), 1000).unref();
}

function onData(client, chunk) {
  client.buf = Buffer.concat([client.buf, chunk]);
  for (;;) {
    const buf = client.buf;
    if (buf.length < 2) return;
    const opcode = buf[0] & 0x0f;
    const masked = (buf[1] & 0x80) !== 0;
    let len = buf[1] & 0x7f;
    let off = 2;
    if (len === 126) {
      if (buf.length < 4) return;
      len = buf.readUInt16BE(2);
      off = 4;
    } else if (len === 127) {
      if (buf.length < 10) return;
      const big = buf.readBigUInt64BE(2);
      if (big > BigInt(MAX_BODY)) return closeClient(client, 1009);
      len = Number(big);
      off = 10;
    }
    if (!masked) return closeClient(client, 1002); // clients must mask
    if (buf.length < off + 4 + len) return;
    const mask = buf.subarray(off, off + 4);
    const payload = Buffer.from(buf.subarray(off + 4, off + 4 + len));
    for (let i = 0; i < payload.length; i += 1) payload[i] ^= mask[i % 4];
    client.buf = buf.subarray(off + 4 + len);
    if (opcode === 0x8) {
      const code = payload.length >= 2 ? payload.readUInt16BE(0) : 1000;
      return closeClient(client, code === 1005 || code === 1006 ? 1000 : code);
    }
    if (opcode === 0x9) sendFrame(client, 0xa, payload); // ping -> pong
    // Text/binary/continuation frames from the client are ignored: the contract has no client->server messages.
  }
}

server.on('upgrade', (req, socket) => {
  const reject = (status, text) => {
    const body = JSON.stringify({ error: text });
    socket.end(`HTTP/1.1 ${status} ${http.STATUS_CODES[status]}\r\nContent-Type: application/json\r\nContent-Length: ${Buffer.byteLength(body)}\r\nConnection: close\r\n\r\n${body}`);
    console.log(`[mock] WS upgrade rejected -> ${status}`);
  };
  const url = new URL(req.url, `http://${HOST}:${PORT}`);
  if (!hostAllowed(req)) return reject(403, 'Host/Origin not allowed');
  if (url.pathname !== '/ws') return reject(404, 'not found');
  if (url.searchParams.get('token') !== TOKEN) return reject(401, 'missing or wrong token');
  const key = req.headers['sec-websocket-key'];
  if (String(req.headers.upgrade || '').toLowerCase() !== 'websocket' || !key || req.headers['sec-websocket-version'] !== '13') return reject(400, 'bad WebSocket handshake');
  const accept = crypto.createHash('sha1').update(key + WS_GUID).digest('base64');
  socket.write(['HTTP/1.1 101 Switching Protocols', 'Upgrade: websocket', 'Connection: Upgrade', `Sec-WebSocket-Accept: ${accept}`, '', ''].join('\r\n'));
  socket.setNoDelay(true);
  const client = { socket, buf: Buffer.alloc(0), closed: false };
  clients.add(client);
  socket.on('data', (chunk) => onData(client, chunk));
  const drop = () => {
    client.closed = true;
    clients.delete(client);
  };
  socket.on('close', drop);
  socket.on('error', drop);
  console.log(`[mock] WS client connected (${clients.size} open)`);
  send(client, { type: 'hello', ts: iso(), health: health(), live: liveStats() });
});

const liveTimer = setInterval(() => broadcast('telemetry.live', { live: liveStats() }), 1000);
const keepwarmTimer = setInterval(keepwarmTick, KEEPWARM_INTERVAL_MS);

function shutdown(signal) {
  console.log(`[mock] ${signal}: shutting down`);
  clearInterval(liveTimer);
  clearInterval(keepwarmTimer);
  clearInterval(streamTimer);
  for (const c of [...clients]) closeClient(c, 1001);
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 1500).unref();
}
process.on('SIGINT', () => shutdown('SIGINT'));
process.on('SIGTERM', () => shutdown('SIGTERM'));

server.listen(PORT, HOST, () => {
  console.log(`[mock] Aegis Console contract mock on http://${HOST}:${PORT}/ (synthetic values; token substituted into index.html, not printed)`);
});
