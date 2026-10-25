// Minimal DOM for running the console's panel modules under `node --test` (development only; the
// daemon never serves dev/). It implements just what assets/*.js touch: element creation, tree
// edits, attributes/dataset, textContent, compound selectors (tag, .class, [attr], [attr="v"])
// joined by the descendant combinator, listeners and dispatch. It is not a browser; real-browser
// behaviour is checked against the daemon.

const camel = (s) => s.replace(/-([a-z])/g, (_, c) => c.toUpperCase());

export class FakeEvent {
  constructor(type, init = {}) {
    this.type = type;
    this.defaultPrevented = false;
    this.target = null;
    this.currentTarget = null;
    Object.assign(this, init);
  }

  preventDefault() {
    this.defaultPrevented = true;
  }
}

class FakeNode {
  constructor(doc) {
    this.ownerDocument = doc;
    this.parentNode = null;
    this.childNodes = [];
  }

  get firstChild() { return this.childNodes[0] || null; }
  get lastChild() { return this.childNodes[this.childNodes.length - 1] || null; }

  get nextSibling() {
    if (!this.parentNode) return null;
    const sib = this.parentNode.childNodes;
    return sib[sib.indexOf(this) + 1] || null;
  }

  get isConnected() {
    let n = this;
    while (n.parentNode) n = n.parentNode;
    return n === this.ownerDocument;
  }

  insertBefore(child, ref) {
    if (child.parentNode) child.parentNode.removeChild(child);
    const i = ref ? this.childNodes.indexOf(ref) : -1;
    if (i < 0) this.childNodes.push(child);
    else this.childNodes.splice(i, 0, child);
    child.parentNode = this;
    return child;
  }

  appendChild(child) { return this.insertBefore(child, null); }

  removeChild(child) {
    const i = this.childNodes.indexOf(child);
    if (i >= 0) this.childNodes.splice(i, 1);
    child.parentNode = null;
    return child;
  }

  append(...nodes) {
    for (const n of nodes) this.appendChild(typeof n === 'string' ? this.ownerDocument.createTextNode(n) : n);
  }

  prepend(...nodes) {
    const first = this.firstChild;
    for (const n of nodes) this.insertBefore(typeof n === 'string' ? this.ownerDocument.createTextNode(n) : n, first);
  }

  remove() { if (this.parentNode) this.parentNode.removeChild(this); }

  replaceWith(node) {
    const p = this.parentNode;
    if (!p) return;
    p.insertBefore(node, this);
    p.removeChild(this);
  }

  contains(node) {
    for (let n = node; n; n = n.parentNode) if (n === this) return true;
    return false;
  }

  get textContent() { return this.childNodes.map((c) => c.textContent).join(''); }

  set textContent(v) {
    for (const c of [...this.childNodes]) this.removeChild(c);
    if (v !== '' && v !== null && v !== undefined) this.appendChild(this.ownerDocument.createTextNode(String(v)));
  }
}

class FakeText extends FakeNode {
  constructor(doc, data) {
    super(doc);
    this.nodeType = 3;
    this.data = String(data);
  }

  get textContent() { return this.data; }
  set textContent(v) { this.data = String(v); }
}

