// Unified-diff viewer with a tiny syntax highlighter (Rust, TOML, Markdown headings, plain text).
// The daemon sends the diff text already redacted and size-capped (API.md "UnifiedDiff"); this
// module only parses and draws it, as DOM text nodes (never innerHTML). `[REDACTED]` is drawn as
// a pill wherever it appears.

import { h, fmtNum, isNum } from './dom.js';

const REDACTED = '[REDACTED]';

const RUST_KW = new Set(('as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move '
  + 'mut pub ref return self Self static struct super trait true type unsafe use where while').split(' '));
const RUST_TY = new Set('u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f32 f64 bool char str String Vec Option Result Box Some None Ok Err'.split(' '));

const NUM_RE = /^(?:0x[0-9a-fA-F_]+|0b[01_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64)?/;
const IDENT_RE = /^[A-Za-z_][A-Za-z0-9_]*/;
const CHAR_RE = /^'(?:\\.|[^\\'])'/;
const LIFE_RE = /^'[A-Za-z_][A-Za-z0-9_]*/;

function readString(src, i, quote) {
  let j = i + 1;
  while (j < src.length) {
    if (src[j] === '\\') j += 2;
    else if (src[j] === quote) return j + 1;
    else j += 1;
  }
  return src.length;
}

/** Tokens of one line of Rust: [{t, v}] with t in kw|str|com|num|ty|mac|fn|attr|life|punct|id|ws|red. */
export function tokenizeRust(src) {
  const out = [];
  let i = 0;
  const push = (t, v) => { if (v) out.push({ t, v }); };
  while (i < src.length) {
    const rest = src.slice(i);
    const c = src[i];
    if (rest.startsWith(REDACTED)) { push('red', REDACTED); i += REDACTED.length; continue; }
    if (rest.startsWith('//')) { push('com', rest); break; }
    if (rest.startsWith('/*')) {
      const end = rest.indexOf('*/', 2);
      const v = end < 0 ? rest : rest.slice(0, end + 2);
      push('com', v); i += v.length; continue;
    }
    if (rest.startsWith('#[') || rest.startsWith('#![')) {
      const end = rest.indexOf(']');
      const v = end < 0 ? rest : rest.slice(0, end + 1);
      push('attr', v); i += v.length; continue;
    }
    if (c === '"' || (c === 'r' && (src[i + 1] === '"' || (src[i + 1] === '#' && src[i + 2] === '"'))) || (c === 'b' && src[i + 1] === '"')) {
      const start = c === '"' ? i : src.indexOf('"', i);
      const end = readString(src, start, '"');
      pushString(out, src.slice(i, end));
      i = end; continue;
    }
    if (c === "'") {
      const ch = CHAR_RE.exec(rest);
      if (ch) { push('str', ch[0]); i += ch[0].length; continue; }
      const life = LIFE_RE.exec(rest);
      if (life) { push('life', life[0]); i += life[0].length; continue; }
    }
    if (/\d/.test(c)) {
      const m = NUM_RE.exec(rest);
      if (m) { push('num', m[0]); i += m[0].length; continue; }
    }
    if (/[A-Za-z_]/.test(c)) {
      const word = IDENT_RE.exec(rest)[0];
      const next = src[i + word.length];
      if (next === '!' && src[i + word.length + 1] !== '=') { push('mac', `${word}!`); i += word.length + 1; continue; }
      if (RUST_KW.has(word)) push('kw', word);
      else if (RUST_TY.has(word) || /^[A-Z]/.test(word)) push('ty', word);
      else if (next === '(') push('fn', word);
      else push('id', word);
      i += word.length; continue;
    }
    if (/\s/.test(c)) {
      const m = /^\s+/.exec(rest)[0];
      push('ws', m); i += m.length; continue;
    }
    push('punct', c);
    i += 1;
  }
  return out;
}

/** A string literal, with any [REDACTED] inside it split out as its own token. */
function pushString(out, v) {
  const parts = v.split(REDACTED);
  parts.forEach((p, k) => {
    if (p) out.push({ t: 'str', v: p });
    if (k < parts.length - 1) out.push({ t: 'red', v: REDACTED });
  });
}

