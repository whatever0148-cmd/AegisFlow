// Virtual clock for frame-exact capture, injected before the console's own scripts.
//
// setTimeout / setInterval / requestAnimationFrame / performance.now run on a clock that only moves
// when the film renderer calls __vt.advanceTo(ms), so the console's own staggered animations play
// at video speed no matter how long a screenshot or a live daemon call takes in real time. Network
// I/O (fetch, WebSocket) stays real: the daemon's answers arrive whenever the daemon sends them.
(() => {
  if (window.__vt) return;
  const realNow = performance.now.bind(performance);
  const base = realNow();
  let now = 0;
  let nextId = 1;
  const timers = new Map();
  let frames = new Map();

  const add = (fn, ms, args, repeat) => {
    const id = nextId++;
    const delay = Math.max(0, Number(ms) || 0);
    timers.set(id, { fn, args, due: now + delay, every: repeat ? Math.max(1, delay) : 0 });
    return id;
  };
  window.setTimeout = (fn, ms, ...args) => add(fn, ms, args, false);
  window.setInterval = (fn, ms, ...args) => add(fn, ms, args, true);
  window.clearTimeout = (id) => { timers.delete(id); };
  window.clearInterval = (id) => { timers.delete(id); };
  window.requestAnimationFrame = (fn) => { const id = nextId++; frames.set(id, fn); return id; };
  window.cancelAnimationFrame = (id) => { frames.delete(id); };
  performance.now = () => base + now;

  const run = (fn, args) => {
    try { if (typeof fn === 'function') fn(...args); else (0, eval)(String(fn)); } catch (e) { console.error(e); }
  };

  window.__vt = {
    get now() { return now; },
    advanceTo(target) {
      for (let guard = 0; guard < 10000; guard += 1) {
        let pick = null;
        for (const [id, t] of timers) if (t.due <= target && (!pick || t.due < pick[1].due)) pick = [id, t];
        if (!pick) break;
        const [id, t] = pick;
        now = Math.max(now, t.due);
        if (t.every) t.due += t.every; else timers.delete(id);
        run(t.fn, t.args || []);
      }
      now = Math.max(now, target);
      const due = frames;
      frames = new Map();
      for (const fn of due.values()) run(fn, [base + now]);
    },
    // Pending timer count, for the renderer's settle loop.
    get pending() { return timers.size; },
  };
})();
