// Aegis Console entry point: loads every panel from the daemon's GET endpoints, then keeps them
// current from the WebSocket. Contract: apps/aegis-console/API.md.

import { TOKEN_MISSING, bus, get, isLagNotice, LiveSocket } from './api.js';
import { setHealth, setConnState, showBanner, hideBanner, bindRetry } from './shell.js';
import { fmtDateTime } from './dom.js';
import { createWorkspacePanel } from './workspace.js';
import { createPaypalPanel } from './paypal.js';
import { createApprovalsPanel } from './approvals.js';
import { createScoreboard } from './scoreboard.js';
import { createSettings } from './settings.js';

const workspace = createWorkspacePanel();
const paypal = createPaypalPanel();
const approvals = createApprovalsPanel();
const scoreboard = createScoreboard();
const settings = createSettings({
  onConfig(cfg) {
    const services = cfg && cfg.services;
    workspace.setServices(services);
    paypal.setServices(services);
  },
});

const reload = { label: 'Reload page', onClick: () => location.reload() };
let firstStartedAt = null;
let hellos = 0;
let socketWasOpen = false;

if (TOKEN_MISSING) {
  showBanner('token', 'error',
    'No session token: this page was not served by aegis-daemon (the token placeholder was not substituted), so every action will be rejected. Open the console from the daemon.');
}

function noteHealth(health) {
  if (!health || typeof health !== 'object') return;
  setHealth(health);
  if (!health.started_at) return;
  if (firstStartedAt === null) {
    firstStartedAt = health.started_at;
  } else if (health.started_at !== firstStartedAt) {
    showBanner('restart', 'warn',
      `The daemon restarted (now running since ${fmtDateTime(health.started_at)}). This page holds the previous process's session token: reload to continue.`, [reload]);
  }
}

async function load(path, ok, bad) {
  try {
    ok(await get(path));
  } catch (err) {
    bad(err);
  }
}

function loadAll() {
  return Promise.all([
    load('/api/health', noteHealth, () => {}),
    load('/api/telemetry', scoreboard.setTelemetry, scoreboard.fail),
    load('/api/config', settings.setConfig, settings.fail),
    load('/api/workspace', workspace.setWorkspace, workspace.fail),
    load('/api/approvals', approvals.setApprovals, approvals.fail),
    load('/api/paypal/orders', paypal.setOrders, paypal.failOrders),
    load('/api/paypal/scenarios', paypal.setScenarios, paypal.failScenarios),
  ]);
}

function onMessage(msg) {
  switch (msg.type) {
    case 'hello':
      hellos += 1;
      noteHealth(msg.health);
      if (msg.live) {
        scoreboard.setLive(msg.live);
        workspace.setCounters(msg.live.counters);
      }
      hideBanner('offline');
      hideBanner('ws-refused');
      if (hellos > 1) {
        // Runs tracked across the gap were marked partial when the socket closed (onState below):
        // the daemon does not replay run events.
        workspace.log('info', 'reconnected; state reloaded from the daemon (run events from the gap are not replayed)', msg.ts);
        loadAll();
      }
      break;
    case 'telemetry.live':
      if (msg.live) {
        scoreboard.setLive(msg.live);
        workspace.setCounters(msg.live.counters);
      }
      break;
    case 'run.started':
    case 'run.stage':
    case 'run.finished':
      workspace.onMessage(msg);
      break;
    case 'approval.created':
    case 'approval.resolved':
      approvals.onMessage(msg);
      workspace.onMessage(msg);
      break;
    case 'paypal.order':
      paypal.onMessage(msg);
      break;
    case 'config.changed':
      settings.onMessage(msg);
      workspace.onMessage(msg);
      break;
    case 'log':
      workspace.onMessage(msg);
      if (isLagNotice(msg)) {
        workspace.markGap('lag', msg.ts);
        loadAll();
      }
      break;
    default:
      break; // unknown message types are ignored (forward compatible)
  }
}

// After a socket failure: is the daemon down, or up and refusing the socket (bad/stale token, Host/Origin)?
async function probe(why) {
  try {
    noteHealth(await get('/api/health'));
  } catch {
    showBanner('offline', 'error', `Daemon offline: no answer from ${location.host}. Live updates resume automatically when it is back.`);
    return 'offline';
  }
  hideBanner('offline');
  if (why === 'closed-before-open') {
    showBanner('ws-refused', 'error',
      'The daemon answers HTTP but refused the live WebSocket (401/403). The session token is probably stale (daemon restarted?) or the page was opened through another host name. Reload the page.', [reload]);
    return 'refused';
  }
  return 'closed';
}

bus.addEventListener('auth', (ev) => {
  const { status, message, path } = ev.detail;
  if (status === 401) {
    showBanner('auth', 'error', `Unauthorized (401) on ${path}: ${message}. The daemon rejected this page's session token; reload to get a fresh one.`, [reload]);
  } else {
    const port = location.port ? `:${location.port}` : '';
    showBanner('auth', 'error', `Forbidden (403) on ${path}: ${message}. The daemon's Host/Origin guard rejected the request; open the console at http://localhost${port} or http://127.0.0.1${port}.`);
  }
});
bus.addEventListener('offline', () => {
  showBanner('offline', 'error', `Daemon offline: no answer from ${location.host}. Live updates resume automatically when it is back.`);
});
bus.addEventListener('online', () => hideBanner('offline'));

const socket = new LiveSocket({
  onMessage,
  onState: (state, info) => {
    setConnState(state, info);
    if (state === 'open') hideBanner('ws-refused');
    // Events broadcast while the socket is down are lost; mark tracked runs partial from this moment.
    if (state === 'waiting' && socketWasOpen) workspace.markGap('disconnect');
    if (state === 'open' || state === 'waiting') socketWasOpen = state === 'open';
  },
  probe,
});
bindRetry(() => socket.reconnectNow());

loadAll();
socket.connect();
