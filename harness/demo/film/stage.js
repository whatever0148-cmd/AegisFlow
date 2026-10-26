// The film stage, injected into the live Aegis Console by render_film.mjs.
//
// It moves the console into a macOS-style window on a wallpaper, then draws one frame per call to
// __film.frame(t): a spring camera (zoom + pan + scroll) over the real page, a cursor with click
// ripples, word-synced captions, glass lower-third callouts and the title scenes. Every motion is
// a pure function of t (Remotion style: no CSS transitions in the overlay), and the console's own
// CSS animations are seeked to the virtual clock so they play at video speed. All text the scenes
// show comes from the spec built by render_film.mjs; numbers in it are read from the telemetry
// artifact or the daemon at render time.
(() => {
  const W = 1920, H = 1080, BAR = 40, VIEW_H = H;
  const SANS = '-apple-system, "SF Pro Display", "SF Pro Text", system-ui, sans-serif';
  const MONO = 'ui-monospace, "SF Mono", Menlo, monospace';
  const COLORS = { emerald: '#34d399', cyan: '#67e8f9', amber: '#fbbf24', crimson: '#fb7185', violet: '#c4b5fd', white: '#f8fafc' };

  // ---- easing (Remotion-style springs as closed forms) -------------------------------------
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const lerp = (a, b, p) => a + (b - a) * p;
  // Critically damped step response, normalised to reach exactly 1 at `dur` seconds.
  const crit = (x, dur = 1) => {
    if (x <= 0) return 0;
    if (x >= dur) return 1;
    const k = 6.6, end = 1 - (1 + k) * Math.exp(-k);
    const w = k / dur;
    return (1 - (1 + w * x) * Math.exp(-w * x)) / end;
  };
  // Under-damped spring (slight overshoot) for UI entrances.
  const pop = (x, dur = 0.7, zeta = 0.62) => {
    if (x <= 0) return 0;
    if (x >= dur) return 1;
    const w = 5 / (zeta * dur), wd = w * Math.sqrt(1 - zeta * zeta);
    return 1 - Math.exp(-zeta * w * x) * (Math.cos(wd * x) + (zeta / Math.sqrt(1 - zeta * zeta)) * Math.sin(wd * x));
  };
  const easeInOut = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
  const ramp = (t, a, b) => clamp((t - a) / (b - a), 0, 1);
  // 0 → 1 → 0 envelope: in over `fin`, out over `fout`.
  const env = (t, start, end, fin = 0.35, fout = 0.35) => Math.min(ramp(t, start, start + fin), 1 - ramp(t, end - fout, end));

  const el = (tag, attrs = {}, ...kids) => {
    const n = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k === 'style') n.style.cssText = v;
      else if (k === 'text') n.textContent = v;
      else if (k === 'html') n.innerHTML = v;
      else n.setAttribute(k, v);
    }
    for (const kid of kids) if (kid != null) n.append(kid);
    return n;
  };

  const CSS = `
  html, body { width: ${W}px !important; height: ${H}px !important; overflow: hidden !important; margin: 0 !important; }
  body { background: #04060b !important; }
  #film-wall { position: fixed; inset: 0; z-index: 0; overflow: hidden; background:
      radial-gradient(1100px 700px at 12% 8%, rgba(34, 211, 238, 0.22), transparent 60%),
      radial-gradient(1000px 700px at 92% 96%, rgba(16, 185, 129, 0.18), transparent 60%),
      radial-gradient(900px 600px at 70% 10%, rgba(139, 92, 246, 0.16), transparent 60%),
      linear-gradient(180deg, #060912, #04060b); }
  #film-wall::after { content: ""; position: absolute; inset: 0; background-image: radial-gradient(rgba(255,255,255,0.07) 1px, transparent 1.2px); background-size: 28px 28px; mask-image: radial-gradient(1200px 800px at 50% 45%, #000 30%, transparent 85%); }
  #film-rig { position: fixed; left: 0; top: 0; width: ${W}px; height: ${H}px; z-index: 1; transform-origin: 50% 50%; }
  #film-win { position: absolute; left: 0; top: 0; width: ${W}px; height: ${VIEW_H + BAR}px; transform-origin: 0 0; border-radius: 18px; overflow: hidden;
      box-shadow: 0 0 0 1px rgba(255,255,255,0.14), 0 50px 140px rgba(0,0,0,0.7), 0 20px 50px rgba(0,0,0,0.5); background: #07090e; }
  .film-bar { height: ${BAR}px; display: flex; align-items: center; justify-content: center; position: relative; background: linear-gradient(180deg, #151b27, #0e131c); border-bottom: 1px solid rgba(255,255,255,0.08); font: 600 14px ${SANS}; color: #c4cedc; }
  .film-bar__lights { position: absolute; left: 16px; top: 13px; display: flex; gap: 8px; }
  .film-bar__lights i { width: 13px; height: 13px; border-radius: 50%; display: block; box-shadow: inset 0 0 0 0.5px rgba(0,0,0,0.3); }
  .film-bar__url { font: 500 13px ${MONO}; color: #8d9bb0; margin-left: 10px; }
  #film-view { position: relative; width: ${W}px; height: ${VIEW_H}px; overflow: hidden; color: var(--text); background:
      radial-gradient(1200px 500px at 15% -10%, rgba(6, 182, 212, 0.07), transparent 60%),
      radial-gradient(900px 420px at 95% 0%, rgba(16, 185, 129, 0.05), transparent 60%), var(--canvas); }
  #film-ui[popover] { position: fixed; inset: 0; width: ${W}px; height: ${H}px; max-width: none; max-height: none; margin: 0; padding: 0; border: 0; background: transparent; overflow: visible; pointer-events: none; color: #f8fafc; font-family: ${SANS}; }
  #film-ui * { box-sizing: border-box; }
  .f-abs { position: absolute; }
  .f-scene { position: absolute; inset: 0; overflow: hidden; }
  .f-glass { background: rgba(10, 14, 22, 0.74); -webkit-backdrop-filter: blur(22px) saturate(160%); backdrop-filter: blur(22px) saturate(160%);
      border: 1px solid rgba(255,255,255,0.14); box-shadow: inset 0 1px 0 rgba(255,255,255,0.10), 0 24px 70px rgba(0,0,0,0.55); }
  .f-cap { position: absolute; left: 50%; bottom: 34px; transform: translateX(-50%); max-width: 1560px; padding: 14px 30px 16px; border-radius: 22px; text-align: center;
      font: 650 44px/1.22 ${SANS}; letter-spacing: -0.4px; background: rgba(4, 6, 11, 0.70); -webkit-backdrop-filter: blur(14px); backdrop-filter: blur(14px);
      border: 1px solid rgba(255,255,255,0.10); box-shadow: 0 18px 50px rgba(0,0,0,0.5); white-space: normal; }
  .f-cap span { display: inline-block; margin: 0 0.12em; transition: none; }
  .f-cap .w-past { color: #f8fafc; }
  .f-cap .w-now { color: #67e8f9; text-shadow: 0 0 24px rgba(103, 232, 249, 0.55); }
  .f-cap .w-next { color: rgba(248, 250, 252, 0.42); }
  .f-lower { position: absolute; left: 56px; bottom: 150px; display: flex; gap: 20px; align-items: center; padding: 22px 30px 22px 22px; border-radius: 22px; min-width: 520px; max-width: 900px; overflow: hidden; }
  .f-lower__bar { position: absolute; left: 0; top: 0; bottom: 0; width: 5px; }
  .f-lower__icon { width: 76px; height: 76px; border-radius: 20px; display: grid; place-items: center; font-size: 40px; flex: none; }
  .f-lower__kicker { font: 750 15px ${SANS}; letter-spacing: 0.16em; text-transform: uppercase; }
  .f-lower__title { font: 650 30px/1.18 ${SANS}; color: #f8fafc; margin-top: 4px; letter-spacing: -0.3px; }
  .f-lower__value { font: 750 50px/1.05 ${MONO}; margin-top: 6px; letter-spacing: -1px; }
  .f-lower__sub { font: 500 17px/1.3 ${SANS}; color: #a8b4c6; margin-top: 6px; }
  .f-badge { position: absolute; padding: 9px 16px; border-radius: 999px; font: 650 17px ${SANS}; letter-spacing: 0.02em; display: flex; gap: 9px; align-items: center; white-space: nowrap; }
  .f-dot { width: 10px; height: 10px; border-radius: 50%; display: inline-block; }
  .f-hl { position: absolute; border-radius: 14px; border: 3px solid; }
  .f-spot { position: absolute; border-radius: 18px; border: 1px solid rgba(165, 243, 252, 0.35);
      box-shadow: 0 0 0 4000px rgba(3, 5, 10, 0.62), 0 0 90px 30px rgba(3, 5, 10, 0.55), inset 0 0 40px rgba(103, 232, 249, 0.08), 0 0 50px rgba(103, 232, 249, 0.18); }
  .f-cursor { position: absolute; left: 0; top: 0; width: 34px; height: 40px; transform-origin: 6px 4px; filter: drop-shadow(0 4px 8px rgba(0,0,0,0.55)); }
  .f-ripple { position: absolute; border-radius: 50%; border: 3px solid #67e8f9; box-shadow: 0 0 24px rgba(103,232,249,0.7), inset 0 0 18px rgba(103,232,249,0.35); }
  .f-ripple--fill { border: 0; background: radial-gradient(circle, rgba(103,232,249,0.45), rgba(103,232,249,0) 70%); box-shadow: none; }
  .f-title { position: absolute; left: 0; right: 0; text-align: center; font: 800 108px/1.04 ${SANS}; letter-spacing: -3px; }
  .f-word { display: inline-block; margin: 0 0.13em; }
  .f-pill { display: inline-flex; align-items: center; gap: 14px; padding: 16px 28px; border-radius: 999px; font: 650 36px ${SANS}; margin: 0 12px; }
  .f-pill code { font: 700 30px ${MONO}; color: #67e8f9; }
  .f-threat { position: absolute; top: 590px; width: 540px; padding: 28px 30px; border-radius: 24px; background: rgba(40, 8, 16, 0.72); border: 1px solid rgba(251, 113, 133, 0.38);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.08), 0 30px 80px rgba(0,0,0,0.6), 0 0 60px rgba(244, 63, 94, 0.18); -webkit-backdrop-filter: blur(16px); backdrop-filter: blur(16px); }
  .f-threat__k { font: 750 15px ${SANS}; letter-spacing: 0.16em; color: #fb7185; text-transform: uppercase; }
  .f-threat__t { font: 700 40px/1.1 ${SANS}; margin-top: 10px; letter-spacing: -0.6px; }
  .f-threat__c { font: 600 22px/1.35 ${MONO}; margin-top: 16px; color: #fecdd3; background: rgba(0,0,0,0.35); padding: 12px 16px; border-radius: 12px; border: 1px solid rgba(251,113,133,0.2); }
  .f-chip { display: inline-flex; align-items: center; gap: 10px; padding: 12px 22px; border-radius: 999px; font: 600 26px ${SANS}; margin: 0 10px; }
  .f-metric { display: inline-flex; flex-direction: column; align-items: flex-start; gap: 4px; padding: 18px 26px; border-radius: 20px; margin: 0 10px; min-width: 260px; }
  .f-metric b { font: 750 40px ${MONO}; letter-spacing: -1px; }
  .f-metric span { font: 600 17px ${SANS}; color: #a8b4c6; letter-spacing: 0.04em; text-transform: uppercase; }
  #receipt-dialog { zoom: 1.32; margin: auto auto auto 90px; }
  `;

  const SHIELD = '<svg viewBox="0 0 32 32" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linejoin="round" stroke-linecap="round">'
    + '<path data-p="shield" d="M16 2 4 6.5v8.2c0 7.4 5 13.4 12 15.3 7-1.9 12-7.9 12-15.3V6.5L16 2Z" pathLength="100" stroke-dasharray="100" />'
    + '<path data-p="check" d="m10.5 16.2 3.8 3.8 7.4-8" pathLength="100" stroke-dasharray="100" /></svg>';
  const CURSOR = '<svg viewBox="0 0 34 40" width="34" height="40"><path d="M5 3 L5 31 L12 24.5 L17.2 36 L22 33.8 L16.9 22.6 L26.5 22.6 Z" fill="#fff" stroke="#0b0f17" stroke-width="2" stroke-linejoin="round"/></svg>';

  let spec = null;
  let ui, rig, win, view, wall;
  const S = {
    cam: { cx: W / 2, cy: (VIEW_H + BAR) / 2, s: 0.84, scroll: 0 },
    applied: { tx: 0, ty: 0, s: 1 },
    shot: -1,
    cursor: { x: 0, y: 0, shown: false, ev: -1, from: null, to: null },
    ripples: [],
    press: -10,
    anim: new WeakMap(),
    lower: { idx: -1, node: null },
    cap: { idx: -1, node: null, spans: [] },
    hl: new Map(),
    badges: new Map(),
    dialogOpen: false,
  };

  // ---- DOM lookups ---------------------------------------------------------------------------
  function find(t) {
    if (!t) return null;
    if (t.union) return t.union.map(find).filter(Boolean);
    let list = [...document.querySelectorAll(t.sel || 'button, [role=button], summary')];
    if (t.text) {
      const want = t.text.toLowerCase();
      list = list.filter((n) => (n.textContent || '').trim().toLowerCase().startsWith(want));
    }
    if (t.contains) {
      const want = t.contains.toLowerCase();
      list = list.filter((n) => (n.textContent || '').toLowerCase().includes(want));
    }
    list = list.filter((n) => n.getClientRects().length > 0);
    return list[t.index || 0] || null;
  }
  // Rect of a target in content coordinates (x from the view's left edge, y from the page top).
  function contentRect(t) {
    const node = find(t);
    if (!node) return null;
    const nodes = Array.isArray(node) ? node : [node];
    const { tx, ty, s } = S.applied;
    const rigT = rig.style.transform, rigF = rig.style.filter;
    rig.style.transform = 'none'; rig.style.filter = 'none';
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const n of nodes) {
      const r = n.getBoundingClientRect();
      const wx = (r.left - tx) / s, wy = (r.top - ty) / s;
      x0 = Math.min(x0, wx); y0 = Math.min(y0, wy - BAR + view.scrollTop);
      x1 = Math.max(x1, wx + r.width / s); y1 = Math.max(y1, wy - BAR + view.scrollTop + r.height / s);
    }
    rig.style.transform = rigT; rig.style.filter = rigF;
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }
  const maxScroll = () => Math.max(0, view.scrollHeight - VIEW_H);
  // Scrolls the console's own inner scroll boxes (not the film view) so a target sits at their top.
  function revealInner(node) {
    if (!node || Array.isArray(node)) return;
    for (let p = node.parentElement; p && p !== view; p = p.parentElement) {
      const oy = getComputedStyle(p).overflowY;
      if ((oy === 'auto' || oy === 'scroll') && p.scrollHeight > p.clientHeight + 2) {
        // Rect deltas are in screen pixels (scaled by the camera); scrollTop is in CSS pixels.
        p.scrollTop += (node.getBoundingClientRect().top - p.getBoundingClientRect().top) / S.applied.s - 12;
      }
    }
  }
  const toScreen = (x, y, cam = S.cam) => {
    const tx = W / 2 - cam.s * cam.cx, ty = H / 2 - cam.s * cam.cy;
    return { x: tx + cam.s * x, y: ty + cam.s * (BAR + y - cam.scroll) };
  };

  // Camera that frames a target: zoom (fixed or fitted), scroll so the target sits mid-view.
  function resolveShot(shot) {
    if (shot.wide) return { cx: W / 2, cy: shot.cy ?? 620, s: shot.s ?? 0.84, scroll: shot.scroll ?? S.cam.scroll };
    if (shot.reveal) revealInner(find(shot.target));
    const r = contentRect(shot.target);
    shot.rect = r;
    if (!r) return { ...S.cam };
    // Free area for the subject: below the top badges, above the callout and captions.
    const TOP = 90, BOTTOM = shot.bottom ?? 330;
    const pad = shot.pad ?? 30;
    let s = shot.zoom ?? Math.min(shot.max ?? 2.4, W / (r.w + 2 * pad), (H - TOP - BOTTOM) / (r.h + 2 * pad));
    s = Math.max(s, shot.min ?? 1);
    const topAligned = shot.anchor === 'top';
    const scroll = shot.scroll ?? clamp(topAligned ? r.y - 120 : r.y + r.h / 2 - VIEW_H / 2, 0, maxScroll());
    let cx = r.x + r.w / 2 + (shot.dx ?? 0);
    // Subject centred in the free area (screen y between TOP and H - BOTTOM).
    let cy = BAR + r.y + r.h / 2 - scroll + (H / 2 - (TOP + (H - BOTTOM)) / 2) / s + (shot.dy ?? 0);
    if (topAligned) cy = BAR + r.y - scroll + (H / 2 - TOP - 10) / s;
    if (shot.anchor === 'window') cy = H / 2 / s;
    cx = clamp(cx, W / 2 / s, W - W / 2 / s);
    cy = clamp(cy, H / 2 / s, VIEW_H + BAR - H / 2 / s);
    return { cx, cy, s, scroll };
  }

  function applyCamera(t) {
    const shots = spec.shots;
    let idx = -1;
    for (let i = 0; i < shots.length; i += 1) if (shots[i].t <= t) idx = i;
    if (idx !== S.shot && idx >= 0) {
      S.shot = idx;
      shots[idx].from = { ...S.cam };
      shots[idx].to = resolveShot(shots[idx]);
    }
    if (idx >= 0) {
      const sh = shots[idx];
      const p = sh.cut ? 1 : crit(t - sh.t, sh.dur ?? 1.1);
      S.cam = {
        cx: lerp(sh.from.cx, sh.to.cx, p),
        cy: lerp(sh.from.cy, sh.to.cy, p),
        s: sh.from.s * Math.pow(sh.to.s / sh.from.s, p),
        scroll: lerp(sh.from.scroll, sh.to.scroll, p),
      };
    }
    applySpot(t, idx);
    // Cuts dip through the wallpaper colour; the camera jumps while the frame is dark.
    let dip = 0;
    for (const sh of shots) if (sh.cut) dip = Math.max(dip, Math.min(ramp(t, sh.t - 0.2, sh.t - 0.02), 1 - ramp(t, sh.t + 0.03, sh.t + 0.26)));
    ui.querySelector('.f-dip').style.opacity = dip.toFixed(3);
    const c = S.cam;
    view.scrollTop = c.scroll;
    c.scroll = view.scrollTop; // clamped by the browser
    const tx = W / 2 - c.s * c.cx, ty = H / 2 - c.s * c.cy;
    win.style.transform = `translate(${tx.toFixed(3)}px, ${ty.toFixed(3)}px) scale(${c.s.toFixed(5)})`;
    S.applied = { tx, ty, s: c.s };
  }

  // Spotlight: everything but the shot's subject dims, like a keynote close-up. The lit area morphs
  // from the previous subject to the next with the camera spring; wide shots have none.
  function applySpot(t, idx) {
    const node = ui.querySelector('.f-spot');
    const sh = idx >= 0 ? spec.shots[idx] : null;
    // A console dialog (the receipt) is itself the subject: no dimming over it.
    if (!sh || document.querySelector('dialog[open]')) { node.style.opacity = '0'; return; }
    if (!sh.spotInit) {
      sh.spotInit = true;
      const prev = spec.shots.slice(0, idx).reverse().find((x) => x.spotRect !== undefined);
      sh.spotRect = sh.spot === false || !sh.rect ? null : (sh.spotTarget ? contentRect(sh.spotTarget) : sh.rect);
      sh.spotFrom = prev && prev.spotRect && !sh.cut ? prev.spotRect : null;
    }
    // The subject is measured every frame: a details block that opens grows the lit area with it.
    const live = sh.spotRect ? contentRect(sh.spotTarget || sh.target) : null;
    const r1 = live || null;
    if (r1) sh.spotRect = r1;
    if (!r1) { node.style.opacity = '0'; return; }
    const p = sh.cut ? 1 : crit(t - sh.t, sh.dur ?? 1.1);
    const r0 = sh.spotFrom || r1;
    const r = { x: lerp(r0.x, r1.x, p), y: lerp(r0.y, r1.y, p), w: lerp(r0.w, r1.w, p), h: lerp(r0.h, r1.h, p) };
    const pad = (sh.spotPad ?? 14);
    const a = toScreen(r.x - pad, r.y - pad), b = toScreen(r.x + r.w + pad, r.y + r.h + pad);
    const alpha = sh.spotFrom ? 1 : crit(t - sh.t - (sh.cut ? 0.15 : 0.35), 0.6);
    Object.assign(node.style, { left: `${a.x}px`, top: `${a.y}px`, width: `${b.x - a.x}px`, height: `${b.y - a.y}px`, opacity: alpha.toFixed(3) });
  }

  // ---- console animations on the virtual clock ------------------------------------------------
  function seekAnimations() {
    const now = window.__vt ? window.__vt.now : 0;
    for (const a of document.getAnimations()) {
      const target = a.effect && a.effect.target;
      if (target && ui.contains(target)) continue;
      if (!S.anim.has(a)) S.anim.set(a, now);
      try { a.pause(); a.currentTime = now - S.anim.get(a); } catch { /* finished or detached */ }
    }
  }

  // ---- window rig: entrance and finale --------------------------------------------------------
  function applyRig(t) {
    const enter = spec.windowIn;
    const p = crit(t - enter, 1.25);
    const out = spec.finale ? crit(t - spec.finale.t, 1.0) : 0;
    const y = lerp(820, 0, p), rx = lerp(22, 0, p), sc = lerp(0.88, 1, p) * lerp(1, 0.93, out);
    rig.style.opacity = String(clamp((t - enter) / 0.25, 0, 1));
    rig.style.transform = `perspective(2400px) translateY(${y.toFixed(2)}px) rotateX(${rx.toFixed(3)}deg) scale(${sc.toFixed(4)})`;
    rig.style.filter = out > 0.001 ? `blur(${(out * 16).toFixed(2)}px) brightness(${lerp(1, 0.42, out).toFixed(3)})` : 'none';
  }

  // ---- overlays ------------------------------------------------------------------------------
  function applyCursor(t) {
    const evs = spec.cursor;
    let idx = -1;
    for (let i = 0; i < evs.length; i += 1) if (evs[i].t <= t) idx = i;
    const c = S.cursor;
    if (idx !== c.ev && idx >= 0) {
      c.ev = idx;
      const ev = evs[idx];
      if (!ev.hide) {
        const r = contentRect(ev.target);
        const to = r ? { x: r.x + r.w * (ev.ax ?? 0.32), y: r.y + r.h * (ev.ay ?? 0.42) } : { x: c.x, y: c.y };
        const from = c.shown ? { x: c.x, y: c.y } : { x: to.x + 160 / S.cam.s, y: to.y + 120 / S.cam.s };
        Object.assign(c, { from, to, start: ev.t, dur: ev.dur ?? 0.75, showAt: c.shown ? c.showAt : ev.t });
        c.shown = true;
      } else {
        Object.assign(c, { hideAt: ev.t });
        c.shown = false;
      }
    }
    const node = ui.querySelector('.f-cursor');
    const ev = idx >= 0 ? evs[idx] : null;
    if (!ev) { node.style.opacity = '0'; return; }
    let alpha;
    if (ev.hide) alpha = 1 - ramp(t, c.hideAt, c.hideAt + 0.25);
    else {
      alpha = ramp(t, c.showAt, c.showAt + 0.25);
      const p = easeInOut(clamp((t - c.start) / c.dur, 0, 1));
      const dx = c.to.x - c.from.x, dy = c.to.y - c.from.y;
      const arc = Math.sin(p * Math.PI) * 0.08;
      c.x = lerp(c.from.x, c.to.x, p) - dy * arc;
      c.y = lerp(c.from.y, c.to.y, p) + dx * arc;
    }
    const sp = toScreen(c.x, c.y);
    const press = 1 - 0.18 * env(t, S.press, S.press + 0.22, 0.06, 0.16);
    const size = clamp(0.9 + 0.25 * Math.sqrt(S.cam.s), 1, 1.35);
    node.style.opacity = alpha.toFixed(3);
    node.style.transform = `translate(${(sp.x - 6).toFixed(2)}px, ${(sp.y - 4).toFixed(2)}px) scale(${(size * press).toFixed(3)})`;
    c.screen = sp;
  }

  function applyRipples(t) {
    const layer = ui.querySelector('.f-ripples');
    for (const r of S.ripples) {
      if (!r.nodes) {
        r.nodes = [el('div', { class: 'f-ripple f-ripple--fill' }), el('div', { class: 'f-ripple' }), el('div', { class: 'f-ripple' })];
        layer.append(...r.nodes);
      }
      r.nodes.forEach((n, i) => {
        const a = t - r.t - i * 0.07;
        const p = clamp(a / 0.7, 0, 1);
        const rad = lerp(i === 0 ? 20 : 12, i === 0 ? 70 : 64 + i * 16, crit(a, 0.7));
        n.style.opacity = a < 0 ? '0' : ((1 - p) * (i === 0 ? 0.9 : 0.95)).toFixed(3);
        n.style.width = n.style.height = `${(rad * 2).toFixed(1)}px`;
        n.style.left = `${(r.x - rad).toFixed(1)}px`;
        n.style.top = `${(r.y - rad).toFixed(1)}px`;
      });
    }
  }

  function applyHighlights(t) {
    const layer = ui.querySelector('.f-hls');
    spec.highlights.forEach((h, i) => {
      const on = t >= h.t && t <= h.t + h.dur;
      let node = S.hl.get(i);
      if (!on) { if (node) node.style.opacity = '0'; return; }
      if (!node) {
        const col = COLORS[h.color || 'cyan'];
        node = el('div', { class: 'f-hl', style: `border-color: ${col}; box-shadow: 0 0 0 6px ${col}22, 0 0 40px ${col}88, inset 0 0 24px ${col}33;` });
        layer.append(node);
        S.hl.set(i, node);
        h.rect = contentRect(h.target);
      }
      if (!h.rect) return;
      const pad = h.pad ?? 8;
      const a = toScreen(h.rect.x - pad, h.rect.y - pad), b = toScreen(h.rect.x + h.rect.w + pad, h.rect.y + h.rect.h + pad);
      const e = env(t, h.t, h.t + h.dur, 0.3, 0.4);
      const grow = 1 + 0.12 * (1 - pop(t - h.t, 0.6));
      const pulse = 0.85 + 0.15 * Math.sin((t - h.t) * 6.2);
      const cx = (a.x + b.x) / 2, cy = (a.y + b.y) / 2, w = (b.x - a.x) * grow, hh = (b.y - a.y) * grow;
      Object.assign(node.style, { left: `${cx - w / 2}px`, top: `${cy - hh / 2}px`, width: `${w}px`, height: `${hh}px`, opacity: (e * pulse).toFixed(3) });
    });
  }

  function applyLower(t) {
    const list = spec.lowers;
    let idx = -1;
    for (let i = 0; i < list.length; i += 1) if (t >= list[i].t && t <= list[i].t + list[i].dur) idx = i;
    const L = S.lower;
    if (idx !== L.idx) {
      if (L.node) L.node.remove();
      L.node = null;
      L.idx = idx;
      if (idx >= 0) {
        const d = list[idx];
        const col = COLORS[d.color || 'cyan'];
        L.node = el('div', { class: 'f-lower f-glass', style: d.pos || '' },
          el('div', { class: 'f-lower__bar', style: `background: linear-gradient(180deg, ${col}, ${col}55);` }),
          el('div', { class: 'f-lower__icon', style: `background: ${col}1f; box-shadow: inset 0 0 0 1px ${col}55, 0 0 34px ${col}40;`, text: d.icon || '•' }),
          el('div', {},
            el('div', { class: 'f-lower__kicker', style: `color: ${col};`, text: d.kicker || '' }),
            d.title ? el('div', { class: 'f-lower__title', text: d.title }) : null,
            d.value ? el('div', { class: 'f-lower__value', style: `color: ${col}; text-shadow: 0 0 30px ${col}66;`, text: d.value }) : null,
            d.sub ? el('div', { class: 'f-lower__sub', text: d.sub }) : null));
        ui.querySelector('.f-lowers').append(L.node);
      }
    }
    if (!L.node) return;
    const d = list[idx];
    const pIn = pop(t - d.t, 0.75), pOut = ramp(t, d.t + d.dur - 0.35, d.t + d.dur);
    L.node.style.opacity = (clamp(pIn * 1.6, 0, 1) * (1 - pOut)).toFixed(3);
    L.node.style.transform = `translate(${lerp(-60, 0, pIn) - 30 * pOut}px, 0) scale(${lerp(0.96, 1, pIn).toFixed(4)})`;
  }

  function applyBadges(t) {
    spec.badges.forEach((b, i) => {
      let node = S.badges.get(i);
      const on = t >= b.t && t <= b.t + b.dur;
      if (!node) {
        const col = COLORS[b.color || 'emerald'];
        node = el('div', { class: 'f-badge f-glass', style: `${b.pos || 'right: 56px; top: 56px;'} color: ${col};` },
          el('i', { class: 'f-dot', style: `background: ${col}; box-shadow: 0 0 12px ${col};` }), el('span', { text: b.text }));
        ui.querySelector('.f-badges').append(node);
        S.badges.set(i, node);
      }
      const e = on ? env(t, b.t, b.t + b.dur, 0.4, 0.4) : 0;
      node.style.opacity = e.toFixed(3);
      node.style.transform = `translateY(${lerp(-12, 0, pop(t - b.t, 0.6)).toFixed(2)}px)`;
      const dot = node.querySelector('.f-dot');
      if (b.blink) dot.style.opacity = (0.45 + 0.55 * (0.5 + 0.5 * Math.cos((t - b.t) * 5))).toFixed(3);
    });
  }

  function applyCaptions(t) {
    const caps = spec.captions;
    let idx = -1;
    for (let i = 0; i < caps.length; i += 1) if (t >= caps[i].start - 0.08 && t < caps[i].end) idx = i;
    const C = S.cap;
    if (idx !== C.idx) {
      if (C.node) C.node.remove();
      C.node = null;
      C.idx = idx;
      if (idx >= 0) {
        C.spans = caps[idx].words.map((w) => el('span', { text: w.w }));
        C.node = el('div', { class: 'f-cap' }, ...C.spans);
        ui.querySelector('.f-caps').append(C.node);
      }
    }
    if (!C.node) return;
    const cap = caps[idx];
    caps[idx].words.forEach((w, i) => {
      const cls = t >= w.e ? 'w-past' : t >= w.s - 0.02 ? 'w-now' : 'w-next';
      if (C.spans[i].className !== cls) C.spans[i].className = cls;
      const lift = cls === 'w-now' ? -3 * (1 - ramp(t, w.s, w.s + 0.12)) - 1 : 0;
      C.spans[i].style.transform = `translateY(${lift.toFixed(2)}px)`;
    });
    const a = ramp(t, cap.start - 0.08, cap.start + 0.06) * (1 - ramp(t, cap.end - 0.08, cap.end));
    C.node.style.opacity = a.toFixed(3);
    C.node.style.transform = `translateX(-50%) translateY(${lerp(10, 0, ramp(t, cap.start - 0.08, cap.start + 0.12)).toFixed(2)}px)`;
  }

  // ---- title scenes --------------------------------------------------------------------------
  function wordsIn(container, t, items, base) {
    items.forEach((it, i) => {
      const n = container.children[i];
      const p = pop(t - it.at, 0.7, 0.7);
      n.style.opacity = clamp((t - it.at) / 0.18, 0, 1).toFixed(3);
      n.style.transform = `translateY(${lerp(base ?? 46, 0, p).toFixed(2)}px) scale(${lerp(0.94, 1, p).toFixed(4)})`;
      n.style.filter = `blur(${(lerp(12, 0, clamp((t - it.at) / 0.3, 0, 1))).toFixed(2)}px)`;
    });
  }

  function buildScenes() {
    const hk = spec.hook, br = spec.brand, fi = spec.finale;
    const hook = el('div', { class: 'f-scene', 'data-scene': 'hook', style: 'background: radial-gradient(1200px 700px at 50% 20%, #0b1424, #03050a 70%);' },
      el('div', { class: 'f-abs', 'data-k': 'floor', style: `left: -1000px; right: -1000px; top: 560px; height: 1400px; transform-origin: 50% 0; transform: perspective(900px) rotateX(70deg);
        background-image: linear-gradient(rgba(103,232,249,0.22) 2px, transparent 2px), linear-gradient(90deg, rgba(103,232,249,0.22) 2px, transparent 2px); background-size: 120px 120px;
        mask-image: linear-gradient(180deg, transparent, #000 25%, #000 60%, transparent);` }),
      el('div', { class: 'f-abs', 'data-k': 'redwash', style: 'inset: 0; background: radial-gradient(1000px 700px at 50% 50%, rgba(244,63,94,0.0), rgba(244,63,94,0.28)); opacity: 0;' }),
      el('div', { class: 'f-title', 'data-k': 'l1', style: 'top: 250px;' }, ...hk.line1.map((w) => el('span', { class: 'f-word', text: w.text }))),
      el('div', { class: 'f-abs', 'data-k': 'pills', style: 'left: 0; right: 0; top: 420px; text-align: center;' },
        ...hk.pills.map((p) => el('span', { class: 'f-pill f-glass' }, el('code', { text: p.icon }), el('span', { text: p.text })))),
      el('div', { class: 'f-title', 'data-k': 'l2', style: 'top: 220px; font-size: 84px; color: #fb7185; text-shadow: 0 0 50px rgba(244,63,94,0.55);' },
        ...hk.line2.map((w) => el('div', { class: 'f-word', style: 'display: block;', text: w.text }))),
      ...hk.threats.map((th, i) => el('div', { class: 'f-threat', 'data-k': `th${i}`, style: `left: ${150 + i * 560}px;` },
        el('div', { class: 'f-threat__k', text: th.kicker }), el('div', { class: 'f-threat__t', text: th.title }), el('div', { class: 'f-threat__c', text: th.code }))));

    const brand = el('div', { class: 'f-scene', 'data-scene': 'brand' },
      el('div', { class: 'f-abs', 'data-k': 'logo', style: 'left: 50%; top: 250px; width: 170px; height: 170px; margin-left: -85px; color: #67e8f9; filter: drop-shadow(0 0 30px rgba(103,232,249,0.6));', html: SHIELD }),
      el('div', { class: 'f-title', 'data-k': 'word', style: 'top: 460px; font-size: 150px; letter-spacing: -5px;' },
        ...[...br.word].map((ch) => el('span', { style: 'display: inline-block; background: linear-gradient(180deg, #ffffff, #a5f3fc); -webkit-background-clip: text; background-clip: text; color: transparent;', text: ch }))),
      el('div', { class: 'f-abs', 'data-k': 'chips', style: 'left: 0; right: 0; top: 680px; text-align: center;' },
        ...br.chips.map((c) => el('span', { class: 'f-chip f-glass', style: `color: ${COLORS[c.color || 'cyan']};`, text: c.text }))));

    const finale = el('div', { class: 'f-scene', 'data-scene': 'finale' },
      el('div', { class: 'f-abs', 'data-k': 'logo', style: 'left: 50%; top: 170px; width: 130px; height: 130px; margin-left: -65px; color: #67e8f9; filter: drop-shadow(0 0 26px rgba(103,232,249,0.6));', html: SHIELD }),
      el('div', { class: 'f-title', 'data-k': 'word', style: 'top: 320px; font-size: 128px; letter-spacing: -4px; background: linear-gradient(180deg, #fff, #a5f3fc); -webkit-background-clip: text; background-clip: text; color: transparent;', text: fi.word }),
      el('div', { class: 'f-title', 'data-k': 'tag', style: 'top: 478px; font: 600 38px/1.2 ' + SANS + '; letter-spacing: -0.5px; color: #c4cedc;', text: fi.tagline }),
      el('div', { class: 'f-abs', 'data-k': 'metrics', style: 'left: 0; right: 0; top: 580px; text-align: center;' },
        ...fi.metrics.map((m) => el('span', { class: 'f-metric f-glass' }, el('b', { style: `color: ${COLORS[m.color || 'cyan']};`, text: m.value }), el('span', { text: m.label })))),
      el('div', { class: 'f-title', 'data-k': 'url', style: 'top: 800px; font: 700 40px ' + MONO + '; letter-spacing: -0.5px; color: #f8fafc;', text: fi.url }),
      el('div', { class: 'f-title', 'data-k': 'lic', style: 'top: 868px; font: 600 26px ' + SANS + '; color: #8d9bb0; letter-spacing: 0.02em;', text: fi.license }));
    return { hook, brand, finale };
  }

  function applyHook(t, n) {
    const hk = spec.hook;
    const vis = t < hk.end + 0.1;
    n.style.display = vis ? 'block' : 'none';
    if (!vis) return;
    const out = ramp(t, hk.end - 0.55, hk.end);
    n.style.opacity = (1 - out).toFixed(3);
    n.style.transform = `scale(${lerp(1, 1.1, easeInOut(out)).toFixed(4)})`;
    n.style.filter = out > 0 ? `blur(${(out * 10).toFixed(2)}px)` : 'none';
    const floor = n.querySelector('[data-k=floor]');
    floor.style.backgroundPosition = `0 ${(t * 60).toFixed(1)}px, 0 0`;
    const l1 = n.querySelector('[data-k=l1]'), pills = n.querySelector('[data-k=pills]'), l2 = n.querySelector('[data-k=l2]');
    wordsIn(l1, t, hk.line1);
    wordsIn(pills, t, hk.pills, 30);
    const dim = crit(t - hk.turn, 0.6);
    l1.style.opacity = lerp(1, 0, dim).toFixed(3);
    l1.style.transform = `translateY(${lerp(0, -80, dim).toFixed(1)}px) scale(${lerp(1, 0.9, dim).toFixed(4)})`;
    pills.style.opacity = lerp(1, 0, dim).toFixed(3);
    pills.style.transform = `translateY(${lerp(0, -60, dim).toFixed(1)}px)`;
    wordsIn(l2, t, hk.line2, 30);
    const up = crit(t - hk.threatsAt + 0.2, 0.7);
    l2.style.transform = `translateY(${lerp(0, -90, up).toFixed(1)}px) scale(${lerp(1, 0.82, up).toFixed(4)})`;
    let shake = 0;
    for (const s of hk.shakes) { const a = t - s; if (a >= 0 && a < 0.45) shake += Math.sin(a * 70) * 14 * (1 - a / 0.45); }
    l2.style.marginLeft = `${shake.toFixed(2)}px`;
    const wash = n.querySelector('[data-k=redwash]');
    wash.style.opacity = (0.55 * ramp(t, hk.turn, hk.turn + 0.8) + hk.shakes.reduce((acc, s) => acc + 0.4 * env(t, s, s + 0.5, 0.04, 0.4), 0)).toFixed(3);
    hk.threats.forEach((th, i) => {
      const c = n.querySelector(`[data-k=th${i}]`);
      const p = pop(t - th.at, 0.8, 0.66);
      c.style.opacity = clamp((t - th.at) / 0.2, 0, 1).toFixed(3);
      c.style.transform = `translateY(${lerp(90, 0, p).toFixed(1)}px) rotate(${lerp(i === 0 ? -3 : i === 2 ? 3 : 0, 0, p).toFixed(3)}deg)`;
    });
  }

  function applyBrand(t, n) {
    const br = spec.brand;
    const vis = t > br.t - 0.1 && t < br.end + 0.1;
    n.style.display = vis ? 'block' : 'none';
    if (!vis) return;
    const out = crit(t - (br.end - 0.7), 0.7);
    n.style.opacity = (ramp(t, br.t, br.t + 0.2) * (1 - out)).toFixed(3);
    n.style.transform = `translateY(${lerp(0, -160, out).toFixed(1)}px) scale(${lerp(1, 0.86, out).toFixed(4)})`;
    const logo = n.querySelector('[data-k=logo]');
    logo.querySelector('[data-p=shield]').style.strokeDashoffset = String(lerp(100, 0, crit(t - br.t, 0.8)));
    logo.querySelector('[data-p=check]').style.strokeDashoffset = String(lerp(100, 0, crit(t - br.t - 0.45, 0.5)));
    logo.style.transform = `scale(${lerp(0.7, 1, pop(t - br.t, 0.9)).toFixed(4)})`;
    [...n.querySelector('[data-k=word]').children].forEach((ch, i) => {
      const at = br.wordAt + i * 0.035;
      const p = pop(t - at, 0.7, 0.6);
      ch.style.opacity = clamp((t - at) / 0.15, 0, 1).toFixed(3);
      ch.style.transform = `translateY(${lerp(60, 0, p).toFixed(1)}px)`;
    });
    wordsIn(n.querySelector('[data-k=chips]'), t, br.chips, 26);
  }

  function applyFinale(t, n) {
    const fi = spec.finale;
    const vis = t > fi.t;
    n.style.display = vis ? 'block' : 'none';
    if (!vis) return;
    const fade = 1 - ramp(t, spec.duration - 0.7, spec.duration);
    n.style.opacity = (ramp(t, fi.t, fi.t + 0.3) * fade).toFixed(3);
    const logo = n.querySelector('[data-k=logo]');
    logo.querySelector('[data-p=shield]').style.strokeDashoffset = String(lerp(100, 0, crit(t - fi.t, 0.8)));
    logo.querySelector('[data-p=check]').style.strokeDashoffset = String(lerp(100, 0, crit(t - fi.t - 0.4, 0.5)));
    const parts = ['word', 'tag', 'metrics', 'url', 'lic'];
    parts.forEach((k, i) => {
      const node = n.querySelector(`[data-k=${k}]`);
      const at = fi.t + 0.25 + i * 0.16;
      const p = pop(t - at, 0.75, 0.7);
      node.style.opacity = clamp((t - at) / 0.2, 0, 1).toFixed(3);
      node.style.transform = `translateY(${lerp(40, 0, p).toFixed(1)}px)`;
    });
    document.getElementById('film-ui').style.setProperty('--fade', fade.toFixed(3));
  }

  // Keep the overlay above any console dialog that opened after it (both live in the top layer).
  function keepOnTop() {
    const open = Boolean(document.querySelector('dialog[open]'));
    if (open && !S.dialogOpen) { ui.hidePopover(); ui.showPopover(); }
    S.dialogOpen = open;
  }

  window.__film = {
    setup(s) {
      spec = s;
      document.head.append(el('style', { text: CSS }));
      wall = el('div', { id: 'film-wall' });
      rig = el('div', { id: 'film-rig' });
      win = el('div', { id: 'film-win' });
      view = el('div', { id: 'film-view' });
      const bar = el('div', { class: 'film-bar' },
        el('div', { class: 'film-bar__lights' }, el('i', { style: 'background:#ff5f57' }), el('i', { style: 'background:#febc2e' }), el('i', { style: 'background:#28c840' })),
        el('span', { text: s.windowTitle }), el('span', { class: 'film-bar__url', text: s.windowUrl }));
      const keep = (n) => !['DIALOG', 'SCRIPT', 'NOSCRIPT', 'STYLE'].includes(n.tagName);
      view.append(...[...document.body.children].filter(keep));
      win.append(bar, view);
      rig.append(win);
      document.body.prepend(wall, rig);
      ui = el('div', { id: 'film-ui', popover: 'manual' },
        el('div', { class: 'f-dip f-abs', style: 'inset: 0; background: #04060b; opacity: 0;' }),
        el('div', { class: 'f-spot', style: 'opacity: 0;' }),
        el('div', { class: 'f-hls f-abs', style: 'inset: 0;' }),
        el('div', { class: 'f-lowers f-abs', style: 'inset: 0;' }),
        el('div', { class: 'f-badges f-abs', style: 'inset: 0;' }),
        el('div', { class: 'f-scenes f-abs', style: 'inset: 0;' }),
        el('div', { class: 'f-ripples f-abs', style: 'inset: 0;' }),
        el('div', { class: 'f-cursor', html: CURSOR, style: 'opacity: 0;' }),
        el('div', { class: 'f-caps f-abs', style: 'inset: 0;' }));
      document.body.append(ui);
      ui.showPopover();
      const scenes = buildScenes();
      ui.querySelector('.f-scenes').append(scenes.hook, scenes.brand, scenes.finale);
      S.scenes = scenes;
      return { scrollHeight: view.scrollHeight };
    },
    frame(t) {
      if (window.__vt) window.__vt.advanceTo(t * 1000);
      seekAnimations();
      keepOnTop();
      applyCamera(t);
      applyRig(t);
      applyHook(t, S.scenes.hook);
      applyBrand(t, S.scenes.brand);
      applyFinale(t, S.scenes.finale);
      applyHighlights(t);
      applyLower(t);
      applyBadges(t);
      applyCursor(t);
      applyRipples(t);
      applyCaptions(t);
      return true;
    },
    // A real click on the console element, drawn as a press + ripple at the cursor.
    click(target, t) {
      const n = find(target);
      if (!n) return false;
      S.press = t;
      const sp = S.cursor.screen || { x: W / 2, y: H / 2 };
      S.ripples.push({ t, x: sp.x, y: sp.y });
      n.click();
      return true;
    },
    // Fill in a callout or badge from a live daemon value (by its spec id) before it shows.
    patch(kind, id, fields) {
      const item = (spec[kind] || []).find((x) => x.id === id);
      if (item) Object.assign(item, fields);
      return Boolean(item);
    },
    rect: contentRect,
    debug: () => spec.shots.map((x) => ({ t: x.t, rect: x.rect && Object.fromEntries(Object.entries(x.rect).map(([k, v]) => [k, Math.round(v)])), to: x.to && { cx: Math.round(x.to.cx), cy: Math.round(x.to.cy), s: +x.to.s.toFixed(2), scroll: Math.round(x.to.scroll) } })),
    camera: () => ({ ...S.cam }),
  };
})();
