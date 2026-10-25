// HTTP + WebSocket client for aegis-daemon (contract: apps/aegis-console/API.md).
//
// - GETs carry no token; every POST sends X-Aegis-Token and Content-Type: application/json.
// - 401/403 and network failures are broadcast on `bus` so the shell can show them visibly.
// - LiveSocket reconnects with capped exponential backoff and a liveness watchdog.

const meta = document.querySelector('meta[name="aegis-token"]');
const PLACEHOLDER = ['%%', 'AEGIS_TOKEN', '%%'].join(''); // assembled so no server-side substitution can touch it

export const TOKEN = (meta && meta.getAttribute('content')) || '';
export const TOKEN_MISSING = !TOKEN || TOKEN === PLACEHOLDER;

/** Shell-level events: 'auth' {status, message}, 'offline' {message}, 'online'. */
export const bus = new EventTarget();

function emit(type, detail) {
  bus.dispatchEvent(new CustomEvent(type, { detail }));
}

export class ApiError extends Error {
  constructor(status, message, body) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.body = body;
  }
}

/** Describes an error for inline display. */
export function describeError(err) {
  if (err instanceof ApiError) {
    if (err.status === 0) return `Daemon unreachable: ${err.message}`;
    return `HTTP ${err.status}: ${err.message}`;
  }
  return String((err && err.message) || err);
}

export async function api(method, path, body) {
  const init = { method, headers: { Accept: 'application/json' }, cache: 'no-store', credentials: 'same-origin' };
  if (method !== 'GET') {
    init.headers['Content-Type'] = 'application/json';
    init.headers['X-Aegis-Token'] = TOKEN;
    init.body = JSON.stringify(body === undefined ? {} : body);
  }
  let res;
  try {
    res = await fetch(path, init);
  } catch (err) {
    emit('offline', { message: String((err && err.message) || err) });
    throw new ApiError(0, 'no response (network error)');
  }
  emit('online');
  const text = await res.text();
  let data = null;
  if (text) {
    try { data = JSON.parse(text); } catch { data = null; }
  }
  if (!res.ok) {
    const message = (data && typeof data.error === 'string' && data.error) || res.statusText || 'request failed';
    if (res.status === 401 || res.status === 403) emit('auth', { status: res.status, message, path });
    throw new ApiError(res.status, message, data);
  }
  if (data === null && text) throw new ApiError(res.status, 'response was not JSON');
  return data;
}

export const get = (path) => api('GET', path);
export const post = (path, body) => api('POST', path, body);

/**
 * True for aegis-daemon's notice that this socket fell behind and broadcast messages were dropped
 * (a `log` message, see crates/aegis-daemon/src/api.rs). The page must then treat its view of
 * in-flight runs as incomplete and reload what it can.
 */
export function isLagNotice(msg) {
  return Boolean(msg) && msg.type === 'log' && typeof msg.message === 'string'
    && /fell behind; \d+ messages? (?:were|was) dropped/.test(msg.message);
}

const BACKOFF_BASE_MS = 500;
const BACKOFF_CAP_MS = 15000;
const WATCHDOG_MS = 15000; // telemetry.live arrives about once per second; silence this long means a dead link

/**
 * States reported to onState(state, info):
 *   'connecting' | 'open' | 'waiting' {attempt, delayMs, until, reason} where reason is
 *   'offline' (daemon unreachable), 'refused' (daemon up, socket rejected: stale token?), 'closed' (unknown yet).
 */
export class LiveSocket {
  constructor({ onMessage, onState, probe }) {
    this.onMessage = onMessage;
    this.onState = onState;
    this.probe = probe;
    this.attempt = 0;
    this.ws = null;
    this.timer = null;
    this.watchdog = null;
    this.stopped = false;
  }

  url() {
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
    return `${scheme}://${location.host}/ws?token=${encodeURIComponent(TOKEN)}`;
  }

  connect() {
    clearTimeout(this.timer);
    this.timer = null;
    if (this.stopped) return;
    this.onState('connecting', { attempt: this.attempt });
    let ws;
    try {
      ws = new WebSocket(this.url());
    } catch (err) {
      this.schedule('closed');
      return;
    }
    this.ws = ws;
    let opened = false;
    ws.addEventListener('open', () => {
      opened = true;
      this.onState('open', {});
      this.kick();
    });
    ws.addEventListener('message', (ev) => {
      this.kick();
      let msg;
      try { msg = JSON.parse(ev.data); } catch { return; }
      if (msg && typeof msg.type === 'string') {
        if (msg.type === 'hello') this.attempt = 0;
        this.onMessage(msg);
      }
    });
    ws.addEventListener('close', () => {
      clearTimeout(this.watchdog);
      if (this.ws !== ws) return;
      this.ws = null;
      this.schedule(opened ? 'closed' : 'closed-before-open');
    });
  }

  kick() {
    clearTimeout(this.watchdog);
    this.watchdog = setTimeout(() => {
      if (this.ws) this.ws.close(4000, 'watchdog: no messages');
    }, WATCHDOG_MS);
  }

  async schedule(why) {
    if (this.stopped) return;
    this.attempt += 1;
    const raw = Math.min(BACKOFF_CAP_MS, BACKOFF_BASE_MS * 2 ** (this.attempt - 1));
    const delayMs = Math.round(raw * (0.75 + Math.random() * 0.5));
    const until = Date.now() + delayMs;
    this.timer = setTimeout(() => this.connect(), delayMs);
    this.onState('waiting', { attempt: this.attempt, delayMs, until, reason: 'closed' });
    // Distinguish "daemon down" from "daemon up but socket refused" (a browser cannot read the WS status code).
    const reason = await this.probe(why).catch(() => 'offline');
    if (this.timer && !this.stopped && !this.ws) this.onState('waiting', { attempt: this.attempt, delayMs, until, reason });
  }

  reconnectNow() {
    if (this.ws) return;
    clearTimeout(this.timer);
    this.attempt = Math.max(0, this.attempt - 1);
    this.connect();
  }
}