/** Tokens of one line of TOML: comments, [sections], keys, strings, numbers, booleans. */
export function tokenizeToml(src) {
  const out = [];
  const lead = /^\s*/.exec(src)[0];
  if (lead) out.push({ t: 'ws', v: lead });
  let rest = src.slice(lead.length);
  if (rest.startsWith('#')) { out.push({ t: 'com', v: rest }); return out; }
  if (rest.startsWith('[') && !rest.startsWith(REDACTED)) { out.push({ t: 'head', v: rest }); return out; }
  const key = /^([A-Za-z0-9_.-]+)(\s*=\s*)/.exec(rest);
  if (key) {
    out.push({ t: 'key', v: key[1] }, { t: 'punct', v: key[2] });
    rest = rest.slice(key[0].length);
  }
  let i = 0;
  while (i < rest.length) {
    const tail = rest.slice(i);
    if (tail.startsWith(REDACTED)) { out.push({ t: 'red', v: REDACTED }); i += REDACTED.length; continue; }
    const c = rest[i];
    if (c === '"' || c === "'") {
      const end = readString(rest, i, c);
      pushString(out, rest.slice(i, end));
      i = end; continue;
    }
    if (c === '#') { out.push({ t: 'com', v: tail }); break; }
    const num = /^-?\d[\d_.]*/.exec(tail);
    if (num && (i === 0 || /[\s=[,]/.test(rest[i - 1]))) { out.push({ t: 'num', v: num[0] }); i += num[0].length; continue; }
    const kw = /^(true|false)\b/.exec(tail);
    if (kw) { out.push({ t: 'kw', v: kw[0] }); i += kw[0].length; continue; }
    out.push({ t: 'punct', v: c });
    i += 1;
  }
  return mergePlain(out);
}

/** Plain text: only [REDACTED] is marked. */
export function tokenizePlain(src) {
  const out = [];
  const parts = String(src).split(REDACTED);
  parts.forEach((p, k) => {
    if (p) out.push({ t: 'id', v: p });
    if (k < parts.length - 1) out.push({ t: 'red', v: REDACTED });
  });
  return out;
}

function tokenizeMarkdown(src) {
  if (/^\s*#{1,6}\s/.test(src)) return [{ t: 'head', v: src }];
  return tokenizePlain(src);
}

function mergePlain(tokens) {
  const out = [];
  for (const tk of tokens) {
    const last = out[out.length - 1];
    if (last && last.t === tk.t && (tk.t === 'punct' || tk.t === 'id')) last.v += tk.v;
    else out.push({ ...tk });
  }
  return out;
}

export function languageFor(path) {
  const p = String(path || '').toLowerCase();
  if (p.endsWith('.rs')) return 'rust';
  if (p.endsWith('.toml') || p.endsWith('.cedar')) return p.endsWith('.toml') ? 'toml' : 'rust';
  if (p.endsWith('.md')) return 'markdown';
  return 'plain';
}

export function tokenize(line, lang) {
  if (lang === 'rust') return tokenizeRust(line);
  if (lang === 'toml') return tokenizeToml(line);
  if (lang === 'markdown') return tokenizeMarkdown(line);
  return tokenizePlain(line);
}

/**
 * Parses unified diff text into display rows:
 * {kind: 'file'|'meta'|'hunk'|'add'|'del'|'ctx', text, oldNo, newNo, path}. Files carry +/- counts.
 */
export function parseUnifiedDiff(text) {
  const rows = [];
  const files = [];
  let path = null;
  let file = null;
  let oldNo = 0;
  let newNo = 0;
  const lines = String(text || '').split('\n');
  if (lines.length && lines[lines.length - 1] === '') lines.pop();
  for (const line of lines) {
    if (line.startsWith('diff --git ')) {
      const m = / b\/(.+)$/.exec(line);
      path = m ? m[1] : line.slice(11);
      file = { path, added: 0, removed: 0 };
      files.push(file);
      rows.push({ kind: 'file', text: path, path, file });
      continue;
    }
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      if (oldNo === 0) oldNo = 1;
      if (newNo === 0) newNo = 1;
      rows.push({ kind: 'hunk', text: line, path });
      continue;
    }
    if (line.startsWith('--- ') || line.startsWith('+++ ') || line === 'new file' || line === 'deleted file' || line.startsWith('(')) {
      rows.push({ kind: 'meta', text: line, path });
      continue;
    }
    const sign = line[0];
    const body = line.slice(1);
    if (sign === '+') {
      rows.push({ kind: 'add', text: body, newNo, path });
      newNo += 1;
      if (file) file.added += 1;
    } else if (sign === '-') {
      rows.push({ kind: 'del', text: body, oldNo, path });
      oldNo += 1;
      if (file) file.removed += 1;
    } else {
      rows.push({ kind: 'ctx', text: sign === ' ' ? body : line, oldNo, newNo, path });
      oldNo += 1;
      newNo += 1;
    }
  }
  return { rows, files };
}

function codeNode(text, lang) {
  return h('code', { class: 'dl__code' }, tokenize(text, lang).map((tk) => (tk.t === 'ws' || tk.t === 'id'
    ? tk.v
    : h('span', { class: `tok-${tk.t}`, text: tk.v }))));
}

const SIGN = { add: '+', del: '−', ctx: '', hunk: '', meta: '', file: '' };

/** The rendered lines of a parsed diff (a .difflines element). */
export function renderDiffLines(parsed) {
  const rows = parsed.rows.map((r) => {
    const lang = languageFor(r.path);
    if (r.kind === 'file') {
      const f = r.file || { added: 0, removed: 0 };
      return h('div', { class: 'dl dl--file' }, h('span', { class: 'dl__no' }), h('span', { class: 'dl__no' }), h('span', { class: 'dl__sign', text: '▾' }),
        h('code', { class: 'dl__code' }, r.text, '  ', h('span', { class: 'diffview__add', text: `+${f.added}` }), ' ', h('span', { class: 'diffview__del', text: `−${f.removed}` })));
    }
    const oldNo = r.kind === 'del' || r.kind === 'ctx' ? String(r.oldNo) : '';
    const newNo = r.kind === 'add' || r.kind === 'ctx' ? String(r.newNo) : '';
    const code = r.kind === 'hunk' || r.kind === 'meta' ? h('code', { class: 'dl__code', text: r.text }) : codeNode(r.text, lang);
    return h('div', { class: `dl dl--${r.kind}` },
      h('span', { class: 'dl__no', text: oldNo }), h('span', { class: 'dl__no', text: newNo }),
      h('span', { class: 'dl__sign', 'aria-hidden': 'true', text: SIGN[r.kind] }),
      r.kind === 'add' ? h('span', { class: 'sr-only', text: 'added: ' }) : r.kind === 'del' ? h('span', { class: 'sr-only', text: 'removed: ' }) : null,
      code);
  });
  return h('div', { class: 'difflines' }, rows);
}

const kib = (n) => (isNum(n) ? `${fmtNum(Math.round((n / 1024) * 10) / 10)} KiB` : '?');

/** One-line description of the daemon's cap and redaction for a UnifiedDiff. */
export function capNote(view) {
  if (!view) return '';
  const parts = [];
  if (view.truncated) {
    parts.push(`showing ${kib(view.bytes)} of ${kib(view.bytes_total)}: cut at the daemon's ${kib(view.cap_bytes)} cap`);
    if (isNum(view.files_total) && isNum(view.files) && view.files < view.files_total) parts.push(`${fmtNum(view.files)} of ${fmtNum(view.files_total)} files`);
  }
  if (isNum(view.redactions) && view.redactions > 0) parts.push(`${fmtNum(view.redactions)} value${view.redactions === 1 ? '' : 's'} redacted`);
  return parts.join(' · ');
}

/**
 * A collapsible diff block. `view` is a UnifiedDiff from the daemon. The lines are rendered when
 * the block is first opened (or at once when `open`), and `onToggle(open)` reports user toggles.
 */
export function diffBlock({ title, view, open = false, onToggle, extra }) {
  const parsed = parseUnifiedDiff(view && view.text);
  const added = parsed.files.reduce((n, f) => n + f.added, 0);
  const removed = parsed.files.reduce((n, f) => n + f.removed, 0);
  const files = isNum(view && view.files_total) ? view.files_total : parsed.files.length;
  const note = capNote(view);
  const body = h('div', { class: 'diffview__body' });
  let rendered = false;
  const fill = () => {
    if (rendered) return;
    rendered = true;
    body.append(parsed.rows.length ? renderDiffLines(parsed) : h('p', { class: 'diffview--empty', text: 'The diff is empty.' }));
  };
  const details = h('details', { class: 'diffview', open: open || null, dataset: { role: 'diff' } },
    h('summary', null,
      h('span', { class: 'diffview__chev', 'aria-hidden': 'true', text: '▸' }),
      h('span', { class: 'diffview__title', text: title }),
      h('span', { class: 'diffview__stat' }, `${fmtNum(files)} file${files === 1 ? '' : 's'} `,
        h('span', { class: 'diffview__add', text: `+${fmtNum(added)}` }), ' ', h('span', { class: 'diffview__del', text: `−${fmtNum(removed)}` })),
      extra || null,
      note ? h('span', { class: 'diffview__cap', text: note }) : null),
    body,
    view && view.truncated ? h('p', { class: 'diffview__note', text: `Truncated by the daemon: ${note}.` }) : null);
  if (open) fill();
  details.addEventListener('toggle', () => {
    const isOpen = typeof details.open === 'boolean' ? details.open : details.hasAttribute('open');
    if (isOpen) fill();
    if (onToggle) onToggle(isOpen);
  });
  return details;
}
