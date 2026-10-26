// "Inspect signed receipt JSON" popover for Panel 2. Reads GET /api/paypal/orders/{id}/receipt
// (API.md): the Orders v2 body as the middleware signed it, redacted for display, plus the detached
// c2pa:ed25519 receipt. Rendered with a tiny JSON highlighter as DOM text nodes; [REDACTED] values
// and the signature are drawn as pills.

import { h, replace, fmtText, copyText } from './dom.js';
import { get, describeError } from './api.js';

const REDACTED = '[REDACTED]';
const JSON_RE = /("(?:\\.|[^"\\])*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b|(\s+)|([\s\S])/g;

/** Tokens of pretty-printed JSON: key | str | sig | red | num | kw | punct | ws. */
export function tokenizeJson(text) {
  const out = [];
  for (const m of String(text).matchAll(JSON_RE)) {
    if (m[1] !== undefined) {
      if (m[2] !== undefined) {
        out.push({ t: 'key', v: m[1] }, { t: 'punct', v: m[2] });
      } else if (m[1].startsWith('"c2pa:')) {
        out.push({ t: 'sig', v: m[1] });
      } else if (m[1].includes(REDACTED)) {
        m[1].split(REDACTED).forEach((p, k, all) => {
          if (p) out.push({ t: 'str', v: p });
          if (k < all.length - 1) out.push({ t: 'red', v: REDACTED });
        });
      } else {
        out.push({ t: 'str', v: m[1] });
      }
    } else if (m[3] !== undefined) out.push({ t: 'num', v: m[3] });
    else if (m[4] !== undefined) out.push({ t: 'kw', v: m[4] });
    else if (m[5] !== undefined) out.push({ t: 'ws', v: m[5] });
    else out.push({ t: 'punct', v: m[6] });
  }
  return out;
}

export function renderJson(value) {
  const text = JSON.stringify(value, null, 2);
  return tokenizeJson(text).map((tk) => (tk.t === 'ws' || tk.t === 'punct' ? tk.v : h('span', { class: `tok-${tk.t}`, text: tk.v })));
}

export function createReceiptViewer() {
  const dialog = document.getElementById('receipt-dialog');
  if (!dialog) return { open: async () => null };
  const $ = (role) => dialog.querySelector(`[data-role="${role}"]`);
  const els = { title: dialog.querySelector('h2'), meta: $('meta'), status: $('status'), body: $('body'), copy: $('copy'), close: $('close') };
  let current = null;
  let opener = null;
  let seq = 0;

  function close() {
    if (typeof dialog.close === 'function') dialog.close();
    else dialog.removeAttribute('open');
  }
  els.close.addEventListener('click', close);
  dialog.addEventListener('close', () => { if (opener && opener.isConnected) opener.focus(); });
  dialog.addEventListener('click', (ev) => { if (ev.target === dialog) close(); });
  els.copy.addEventListener('click', async () => {
    if (!current) return;
    const ok = await copyText(JSON.stringify(current, null, 2));
    els.status.textContent = ok ? 'Receipt JSON copied to the clipboard.' : 'Copy failed: the browser refused clipboard access.';
  });

  async function open(orderId, label, trigger) {
    opener = trigger || null;
    const mine = ++seq;
    current = null;
    els.title.textContent = `Signed receipt · ${fmtText(label || orderId)}`;
    els.meta.textContent = '';
    els.status.textContent = `GET /api/paypal/orders/${orderId}/receipt…`;
    replace(els.body);
    if (typeof dialog.showModal === 'function') { if (!dialog.open) dialog.showModal(); } else dialog.setAttribute('open', '');
    els.close.focus();
    try {
      const doc = await get(`/api/paypal/orders/${encodeURIComponent(orderId)}/receipt`);
      if (mine !== seq) return doc;
      current = doc;
      const r = (doc && doc.receipt) || {};
      els.meta.textContent = [
        `${fmtText(r.scheme)} · ${r.verified === true ? 'signature verified' : r.verified === false ? 'signature NOT verified' : 'verification unknown'}`,
        `payload sha256 ${fmtText(r.payload_sha256)}`,
        `${fmtText(doc && doc.redactions)} field(s) redacted before signing${doc && doc.display_redactions ? `; ${doc.display_redactions} more masked here for display (the scenario disabled the middleware's redaction)` : ''}`,
      ].join(' · ');
      els.status.textContent = '';
      replace(els.body, renderJson(doc));
      return doc;
    } catch (err) {
      if (mine === seq) els.status.textContent = `Could not load the receipt: ${describeError(err)}`;
      return null;
    }
  }

  return { open };
}
