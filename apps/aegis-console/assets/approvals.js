// Panel 3 (left column): pending approval cards with Approve / Reject, and resolved history.

import { h, replace, badge, riskBar, chips, kvList, fmtMs, fmtNum, fmtText, fmtDateTime, DASH, storageGet, storageSet } from './dom.js';
import { post, describeError } from './api.js';

const MAX_RESOLVED = 50;
export const APPROVE_LABEL = 'Approve · RENAME_SWAP';
export const REJECT_LABEL = 'Reject · rollback';
const APPROVER_KEY = 'aegis.console.approver';

function cardStatus(value) {
  if (value === 'sent') return badge('sent', 'ok');
  if (value === 'disabled') return badge('disabled', 'skip');
  if (typeof value === 'string' && value.startsWith('error')) return badge(value, 'fail');
  return badge(fmtText(value), 'neutral');
}

function resultText(result) {
  if (!result) return DASH;
  return `${fmtText(result.action)} in ${fmtMs(result.ms)}`;
}

export function createApprovalsPanel() {
  const root = document.getElementById('panel-approvals');
  const $ = (role) => root.querySelector(`[data-role="${role}"]`);
  const els = { error: $('error'), pending: $('pending'), resolved: $('resolved'), count: $('pending-count'), status: $('action-status'), approver: $('approver') };

  const pending = new Map();
  let resolved = [];
  const inflight = new Map(); // id -> 'approve' | 'reject'
  const cardErrors = new Map();
  let loaded = false;

  els.pending.tabIndex = -1;
  els.approver.value = storageGet(APPROVER_KEY) || 'console';
  els.approver.addEventListener('change', () => storageSet(APPROVER_KEY, els.approver.value.trim()));

  function showError(msg) {
    els.error.hidden = !msg;
    els.error.textContent = msg || '';
  }

  // Re-rendering replaces the cards, so keyboard focus is carried over by data-focus-key:
  // the same button if it is still enabled, else its card, else the list itself.
  function renderPending() {
    const active = document.activeElement;
    const key = active && els.pending.contains(active) && active.dataset ? active.dataset.focusKey : null;
    els.count.textContent = loaded ? `(${pending.size})` : '';
    if (!pending.size) {
      replace(els.pending, h('p', { class: 'muted', text: loaded ? 'No approvals waiting.' : 'Loading…' }));
    } else {
      const list = [...pending.values()].sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)));
      replace(els.pending, list.map(renderCard));
    }
    if (key) {
      const find = (k) => els.pending.querySelector(`[data-focus-key="${CSS.escape(k)}"]`);
      const target = find(key);
      const card = find(key.replace(/:(approve|reject)$/, ':card'));
      if (target && !target.disabled) target.focus();
      else if (card) card.focus();
      else els.pending.focus();
    }
  }

  function renderCard(a) {
    const action = inflight.get(a.id);
    const titleId = `appr-${a.id}-title`;
    const cards = a.cards && typeof a.cards === 'object' ? Object.entries(a.cards) : [];
    const err = cardErrors.get(a.id);
    return h('article', { class: 'approval', 'aria-labelledby': titleId, 'aria-busy': action ? 'true' : null, tabindex: '-1', dataset: { focusKey: `${a.id}:card` } },
      h('header', { class: 'approval__head' },
        h('h4', { id: titleId, text: fmtText(a.title) }),
        badge(fmtText(a.kind), a.kind === 'paypal' ? 'info' : 'neutral'),
        h('span', { class: 'mono small muted', text: [a.id, a.run_id].filter(Boolean).join(' · ') })),
      a.summary ? h('p', { class: 'approval__summary', text: a.summary }) : null,
      kvList([
        ['Risk', riskBar(a.risk)],
        ['Files changed', fmtNum(a.files_changed)],
        ['Determining policies', chips(a.determining_policies)],
        ['Requested', fmtDateTime(a.created_at)],
        ['Card delivery', cards.length ? h('span', { class: 'cards' }, cards.map(([k, v]) => h('span', { class: 'card-status' }, h('span', { class: 'muted', text: `${k} ` }), cardStatus(v)))) : DASH],
      ], 'kv kv--card'),
      a.excerpt ? h('details', { class: 'excerpt', open: true },
        h('summary', { text: 'Redacted excerpt' }),
        h('pre', { class: 'mono', text: a.excerpt })) : null,
      h('div', { class: 'approval__actions' },
        h('button', {
          type: 'button', class: 'btn btn--ok', disabled: Boolean(action), dataset: { focusKey: `${a.id}:approve` }, onclick: () => resolve(a, 'approve'),
          'aria-label': `Approve ${fmtText(a.title)}: Cedar re-checks with team approval, then RENAME_SWAP commit`,
        }, action === 'approve' ? 'Approving…' : APPROVE_LABEL),
        h('button', {
          type: 'button', class: 'btn btn--danger', disabled: Boolean(action), dataset: { focusKey: `${a.id}:reject` }, onclick: () => resolve(a, 'reject'),
          'aria-label': `Reject ${fmtText(a.title)}: roll the branch back`,
        }, action === 'reject' ? 'Rejecting…' : REJECT_LABEL),
        err ? h('span', { class: 'error-text small', role: 'alert', text: err }) : null));
  }

  function renderResolved() {
    if (!resolved.length) {
      replace(els.resolved, h('li', { class: 'muted', text: loaded ? 'Nothing resolved yet.' : '' }));
      return;
    }
    replace(els.resolved, resolved.map((a) => h('li', { class: `resolved__item resolved__item--${a.status}` },
      a.status === 'approved' ? badge('approved', 'ok') : a.status === 'rejected' ? badge('rejected', 'fail') : badge(fmtText(a.status), 'neutral'),
      h('span', { class: 'resolved__title', text: fmtText(a.title) }),
      h('span', { class: 'mono small', text: resultText(a.result) }),
      h('span', { class: 'muted small', text: `${fmtText(a.resolved_by)} · ${fmtDateTime(a.resolved_at)}` }))));
  }

  function upsert(a) {
    if (!a || !a.id) return;
    if (a.status === 'pending') {
      pending.set(a.id, a);
    } else {
      pending.delete(a.id);
      inflight.delete(a.id);
      cardErrors.delete(a.id);
      resolved = [a, ...resolved.filter((r) => r.id !== a.id)].slice(0, MAX_RESOLVED);
    }
  }

  async function resolve(a, action) {
    const by = els.approver.value.trim();
    if (!by) {
      els.status.textContent = 'Enter an approver name first.';
      els.approver.focus();
      return;
    }
    storageSet(APPROVER_KEY, by);
    inflight.set(a.id, action);
    cardErrors.delete(a.id);
    renderPending();
    els.status.textContent = `${action === 'approve' ? 'Approving' : 'Rejecting'} ${a.id}…`;
    try {
      const res = await post(`/api/approvals/${encodeURIComponent(a.id)}/${action}`, { by });
      upsert(res);
      els.status.textContent = `${a.id} ${fmtText(res && res.status)} → ${resultText(res && res.result)}`;
    } catch (err) {
      inflight.delete(a.id);
      cardErrors.set(a.id, `${action} failed: ${describeError(err)}`);
      els.status.textContent = `${action} ${a.id} failed: ${describeError(err)}`;
    }
    renderPending();
    renderResolved();
  }

  return {
    setApprovals(data) {
      loaded = true;
      showError('');
      pending.clear();
      for (const a of (data && data.pending) || []) upsert(a);
      resolved = ((data && data.resolved) || []).slice(0, MAX_RESOLVED);
      for (const id of [...inflight.keys()]) if (!pending.has(id)) inflight.delete(id);
      renderPending();
      renderResolved();
    },
    onMessage(msg) {
      if ((msg.type === 'approval.created' || msg.type === 'approval.resolved') && msg.approval) {
        upsert(msg.approval);
        renderPending();
        renderResolved();
      }
    },
    fail(err) {
      showError(`GET /api/approvals: ${describeError(err)}`);
      if (!loaded) replace(els.pending, h('p', { class: 'muted', text: 'Approvals unavailable until the daemon answers.' }));
    },
  };
}