function parseSelector(sel) {
  const m = /^([a-zA-Z][\w-]*)?((?:\.[\w-]+|\[[\w-]+(?:="(?:[^"\\]|\\.)*")?\])*)$/.exec(sel.trim());
  if (!m) throw new Error(`fake-dom: unsupported selector ${sel}`);
  const parts = { tag: m[1] ? m[1].toLowerCase() : null, classes: [], attrs: [] };
  for (const [, cls] of m[2].matchAll(/\.([\w-]+)/g)) parts.classes.push(cls);
  for (const [, name, value] of m[2].matchAll(/\[([\w-]+)(?:="((?:[^"\\]|\\.)*)")?\]/g)) {
    parts.attrs.push([name, value === undefined ? null : value.replace(/\\(.)/g, '$1')]);
  }
  return parts;
}

class FakeElement extends FakeNode {
  constructor(doc, tag) {
    super(doc);
    this.nodeType = 1;
    this.localName = tag.toLowerCase();
    this.tagName = tag.toUpperCase();
    this.attrs = new Map();
    this.dataset = {};
    this.listeners = new Map();
    this.style = {};
    this.hidden = false;
    this.disabled = false;
    this.value = '';
  }

  setAttribute(name, value) {
    const v = String(value);
    if (name.startsWith('data-')) { this.dataset[camel(name.slice(5))] = v; return; }
    this.attrs.set(name, v);
    if (name === 'hidden') this.hidden = true;
    if (name === 'disabled') this.disabled = true;
  }

  getAttribute(name) {
    if (name.startsWith('data-')) {
      const v = this.dataset[camel(name.slice(5))];
      return v === undefined ? null : String(v);
    }
    if (name === 'hidden') return this.hidden ? '' : null;
    if (name === 'disabled') return this.disabled ? '' : null;
    return this.attrs.has(name) ? this.attrs.get(name) : null;
  }

  hasAttribute(name) { return this.getAttribute(name) !== null; }

  removeAttribute(name) {
    if (name.startsWith('data-')) delete this.dataset[camel(name.slice(5))];
    else this.attrs.delete(name);
    if (name === 'hidden') this.hidden = false;
    if (name === 'disabled') this.disabled = false;
  }

  get id() { return this.getAttribute('id') || ''; }
  get className() { return this.getAttribute('class') || ''; }

  get classList() {
    const el = this;
    const list = () => el.className.split(/\s+/).filter(Boolean);
    const write = (items) => el.setAttribute('class', items.join(' '));
    return {
      contains: (c) => list().includes(c),
      add: (...cs) => write([...new Set([...list(), ...cs])]),
      remove: (...cs) => write(list().filter((c) => !cs.includes(c))),
      toggle(c, force) {
        const on = force === undefined ? !list().includes(c) : Boolean(force);
        if (on) this.add(c); else this.remove(c);
        return on;
      },
    };
  }

  get children() { return this.childNodes.filter((c) => c instanceof FakeElement); }
  get firstElementChild() { return this.children[0] || null; }
  get lastElementChild() { const c = this.children; return c[c.length - 1] || null; }
  get tBodies() { return this.children.filter((c) => c.localName === 'tbody'); }
  get clientWidth() { return 0; }

  addEventListener(type, fn) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(fn);
  }

  removeEventListener(type, fn) {
    const list = this.listeners.get(type) || [];
    const i = list.indexOf(fn);
    if (i >= 0) list.splice(i, 1);
  }

  dispatchEvent(ev) {
    if (!ev.target) ev.target = this;
    ev.currentTarget = this;
    for (const fn of [...(this.listeners.get(ev.type) || [])]) fn(ev);
    return !ev.defaultPrevented;
  }

  click() { this.dispatchEvent(new FakeEvent('click')); }
  focus() { this.ownerDocument.activeElement = this; }

  // Compound selectors joined by the descendant combinator (whitespace).
  matches(sel) {
    const chain = sel.trim().split(/\s+/);
    if (!this.matchesCompound(chain[chain.length - 1])) return false;
    let i = chain.length - 2;
    for (let n = this.parentNode; n && i >= 0; n = n.parentNode) {
      if (n instanceof FakeElement && n.matchesCompound(chain[i])) i -= 1;
    }
    return i < 0;
  }

  matchesCompound(sel) {
    const p = parseSelector(sel);
    if (p.tag && p.tag !== this.localName) return false;
    const classes = this.className.split(/\s+/);
    if (!p.classes.every((c) => classes.includes(c))) return false;
    return p.attrs.every(([name, value]) => {
      const actual = this.getAttribute(name);
      return value === null ? actual !== null : actual === value;
    });
  }

  querySelectorAll(sel) {
    const out = [];
    const walk = (node) => {
      for (const c of node.children) {
        if (c.matches(sel)) out.push(c);
        walk(c);
      }
    };
    walk(this);
    return out;
  }

  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}

class FakeDocument extends FakeNode {
  constructor() {
    super(null);
    this.ownerDocument = this;
    this.documentElement = new FakeElement(this, 'html');
    this.body = new FakeElement(this, 'body');
    this.appendChild(this.documentElement);
    this.documentElement.appendChild(this.body);
    this.activeElement = this.body;
  }

  createElement(tag) { return new FakeElement(this, tag); }
  createElementNS(_ns, tag) { return new FakeElement(this, tag); }
  createTextNode(data) { return new FakeText(this, data); }
  getElementById(id) { return this.documentElement.querySelector(`[id="${id}"]`); }
  querySelector(sel) { return this.documentElement.matches(sel) ? this.documentElement : this.documentElement.querySelector(sel); }
  querySelectorAll(sel) { return this.documentElement.querySelectorAll(sel); }
}

/** Installs a fresh fake DOM on globalThis and returns its document. */
export function installFakeDom() {
  const doc = new FakeDocument();
  globalThis.document = doc;
  globalThis.Node = FakeNode;
  globalThis.window = globalThis;
  globalThis.CSS = { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };
  globalThis.getComputedStyle = () => ({ getPropertyValue: () => '' });
  globalThis.matchMedia = () => ({ matches: false });
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
  return doc;
}

/** Builds an element tree for a panel fixture: el('div', {id, 'data-role': 'x'}, ...children). */
export function el(tag, attrs = {}, ...children) {
  const node = globalThis.document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  for (const c of children) node.append(c);
  return node;
}

/** Elements carrying data-role="<role>" for each role, as fixtures for a panel root. */
export function roles(...names) {
  return names.map((name) => {
    const [role, tag] = name.split(':');
    return el(tag || 'div', { 'data-role': role });
  });
}

