// Records a showcase of the live Aegis Console with headless Chrome over the DevTools protocol.
//
//   node harness/demo/record_demo.mjs [--url http://127.0.0.1:4173/] [--out artifacts] [--work DIR]
//
// Captures screencast frames (with timestamps) while it clicks through every workspace scenario,
// the team approval flow and the PayPal orders + stream, then writes high-resolution screenshots.
// Encode the frames afterwards with harness/demo/frames_to_mp4.swift (see harness/demo/record.sh).
// Everything shown is the daemon's own live output; nothing is staged or edited.
import { spawn } from 'node:child_process';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const arg = (name, fallback) => {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : fallback;
};
const URL_ = arg('--url', 'http://127.0.0.1:4173/');
const OUT = resolve(arg('--out', 'artifacts'));
const WORK = resolve(arg('--work', 'target/demo-recording'));
const FRAMES = join(WORK, 'frames');
const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const [W, H] = [1920, 1080];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

rmSync(WORK, { recursive: true, force: true });
mkdirSync(FRAMES, { recursive: true });
mkdirSync(OUT, { recursive: true });

const port = 9300 + (process.pid % 500);
const chrome = spawn(CHROME, [
  '--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${join(WORK, 'profile')}`,
  `--window-size=${W},${H}`, '--hide-scrollbars', '--force-color-profile=srgb', '--no-first-run',
  '--no-default-browser-check', '--disable-extensions', 'about:blank',
], { stdio: 'ignore' });
process.on('exit', () => chrome.kill('SIGTERM'));

async function target() {
  for (let i = 0; i < 100; i += 1) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = list.find((t) => t.type === 'page');
      if (page) return page.webSocketDebuggerUrl;
    } catch { /* not up yet */ }
    await sleep(100);
  }
  throw new Error('Chrome DevTools endpoint did not come up');
}

const ws = new WebSocket(await target());
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0;
const pending = new Map();
const frames = [];
ws.onmessage = (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) {
    const { resolve: ok, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.error) reject(new Error(JSON.stringify(msg.error))); else ok(msg.result);
  } else if (msg.method === 'Page.screencastFrame') {
    const { data, metadata, sessionId } = msg.params;
    const file = `frame-${String(frames.length).padStart(6, '0')}.jpg`;
    writeFileSync(join(FRAMES, file), Buffer.from(data, 'base64'));
    frames.push({ file, t: metadata.timestamp });
    send('Page.screencastFrameAck', { sessionId }).catch(() => {});
  }
};
function send(method, params = {}) {
  const id = ++seq;
  ws.send(JSON.stringify({ id, method, params }));
  return new Promise((ok, reject) => pending.set(id, { resolve: ok, reject }));
}
async function js(expression) {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(`page error: ${r.exceptionDetails.text}`);
  return r.result.value;
}
// Clicks the first visible button/role=button whose text starts with `text` (a prefix, so a scenario
// card whose description mentions "Approve" or "Reject" never matches those labels).
const click = (text) => js(`(() => {
  const want = ${JSON.stringify(text)}.toLowerCase();
  const els = [...document.querySelectorAll('button, [role="button"]')]
    .filter((e) => e.offsetParent !== null && !e.disabled && (e.textContent || '').trim().toLowerCase().startsWith(want));
  const el = els[0];
  if (!el) return false;
  el.scrollIntoView({ behavior: 'smooth', block: 'center' });
  el.click();
  return true;
})()`);
const scrollTo = (selectorOrText) => js(`(() => {
  const q = ${JSON.stringify(selectorOrText)};
  let el = null;
  try { el = document.querySelector(q); } catch {}
  if (!el) el = [...document.querySelectorAll('h1,h2,h3,h4')].find((h) => h.textContent.toLowerCase().includes(q.toLowerCase()));
  if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
  return !!el;
})()`);
const api = (path) => js(`fetch(${JSON.stringify(path)}).then((r) => r.json())`);
const runIds = async () => new Set((await api('/api/runs')).runs.map((r) => r.run_id));
// Waits until a run that was not in `before` exists and every run has finished.
async function waitNewRunDone(before, timeoutMs = 180000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    const { runs } = await api('/api/runs');
    const fresh = runs.filter((r) => !before.has(r.run_id));
    if (fresh.length && runs.every((r) => r.outcome && r.outcome !== 'running')) return fresh;
    await sleep(400);
  }
  throw new Error('the new run did not finish in time');
}
const pendingCount = async () => (await api('/api/approvals')).pending.length;
async function resolveFirst(label) {
  const before = await pendingCount();
  if (!(await click(label))) throw new Error(`${label} button not found`);
  const end = Date.now() + 30000;
  while (Date.now() < end) {
    if ((await pendingCount()) < before) return;
    await sleep(300);
  }
  throw new Error(`${label} did not resolve an approval`);
}
async function shot(name, scale = 2) {
  await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: scale, mobile: false });
  await sleep(400);
  const { data } = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  writeFileSync(join(OUT, name), Buffer.from(data, 'base64'));
  await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
  console.log(`screenshot ${join(OUT, name)}`);
}

