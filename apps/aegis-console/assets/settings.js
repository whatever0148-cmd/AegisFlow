// Settings drawer: key presence (set/unset + source), write-only inputs, Save, per-service Test.
// Inputs are never prefilled; Save posts only the fields the user typed (or explicitly marked to clear).
// A key with `choices` (NEBIUS_BASE_URL: the allowlisted Nemotron providers) renders as a selector
// whose first option keeps the current value; "Default" posts "" (unset), like "Clear on save".
// Keys the daemon reports with console_writable:false are read-only here: the daemon refuses to
// set them over HTTP (400), but "Clear on save" (unset) stays available.

import { h, replace, badge, fmtMs, fmtText, DASH } from './dom.js';
import { post, describeError } from './api.js';

const TESTABLE = ['nebius', 'tavily', 'paypal'];
const SERVICE_LABELS = { nebius: 'Nemotron provider', tavily: 'Tavily', paypal: 'PayPal Sandbox', mattermost: 'Mattermost', slack: 'Slack' };
/** Selector value meaning "unset this key" (posted as ""); '' means "keep the current value". */
export const DEFAULT_CHOICE = '__default__';
const TIER_KEYS = { NEBIUS_TIER1_MODEL: 'tier1_model', NEBIUS_TIER2_MODEL: 'tier2_model' };

const trimSlash = (v) => (typeof v === 'string' ? v.trim().replace(/\/$/, '') : '');

/** The allowlisted provider whose base_url is `url` (one trailing slash ignored), or null. */
export function providerFor(cfg, url) {
  const list = (cfg && Array.isArray(cfg.nemotron_providers)) ? cfg.nemotron_providers : [];
  const want = trimSlash(url);
  return want ? (list.find((p) => p && trimSlash(p.base_url) === want) || null) : null;
}

/** Extra hint text for the tier-model keys: the selected provider's default, or that one is required. */
export function tierHint(cfg, name) {
  const field = TIER_KEYS[name];
  const nebius = cfg && cfg.services && cfg.services.nebius;
  const provider = field && nebius ? providerFor(cfg, nebius.base_url) : null;
  if (!provider) return '';
  return provider[field]
    ? ` ${provider.label} default: ${provider[field]}.`
    : ` Required with ${provider.label}: it has no confirmed default model id (pick one from its GET /models; “Test Nemotron provider” lists them).`;
}

