// Minimal Chrome DevTools Protocol client: launches headless Chrome and talks to its first page.
import { spawn } from 'node:child_process';
import { join } from 'node:path';

const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function launch({ work, width, height }) {
  const port = 9400 + (process.pid % 500);
  const chrome = spawn(CHROME, [
    '--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${join(work, 'profile')}`,
    `--window-size=${width},${height}`, '--hide-scrollbars', '--force-color-profile=srgb', '--no-first-run',
    '--no-default-browser-check', '--disable-extensions', '--font-render-hinting=none', 'about:blank',
  ], { stdio: 'ignore' });
  process.on('exit', () => chrome.kill('SIGTERM'));
  let url = null;
  for (let i = 0; i < 100 && !url; i += 1) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      url = list.find((t) => t.type === 'page')?.webSocketDebuggerUrl;
    } catch { /* not up yet */ }
    if (!url) await sleep(100);
  }
  if (!url) throw new Error('Chrome DevTools endpoint did not come up');
  const ws = new WebSocket(url);
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
  let seq = 0;
  const pending = new Map();
  const listeners = new Map();
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { ok, fail } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) fail(new Error(JSON.stringify(msg.error))); else ok(msg.result);
    } else if (msg.method && listeners.has(msg.method)) {
      for (const fn of listeners.get(msg.method)) fn(msg.params);
    }
  };
  const send = (method, params = {}) => {
    const id = ++seq;
    ws.send(JSON.stringify({ id, method, params }));
    return new Promise((ok, fail) => pending.set(id, { ok, fail }));
  };
  const on = (method, fn) => listeners.set(method, [...(listeners.get(method) || []), fn]);
  const js = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`page error: ${r.exceptionDetails.exception?.description || r.exceptionDetails.text}`);
    return r.result.value;
  };
  const close = () => { ws.close(); chrome.kill('SIGTERM'); };
  return { send, on, js, close };
}