await send('Page.enable');
await send('Runtime.enable');
await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url: URL_ });
await sleep(2500);
const token = await js(`document.querySelector('meta[name="aegis-token"]').content`);
const post = (path, body) => js(`fetch(${JSON.stringify(path)}, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Aegis-Token': ${JSON.stringify(token)} }, body: JSON.stringify(${JSON.stringify(body)}) }).then((r) => r.status)`);

// Start from a clean demo tree (the console's own Reset button does the same; it also rolls back
// approvals still pending from earlier runs).
await post('/api/workspace/reset', {});
await send('Page.reload');
await sleep(2500);

await send('Page.startScreencast', { format: 'jpeg', quality: 88, maxWidth: W, maxHeight: H, everyNthFrame: 1 });
await sleep(2500);

const scenarios = ['Safe multi-file refactor', 'Unformatted secret', 'Build break and recovery', 'Mass delete', 'Mass delete'];
for (const s of scenarios) {
  await scrollTo('Scenarios');
  await sleep(700);
  const before = await runIds();
  if (!(await click(s))) throw new Error(`scenario button not found: ${s}`);
  await waitNewRunDone(before);
  await sleep(2200);
}

await scrollTo('Pending approvals');
await sleep(1500);
await resolveFirst('Reject');
await sleep(2500);
await resolveFirst('Approve');
await sleep(3000);

await scrollTo('PayPal');
await sleep(1200);
for (const o of ['$42.00 known payee', '$120.00 known payee', '$480.00 new payee', '$25.00 with redaction']) {
  if (!(await click(o))) throw new Error(`order button not found: ${o}`);
  await sleep(1800);
}
if (await click('Stream')) {
  await sleep(6500);
  await click('Stream');
}
await sleep(1500);
await scrollTo('Timeline');
await sleep(2500);
await scrollTo('Scoreboard');
await sleep(3000);
await js('window.scrollTo({ top: 0, behavior: "smooth" })');
await sleep(2500);
await send('Page.stopScreencast');
await sleep(300);
writeFileSync(join(FRAMES, 'frames.json'), JSON.stringify(frames));
console.log(`captured ${frames.length} frames over ${(frames.at(-1).t - frames[0].t).toFixed(1)} s into ${FRAMES}`);

await js('window.scrollTo(0, 0)');
await sleep(800);
await shot('console_cockpit_hero.png');
await scrollTo('PayPal');
await sleep(1200);
await shot('console_paypal_aggrid.png');
await scrollTo('Scoreboard');
await sleep(1200);
await shot('console_scoreboard_verified.png');

ws.close();
chrome.kill('SIGTERM');
process.exit(0);