export function createSettings({ onConfig }) {
  const dialog = document.getElementById('settings');
  const opener = document.getElementById('open-settings');
  const $ = (role) => dialog.querySelector(`[data-role="${role}"]`);
  const els = { error: $('error'), path: $('path'), form: $('form'), keys: $('keys'), save: $('save'), status: $('save-status'), services: $('services'), close: $('close') };

  let config = null;
  const clearing = new Set();
  const testResults = new Map();
  const testing = new Set();

  function open() {
    if (typeof dialog.showModal === 'function') dialog.showModal();
    else dialog.setAttribute('open', '');
    const first = inputs().find((i) => !i.disabled);
    (first || els.close).focus();
  }

  function close() {
    if (typeof dialog.close === 'function') dialog.close();
    else dialog.removeAttribute('open');
  }

  opener.addEventListener('click', open);
  els.close.addEventListener('click', close);
  dialog.addEventListener('close', () => opener.focus());
  dialog.addEventListener('click', (ev) => {
    if (ev.target === dialog) close(); // backdrop click
  });

  function inputs() {
    // Text inputs and provider selectors, in document order.
    return [...els.keys.querySelectorAll('[data-key]')].filter((el) => el.localName === 'input' || el.localName === 'select');
  }

  /** NEBIUS_BASE_URL-style selector: keep current / each allowed choice / default (unset). */
  function choiceSelect(k, { id, hintId, marked }) {
    const nebius = config && config.services && config.services.nebius;
    const currentUrl = nebius ? nebius.base_url : null;
    const current = (k.choices.find((c) => trimSlash(c.value) === trimSlash(currentUrl)) || null);
    const currentLabel = current ? current.label : (nebius && nebius.provider) || (k.set ? 'set' : 'default');
    const first = k.choices[0];
    return h('select', { id, name: k.name, dataset: { key: k.name, choice: 'true' }, 'aria-describedby': hintId, disabled: marked },
      h('option', { value: '' }, `Keep current (${currentLabel})`),
      k.choices.map((c) => h('option', { value: c.value }, `${c.label}${current === c ? ' (current)' : ''} — ${c.value}`)),
      h('option', { value: DEFAULT_CHOICE }, `Default (unset${first ? `: ${first.label}` : ''})`));
  }

  /** Per-provider key links and default-model notes under the provider selector. */
  function providerNotes() {
    const list = (config && Array.isArray(config.nemotron_providers)) ? config.nemotron_providers : [];
    if (!list.length) return null;
    const models = (p) => {
      const t1 = p.tier1_model ? `tier 1 ${p.tier1_model}` : 'tier 1: set NEBIUS_TIER1_MODEL';
      const t2 = p.tier2_model ? `tier 2 ${p.tier2_model}` : 'tier 2: set NEBIUS_TIER2_MODEL';
      return `${t1} · ${t2}`;
    };
    return h('ul', { class: 'providers small', dataset: { role: 'providers' } }, list.map((p) => h('li', { dataset: { provider: p.id } },
      h('strong', { text: p.label }), ' ', h('span', { class: 'mono muted', text: models(p) }), ' ',
      p.signup_url ? h('a', { href: p.signup_url, target: '_blank', rel: 'noopener noreferrer', class: 'link' },
        p.key_hint || 'Get a key', h('span', { 'aria-hidden': 'true', text: ' ↗' }), h('span', { class: 'sr-only', text: ' (opens in a new tab)' })) : null)));
  }

  function renderKeys() {
    // Keep anything the user has typed but not saved yet; values never leave the DOM except via Save.
    const typed = new Map(inputs().filter((i) => i.value !== '').map((i) => [i.dataset.key, i.value]));
    const ds = document.activeElement && els.keys.contains(document.activeElement) ? document.activeElement.dataset : null;
    const active = ds && ds.key ? `[data-key="${CSS.escape(ds.key)}"]` : (ds && ds.clear ? `[data-clear="${CSS.escape(ds.clear)}"]` : null);
    const keys = (config && Array.isArray(config.keys)) ? config.keys : [];
    els.path.textContent = fmtText(config && config.path);
    if (!keys.length) {
      replace(els.keys, h('p', { class: 'muted', text: 'The daemon reported no configurable keys.' }));
      return;
    }
    replace(els.keys, keys.map((k) => {
      const id = `cfg-${k.name}`;
      const hintId = `${id}-hint`;
      const canClear = k.set === true && k.source === 'dotenv';
      const marked = clearing.has(k.name);
      const readOnly = k.console_writable === false;
      const choices = !readOnly && Array.isArray(k.choices) && k.choices.length > 0;
      const placeholder = readOnly ? (k.set ? 'set — read-only in the console' : 'not set — read-only in the console')
        : (k.set ? 'set — type to replace' : 'not set');
      // Secrets are masked text inputs, not type=password: Chrome offers (and can insert) a generated
      // password or a saved one into password fields even with autocomplete=new-password, and saving
      // that would write it to .env. The data-* flags keep 1Password/LastPass/Dashlane out as well.
      const secret = k.secret !== false;
      const input = choices ? choiceSelect(k, { id, hintId, marked }) : h('input', {
        id, type: 'text', name: `aegis-${k.name.toLowerCase()}`, class: secret ? 'key__secret' : undefined,
        dataset: { key: k.name, ...(readOnly ? { readonly: 'true' } : {}), ...(secret ? { '1pIgnore': 'true', lpignore: 'true', formType: 'other' } : {}) },
        autocomplete: 'off', spellcheck: 'false', autocapitalize: 'off',
        'aria-describedby': hintId, placeholder, readonly: readOnly, disabled: marked || readOnly,
      });
      if (typed.has(k.name) && !readOnly) input.value = typed.get(k.name);
      return h('div', { class: `key ${k.set ? 'key--set' : 'key--unset'}` },
        h('div', { class: 'key__head' },
          h('label', { for: id, class: 'mono key__name', text: k.name }),
          k.set === true ? badge('set', 'ok') : badge('unset', 'skip'),
          h('span', { class: 'small muted', text: `source: ${fmtText(k.source)}` }),
          h('span', { class: 'small muted', text: k.secret === false ? 'public' : 'secret · write-only' }),
          readOnly ? badge('read-only', 'neutral', 'The daemon does not accept this key over HTTP') : null),
        h('p', { class: 'key__hint small', id: hintId },
          fmtText(k.hint),
          tierHint(config, k.name),
          choices ? ' Only these providers can be chosen here; switching provider needs that provider’s key in NEBIUS_API_KEY in the same save.' : '',
          k.source === 'env' ? ' (Set in the daemon’s environment, which takes precedence over the .env file.)' : '',
          readOnly ? ` Read-only in the console: set it with \`aegis-daemon config set ${k.name}\` (harness/configure_keys.sh) or the environment.${canClear ? ' “Clear on save” restores the default.' : ''}` : ''),
        h('div', { class: 'key__row' },
          input,
          canClear ? h('button', {
            type: 'button', class: 'btn btn--small', 'aria-pressed': String(marked), dataset: { clear: k.name },
            onclick: () => { if (clearing.has(k.name)) clearing.delete(k.name); else clearing.add(k.name); renderKeys(); },
          }, marked ? 'Will clear on save' : 'Clear on save') : null,
          k.signup_url ? h('a', { href: k.signup_url, target: '_blank', rel: 'noopener noreferrer', class: 'link small' },
            'Get a key', h('span', { 'aria-hidden': 'true', text: ' ↗' }), h('span', { class: 'sr-only', text: ' (opens in a new tab)' })) : null),
        choices ? providerNotes() : null);
    }));
    if (active) {
      const again = els.keys.querySelector(active);
      if (again && !again.disabled) again.focus();
    }
  }

  function serviceState(svc) {
    if (!svc) return badge('unknown', 'neutral');
    return svc.enabled === true ? badge('enabled', 'ok') : badge('disabled', 'skip');
  }

  function renderServices() {
    const services = (config && config.services) || {};
    const names = Object.keys(services);
    if (!names.length) {
      replace(els.services, h('p', { class: 'muted', text: 'No service status reported.' }));
      return;
    }
    replace(els.services, names.map((name) => {
      const svc = services[name] || {};
      const extra = Object.entries(svc).filter(([k]) => !['enabled', 'reason'].includes(k));
      const result = testResults.get(name);
      return h('div', { class: 'service' },
        h('div', { class: 'service__head' },
          h('strong', { text: SERVICE_LABELS[name] || name }),
          serviceState(svc),
          TESTABLE.includes(name) ? h('button', {
            type: 'button', class: 'btn btn--small', disabled: testing.has(name), dataset: { test: name },
            onclick: () => runTest(name),
          }, testing.has(name) ? 'Testing…' : `Test ${SERVICE_LABELS[name] || name}`) : null),
        svc.reason ? h('p', { class: 'small reason', text: svc.reason }) : null,
        extra.length ? h('dl', { class: 'kv kv--compact' }, extra.map(([k, v]) => h('div', null, h('dt', { text: k }), h('dd', { class: 'mono break', text: fmtText(v) })))) : null,
        result ? h('p', { class: 'test-result small', role: 'status' },
          result.error ? badge('error', 'fail') : (result.ok === true ? badge('OK', 'ok') : badge('FAILED', 'fail')),
          result.error ? ` ${result.error}` : ` latency ${fmtMs(result.latency_ms)} · ${fmtText(result.detail)}`) : null);
    }));
  }

  async function runTest(name) {
    testing.add(name);
    renderServices();
    try {
      const res = await post('/api/config/test', { service: name });
      testResults.set(name, res || { ok: false, detail: DASH });
    } catch (err) {
      testResults.set(name, { error: describeError(err) });
    } finally {
      testing.delete(name);
      renderServices();
      const again = els.services.querySelector(`[data-test="${name}"]`);
      if (again && (!document.activeElement || document.activeElement === document.body || !dialog.contains(document.activeElement))) again.focus();
    }
  }

  els.form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const values = {};
    const blank = [];
    for (const input of inputs()) {
      if (input.value === '' || input.dataset.readonly === 'true' || clearing.has(input.dataset.key)) continue;
      // A selector's "Default" choice unsets the key, exactly like "Clear on save".
      if (input.dataset.choice === 'true' && input.value === DEFAULT_CHOICE) { values[input.dataset.key] = ''; continue; }
      // The daemon refuses a whitespace-only value (400; only "" unsets); say so before posting.
      // Unsetting goes only through "Clear on save".
      if (input.value.trim() === '') blank.push(input);
      else values[input.dataset.key] = input.value;
    }
    if (blank.length) {
      els.status.textContent = `Nothing saved: ${blank.map((i) => i.dataset.key).join(', ')} ${blank.length === 1 ? 'contains' : 'contain'} only spaces. Type the value, or use “Clear on save” to remove a key stored in the .env file.`;
      blank[0].focus();
      return;
    }
    for (const name of clearing) values[name] = '';
    const names = Object.keys(values);
    if (!names.length) {
      els.status.textContent = 'Nothing to save: type a value, or mark a key to clear.';
      return;
    }
    if (Object.values(values).some((v) => /[\r\n]/.test(v))) {
      els.status.textContent = 'Values cannot contain line breaks.';
      return;
    }
    els.save.disabled = true;
    els.status.textContent = 'Saving…';
    try {
      const res = await post('/api/config', { values });
      for (const input of inputs()) input.value = '';
      clearing.clear();
      setConfig(res);
      els.status.textContent = `Saved ${names.length} key${names.length === 1 ? '' : 's'}: ${names.join(', ')}.`;
    } catch (err) {
      els.status.textContent = `Save failed: ${describeError(err)}`;
    } finally {
      els.save.disabled = false;
    }
  });

  function setConfig(cfg) {
    config = cfg || null;
    els.error.hidden = true;
    renderKeys();
    renderServices();
    if (onConfig) onConfig(config);
  }

  return {
    setConfig,
    fail(err) {
      els.error.hidden = false;
      els.error.textContent = `GET /api/config: ${describeError(err)}`;
      if (!config) replace(els.keys, h('p', { class: 'muted', text: 'Configuration unavailable until the daemon answers.' }));
    },
    onMessage(msg) {
      if (msg.type === 'config.changed' && msg.config) setConfig(msg.config);
    },
  };
}
