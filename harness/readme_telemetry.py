#!/usr/bin/env python3
"""Regenerates the telemetry block of README.md from artifacts/aegis_phase1_telemetry.json.

    python3 harness/readme_telemetry.py [--check]

Everything between <!-- aegis:telemetry:start --> and <!-- aegis:telemetry:end --> is replaced; no
number in that block is typed by hand. --check exits 1 when README.md is out of date.
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
README = ROOT / "README.md"
ARTIFACT = ROOT / "artifacts" / "aegis_phase1_telemetry.json"
START, END = "<!-- aegis:telemetry:start -->", "<!-- aegis:telemetry:end -->"


def fmt_ms(v):
    if v is None:
        return "—"
    return f"{v * 1000:.1f} µs" if v < 1 else f"{v:.3f} ms"


def measured(row):
    m = row.get("measured")
    if row.get("measured_p50_ms") is not None:
        text = f"p50 {fmt_ms(row['measured_p50_ms'])}"
        if row.get("p95_ms") is not None:
            text += f" · p95 {fmt_ms(row['p95_ms'])}"
        if row.get("n"):
            text += f" · n={row['n']}"
        return text
    if isinstance(m, dict):
        if "neural_engine" in m:
            return f"ANE {m['neural_engine']} · CPU {m.get('cpu')} · GPU {m.get('gpu')}"
        if "speedup" in m:
            return f"{m['speedup']:.2f}× ({fmt_ms(m.get('cpuAndNeuralEngine p50 ms'))} vs {fmt_ms(m.get('cpuOnly p50 ms'))} cpuOnly)"
        return ", ".join(f"{k} {v}" for k, v in m.items())
    return str(m)


def block(a):
    rows = a["targets_summary"]
    passed = sum(1 for r in rows if r.get("pass") is True)
    rc = a.get("run_conditions", {})
    uc = rc.get("user_contention") or {}
    host = a.get("host") or {}
    status = "PRELIMINARY" if a.get("preliminary") else "verified (full scale, quiet machine)"
    out = [
        START,
        f"**{passed}/{len(rows)} gated targets met** · run {status} · generated {a.get('generated_at', '?')}"
        f" · {host.get('chip') or host.get('cpu') or 'Apple Silicon'}",
        "",
        "| Target | Threshold | Measured | Result |",
        "|---|---|---|---|",
    ]
    for r in rows:
        verdict = "✅ PASS" if r.get("pass") is True else ("❌ FAIL" if r.get("pass") is False else "❌ not measured")
        out.append(f"| {r['metric']} | {r.get('target', '')} | {measured(r)} | {verdict} |")
    info = {r["metric"]: r for r in a.get("informational", [])}
    cold = [r for m, r in info.items() if m.startswith("cold clonefile branch")]
    if cold:
        out += ["", "Cold `clonefile` for comparison (informational, not gated):", "",
                "| Fixture | Cold clonefile p50 |", "|---|---|"]
        out += [f"| {r['metric'].split('·', 1)[-1].strip()} | {fmt_ms(r.get('measured_p50_ms'))} |" for r in cold]
    if uc.get("mean_cores") is not None:
        out += ["", f"Run conditions: other programs averaged {uc['mean_cores']:.2f} cores during the run "
                f"(busy threshold {uc.get('busy_threshold_cores')}); start load {rc.get('load_average_start', ['?'])[0]}."]
    out.append(END)
    return "\n".join(out)


def main():
    a = json.loads(ARTIFACT.read_text())
    text = README.read_text()
    if START not in text or END not in text:
        sys.exit("README.md has no telemetry markers")
    head, rest = text.split(START, 1)
    _, tail = rest.split(END, 1)
    new = head + block(a) + tail
    if "--check" in sys.argv:
        sys.exit(0 if new == text else 1)
    README.write_text(new)
    print("README.md telemetry block regenerated")


if __name__ == "__main__":
    main()
