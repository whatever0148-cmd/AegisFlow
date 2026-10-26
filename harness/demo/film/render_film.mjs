// Renders artifacts/aegisflow_demo.mp4: a frame-exact product film of the live Aegis Console.
//
//   FFMPEG=/path/to/ffmpeg node harness/demo/film/render_film.mjs [--preview] [--from S --to S]
//       [--url http://127.0.0.1:4173/] [--out artifacts/aegisflow_demo.mp4] [--voice artifacts/voiceover.wav]
//
// Headless Chrome loads the real console with a virtual clock (vtime.js), stage.js wraps it in a
// window with a spring camera, captions and callouts, and every frame is captured at 2x
// (3840x2160) and downscaled to 1080p60 by ffmpeg (libx264, CRF 16). The clicks are real: the
// daemon runs every scenario while the film waits, so each value on screen is the daemon's own.
// Afterwards the voiceover is mixed with generated UI sound (sfx.mjs) into the final MP4.
import { spawn } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch } from './cdp.mjs';
import { buildFilm } from './script.mjs';
import { renderSfx } from './sfx.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '../../..');
const arg = (name, fallback) => { const i = process.argv.indexOf(name); return i >= 0 ? process.argv[i + 1] : fallback; };
const PREVIEW = process.argv.includes('--preview');
const URL_ = arg('--url', 'http://127.0.0.1:4173/');
const OUT = resolve(arg('--out', PREVIEW ? join(ROOT, 'target/film/preview.mp4') : join(ROOT, 'artifacts/aegisflow_demo.mp4')));
const VOICE = resolve(arg('--voice', join(ROOT, 'artifacts/voiceover.wav')));
const WORK = resolve(arg('--work', join(ROOT, 'target/film')));
const FFMPEG = process.env.FFMPEG || 'ffmpeg';
const FPS = Number(arg('--fps', PREVIEW ? 30 : 60));
const DSF = Number(arg('--scale', PREVIEW ? 1 : 2));
const FROM = Number(arg('--from', 0));
const TO = arg('--to', null);
const [W, H] = [1920, 1080];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...m) => console.log(`[film]`, ...m);

const artifact = JSON.parse(readFileSync(join(ROOT, 'artifacts/aegis_phase1_telemetry.json'), 'utf8'));
const rawWords = JSON.parse(readFileSync(join(HERE, 'voiceover_words.json'), 'utf8'));
const { spec, actions } = buildFilm(artifact, rawWords);
const END = TO ? Number(TO) : spec.duration;

rmSync(join(WORK, 'profile'), { recursive: true, force: true });
mkdirSync(WORK, { recursive: true });
mkdirSync(dirname(OUT), { recursive: true });

const b = await launch({ work: WORK, width: W, height: H });
await b.send('Page.enable');
await b.send('Runtime.enable');
await b.send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: DSF, mobile: false });
await b.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'no-preference' }] });
await b.send('Page.addScriptToEvaluateOnNewDocument', { source: readFileSync(join(HERE, 'vtime.js'), 'utf8') });

