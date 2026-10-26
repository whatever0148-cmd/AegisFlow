// Generated sound design for the film (no samples, no downloads): a low ambient pad, tension
// pulses and impacts under the hook, a chime for the brand reveal and the finale, a whoosh for
// the window entrance, and a soft UI click for every real click on the console. Returns a
// 48 kHz stereo 16-bit WAV buffer exactly `duration` seconds long.
const SR = 48000;

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function renderSfx({ duration, clicks, lowers, spec }) {
  const n = Math.round(duration * SR);
  const L = new Float32Array(n), R = new Float32Array(n);
  const rand = rng(7);
  const db = (d) => Math.pow(10, d / 20);
  const add = (i, l, r = l) => { if (i >= 0 && i < n) { L[i] += l; R[i] += r; } };

  // Ambient pad: an open A chord with slow movement, low-passed, under everything.
  const partials = [[55, 1], [82.41, 0.7], [110, 0.55], [123.47, 0.35], [164.81, 0.3], [220, 0.16]];
  let lpL = 0, lpR = 0;
  const lpA = 1 - Math.exp((-2 * Math.PI * 900) / SR);
  for (let i = 0; i < n; i += 1) {
    const t = i / SR;
    let l = 0, r = 0;
    for (const [f, a] of partials) {
      const wob = 1 + 0.12 * Math.sin(2 * Math.PI * (0.07 + f / 4000) * t);
      l += a * wob * Math.sin(2 * Math.PI * f * 1.0007 * t);
      r += a * wob * Math.sin(2 * Math.PI * f * 0.9993 * t + 0.6);
    }
    const fade = Math.min(1, t / 2.5) * Math.min(1, (duration - t) / 2.2);
    const lift = t < spec.hook.end ? 1.25 : t > spec.finale.t ? 1.15 : 0.85;
    lpL += lpA * (l - lpL); lpR += lpA * (r - lpR);
    L[i] += lpL * db(-36) * fade * lift; R[i] += lpR * db(-36) * fade * lift;
  }

  // Low impact: a sine that drops in pitch, plus a short noise transient.
  const impact = (t0, gain, f0 = 70, f1 = 34, len = 0.7) => {
    let ph = 0;
    for (let k = 0; k < len * SR; k += 1) {
      const t = k / SR, p = t / len;
      ph += (2 * Math.PI * (f0 + (f1 - f0) * Math.sqrt(p))) / SR;
      const env = Math.exp(-5.5 * p) * Math.min(1, t / 0.004);
      const noise = t < 0.03 ? (rand() * 2 - 1) * Math.exp(-t / 0.008) * 0.35 : 0;
      add(Math.round(t0 * SR) + k, (Math.sin(ph) * env + noise) * gain);
    }
  };
  // Bell-like chime from a few inharmonic partials.
  const chime = (t0, gain, base = 523.25) => {
    const parts = [[1, 1], [1.5, 0.6], [2, 0.45], [3.01, 0.25], [4.2, 0.12]];
    for (let k = 0; k < 2.6 * SR; k += 1) {
      const t = k / SR;
      let v = 0;
      parts.forEach(([m, a], j) => { v += a * Math.sin(2 * Math.PI * base * m * t) * Math.exp(-t * (1.6 + j * 0.9)); });
      const env = Math.min(1, t / 0.006);
      add(Math.round(t0 * SR) + k, v * env * gain * 0.9, v * env * gain);
    }
  };
  // Band-passed noise sweep (state-variable filter), rising or falling.
  const whoosh = (t0, len, gain, fFrom = 300, fTo = 3200) => {
    let low = 0, band = 0;
    for (let k = 0; k < len * SR; k += 1) {
      const p = k / (len * SR);
      const f = fFrom * Math.pow(fTo / fFrom, p);
      const F = 2 * Math.sin((Math.PI * f) / SR), q = 0.5;
      const x = rand() * 2 - 1;
      low += F * band; const high = x - low - q * band; band += F * high;
      const env = Math.sin(Math.PI * p) ** 2;
      add(Math.round(t0 * SR) + k, band * env * gain * (1 - 0.3 * p), band * env * gain * (0.7 + 0.3 * p));
    }
  };
  // UI click: a tiny noise tick plus a fast-decaying high sine.
  const click = (t0, gain) => {
    for (let k = 0; k < 0.06 * SR; k += 1) {
      const t = k / SR;
      const v = (rand() * 2 - 1) * Math.exp(-t / 0.0012) * 0.8 + Math.sin(2 * Math.PI * 2300 * t) * Math.exp(-t / 0.012) * 0.6
        + Math.sin(2 * Math.PI * 900 * t) * Math.exp(-t / 0.02) * 0.25;
      add(Math.round(t0 * SR) + k, v * gain);
    }
  };
  // Soft two-note pluck for callouts.
  const blip = (t0, gain) => {
    for (const [dt, f] of [[0, 880], [0.06, 1318.5]]) {
      for (let k = 0; k < 0.25 * SR; k += 1) {
        const t = k / SR;
        add(Math.round((t0 + dt) * SR) + k, Math.sin(2 * Math.PI * f * t) * Math.exp(-t / 0.07) * Math.min(1, t / 0.003) * gain);
      }
    }
  };

  const hk = spec.hook;
  for (let t = hk.turn; t < hk.end - 0.6; t += 0.74) impact(t, db(-24), 52, 40, 0.35);
  for (const w of hk.line2) impact(w.at, db(-14));
  for (const th of hk.threats) impact(th.at, db(-16), 80, 38, 0.8);
  whoosh(spec.brand.t - 1.1, 1.15, db(-20), 200, 5000);
  chime(spec.brand.t, db(-20));
  whoosh(spec.windowIn - 0.1, 0.95, db(-24), 2400, 260);
  for (const t of lowers) blip(t + 0.04, db(-30));
  for (const t of clicks) click(t, db(-16));
  whoosh(spec.finale.t - 0.5, 0.9, db(-24), 300, 4000);
  chime(spec.finale.t + 0.15, db(-21), 440);

  const buf = Buffer.alloc(44 + n * 4);
  buf.write('RIFF', 0); buf.writeUInt32LE(36 + n * 4, 4); buf.write('WAVE', 8);
  buf.write('fmt ', 12); buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(2, 22);
  buf.writeUInt32LE(SR, 24); buf.writeUInt32LE(SR * 4, 28); buf.writeUInt16LE(4, 32); buf.writeUInt16LE(16, 34);
  buf.write('data', 36); buf.writeUInt32LE(n * 4, 40);
  for (let i = 0; i < n; i += 1) {
    buf.writeInt16LE(Math.round(Math.max(-1, Math.min(1, L[i])) * 32767), 44 + i * 4);
    buf.writeInt16LE(Math.round(Math.max(-1, Math.min(1, R[i])) * 32767), 46 + i * 4);
  }
  return buf;
}