const api = (path) => b.js(`fetch(${JSON.stringify(path)}).then((r) => r.json())`);
let token = null;
const post = (path, body) => b.js(`fetch(${JSON.stringify(path)}, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Aegis-Token': ${JSON.stringify(token)} }, body: JSON.stringify(${JSON.stringify(body)}) }).then((r) => r.json())`);
// Lets the console's timers run in real time while nothing is being filmed.
async function settle(ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    await b.js('window.__vt.advanceTo(window.__vt.now + 50)');
    await sleep(50);
  }
}
async function until(what, fn, timeoutMs = 240000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    const v = await fn();
    if (v) return v;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

// ---- prepare the daemon: clean demo tree, warm caches, no stream --------------------------------
await b.send('Page.navigate', { url: URL_ });
await sleep(1500);
token = await b.js(`document.querySelector('meta[name="aegis-token"]').content`);
// MLComputePlan can come back empty at daemon start; never film a console that shows ANE 0.
const plan = (await api('/api/health')).model.compute_plan;
if (!(plan && plan.neural_engine > 0)) throw new Error(`daemon reports MLComputePlan ${JSON.stringify(plan)}; restart it (bash harness/console.sh) before filming`);
await post('/api/paypal/stream', { enabled: false, interval_ms: 2500 });
await post('/api/workspace/reset', {});
for (const id of (await api('/api/approvals')).pending.map((p) => p.id)) await post(`/api/approvals/${id}/reject`, { by: 'film-setup' });
// Two off-camera warm-up runs so the first filmed run is not paying first-call costs.
for (let i = 0; i < 2; i += 1) {
  const { run_id: id } = await post('/api/workspace/run', { scenario: 'safe_refactor' });
  await until('warm-up run', async () => (await api('/api/runs')).runs.find((r) => r.run_id === id && r.outcome != null));
}
await post('/api/workspace/reset', {});
await b.send('Page.reload');
await sleep(1200);
await settle(3500);
await until('console WebSocket', () => b.js(`document.getElementById('conn').dataset.state === 'open'`), 20000);
await settle(800);

await b.js(readFileSync(join(HERE, 'stage.js'), 'utf8'));
const setup = await b.js(`window.__film.setup(${JSON.stringify(spec)})`);
log(`stage ready, page height ${setup.scrollHeight}px; ${Math.round(END * FPS)} frames at ${FPS} fps, ${W * DSF}x${H * DSF} capture`);

// ---- ffmpeg: JPEG frames in, 1080p H.264 out ------------------------------------------------------
const silent = join(WORK, 'video_only.mp4');
const enc = spawn(FFMPEG, [
  '-hide_banner', '-loglevel', 'error', '-y',
  '-f', 'image2pipe', '-framerate', String(FPS), '-c:v', 'mjpeg', '-i', '-',
  '-vf', `scale=${W}:${H}:flags=lanczos+accurate_rnd+full_chroma_int:out_color_matrix=bt709,format=yuv420p`,
  '-c:v', 'libx264', '-preset', PREVIEW ? 'veryfast' : 'slow', '-crf', PREVIEW ? '22' : '16', '-tune', 'film',
  '-profile:v', 'high', '-r', String(FPS), '-g', String(FPS * 2),
  '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv',
  '-movflags', '+faststart', silent,
], { stdio: ['pipe', 'inherit', 'inherit'] });
const encDone = new Promise((ok, fail) => enc.on('close', (code) => (code === 0 ? ok() : fail(new Error(`ffmpeg exited ${code}`)))));
const write = (buf) => new Promise((ok) => { if (enc.stdin.write(buf)) ok(); else enc.stdin.once('drain', ok); });

// ---- the frame loop -------------------------------------------------------------------------------
const events = [];
const pending = [...actions].sort((x, y) => x.t - y.t);
let runsBefore = new Set();
let ordersBefore = 0;
let pendingBefore = 0;

async function perform(act) {
  if (act.quiet) {
    await b.js(`(() => { const n = document.querySelector(${JSON.stringify(act.quiet.sel)}); if (n) n.click(); return !!n; })()`);
    return;
  }
  runsBefore = new Set((await api('/api/runs')).runs.map((r) => r.run_id));
  ordersBefore = (await api('/api/paypal/orders')).rows.length;
  pendingBefore = (await api('/api/approvals')).pending.length;
  if (act.pace) {
    // The order stream runs in real time while frames take real time to capture; pick an interval
    // that shows roughly one new order per video second, and say on screen that it is sped up.
    const realPerVideoSec = pace.length > 1 ? (pace[pace.length - 1].ms - pace[0].ms) / 1000 / (pace[pace.length - 1].t - pace[0].t) : 1;
    const interval = Math.max(1000, Math.min(600000, Math.round(realPerVideoSec * 900)));
    await b.js(`(() => { const n = document.querySelector(${JSON.stringify(act.pace)}); n.value = '${interval}'; n.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
    log(`  stream interval ${interval} ms (${realPerVideoSec.toFixed(2)} s of capture per video second)`);
  }
  const ok = await b.js(`window.__film.click(${JSON.stringify(act.click)}, ${act.t})`);
  if (!ok) throw new Error(`click target not found at ${act.t}s: ${JSON.stringify(act.click)}`);
  events.push({ t: act.t, kind: 'click' });
  const t0 = Date.now();
  if (act.wait === 'run') {
    const run = await until(`run after ${act.t}s`, async () => {
      const { runs } = await api('/api/runs');
      const fresh = runs.find((r) => !runsBefore.has(r.run_id));
      return fresh && fresh.outcome != null ? fresh : null;
    });
    log(`  run ${run.run_id} ${run.scenario} → ${run.outcome} in ${((Date.now() - t0) / 1000).toFixed(1)} s real time`);
    if (act.patch === 'heal') {
      const tier = run.stages.filter((s) => s.stage === 'recover' && s.status === 'ok' && s.detail && s.detail.tier >= 1).pop();
      if (!tier) throw new Error(`build_break run ${run.run_id} did not recover through a remote tier (outcome ${run.outcome}); not filming a failed heal`);
      const secs = (tier.ms / 1000).toFixed(1);
      await b.js(`window.__film.patch('lowers', 'heal', ${JSON.stringify({ value: `${secs} s live call`, sub: `${tier.detail.model} · patch re-checked by rustc and Cedar before commit` })})`);
      await b.js(`window.__film.patch('badges', 'lapse', ${JSON.stringify({ text: `Live Nemotron call took ${secs} s, shortened here` })})`);
    }
    if (act.patch === 'hold') {
      const score = run.stages.filter((s) => s.stage === 'score').pop();
      const risk = score && score.detail ? Number(score.detail.risk).toFixed(2) : '?';
      await b.js(`window.__film.patch('lowers', 'hold', ${JSON.stringify({ value: `risk ${risk}` })})`);
    }
  } else if (act.wait === 'resolved') {
    await until('approval resolved', async () => (await api('/api/approvals')).pending.length < pendingBefore);
  } else if (act.wait === 'order') {
    await until('order row', async () => (await api('/api/paypal/orders')).rows.length > ordersBefore || ordersBefore >= 20);
  } else if (act.wait === 'receipt') {
    await until('receipt dialog', () => b.js(`(() => { const d = document.getElementById('receipt-dialog'); return d.open && d.querySelector('[data-role=body]').textContent.length > 40; })()`), 15000);
  } else if (act.wait === 'stream') {
    await until('stream on', async () => (await api('/api/paypal/scenarios')).stream.enabled === true, 15000);
  }
  // Let WebSocket messages that arrived during the wait reach the page before the next frame.
  await sleep(act.wait ? 350 : 60);
}

const pace = [];
const total = Math.round(END * FPS);
const first = Math.round(FROM * FPS);
const started = Date.now();
for (let i = 0; i < total; i += 1) {
  const t = i / FPS;
  while (pending.length && pending[0].t <= t + 1e-9) await perform(pending.shift());
  await b.js(`window.__film.frame(${t})`);
  if (i < first) continue;
  pace.push({ t, ms: Date.now() });
  if (pace.length > FPS * 4) pace.shift();
  const { data } = await b.send('Page.captureScreenshot', { format: 'jpeg', quality: PREVIEW ? 82 : 94, optimizeForSpeed: true });
  await write(Buffer.from(data, 'base64'));
  if (i % (FPS * 5) === 0) {
    const el = (Date.now() - started) / 1000;
    log(`t=${t.toFixed(1)}s frame ${i}/${total} · ${el.toFixed(0)} s elapsed`);
  }
}
for (const d of await b.js('window.__film.debug()')) log(`  shot ${d.t}s rect ${JSON.stringify(d.rect)} → ${JSON.stringify(d.to)}`);
enc.stdin.end();
await encDone;
writeFileSync(join(WORK, 'events.json'), JSON.stringify({ events, spec: { duration: spec.duration, lowers: spec.lowers.map((l) => l.t) } }, null, 1));
b.close();

// ---- sound: voiceover + UI sound design, then mux -----------------------------------------------
const sfxPath = join(WORK, 'sfx.wav');
writeFileSync(sfxPath, renderSfx({ duration: spec.duration, clicks: events.map((e) => e.t), lowers: spec.lowers.map((l) => l.t), spec }));
const offset = FROM.toFixed(3);
const mix = spawn(FFMPEG, [
  '-hide_banner', '-loglevel', 'error', '-y',
  '-i', silent, '-ss', offset, '-i', VOICE, '-ss', offset, '-i', sfxPath,
  '-filter_complex',
  '[1:a]aresample=48000,aformat=channel_layouts=stereo,highpass=f=70,acompressor=threshold=-20dB:ratio=2.5:attack=8:release=160:makeup=2[v];'
  + '[2:a]aresample=48000[s];[v][s]amix=inputs=2:duration=longest:normalize=0,loudnorm=I=-15:TP=-1.5:LRA=9[a]',
  '-map', '0:v', '-map', '[a]', '-c:v', 'copy', '-c:a', 'aac', '-b:a', '192k', '-ar', '48000',
  '-t', (END - FROM).toFixed(3), '-movflags', '+faststart', OUT,
], { stdio: 'inherit' });
await new Promise((ok, fail) => mix.on('close', (code) => (code === 0 ? ok() : fail(new Error(`ffmpeg mux exited ${code}`)))));
log(`wrote ${OUT} in ${((Date.now() - started) / 60000).toFixed(1)} min`);
process.exit(0);
