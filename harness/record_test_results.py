#!/usr/bin/env python3
"""Parse `cargo test` output into artifacts/aegis_telemetry.json.

Test binaries are mapped back to their package with `cargo metadata`, so the brain can show
per-crate pass/fail. The latest benchmark file is embedded unchanged under "bench".
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parent.parent
RUNNING = re.compile(r"^\s*Running (unittests )?(\S+) \(([^)]+)\)")
DOCTEST = re.compile(r"^\s*Doc-tests (\S+)")
RESULT = re.compile(r"^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; "
                    r"(\d+) filtered out(?:; finished in ([\d.]+)s)?")
FAILED_TEST = re.compile(r"^test (\S+) \.\.\. FAILED", re.M)


def now_iso() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def cargo_env() -> Dict[str, str]:
    env = dict(os.environ)
    env["PATH"] = str(Path.home() / ".cargo" / "bin") + ":" + env.get("PATH", "")
    return env


def target_map() -> Dict[str, str]:
    proc = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"],
                          cwd=ROOT, capture_output=True, text=True, env=cargo_env())
    mapping: Dict[str, str] = {}
    if proc.returncode != 0:
        return mapping
    for pkg in json.loads(proc.stdout).get("packages", []):
        crate = pkg["name"].replace("-", "_")
        mapping[crate] = crate
        for target in pkg.get("targets", []):
            mapping[target["name"].replace("-", "_")] = crate
    return mapping


def parse(text: str, mapping: Dict[str, str]) -> List[Dict[str, Any]]:
    suites: List[Dict[str, Any]] = []
    current: Optional[Dict[str, Any]] = None
    for line in text.splitlines():
        m = RUNNING.match(line)
        if m:
            binary = Path(m.group(3)).name.rsplit("-", 1)[0]
            current = {"crate": mapping.get(binary, binary), "target": binary, "source": m.group(2),
                       "kind": "unittests" if m.group(1) else "integration"}
            continue
        m = DOCTEST.match(line)
        if m:
            name = m.group(1)
            current = {"crate": mapping.get(name, name), "target": name, "source": "doc-tests", "kind": "doctests"}
            continue
        m = RESULT.match(line)
        if m and current is not None:
            current.update({"result": m.group(1), "passed": int(m.group(2)), "failed": int(m.group(3)),
                            "ignored": int(m.group(4)), "duration_s": float(m.group(7)) if m.group(7) else None})
            suites.append(current)
            current = None
    return suites


def git_info() -> Dict[str, Any]:
    try:
        sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True,
                             text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return {"commit": "uncommitted", "dirty": True}
    dirty = subprocess.run(["git", "status", "--porcelain", "--", "crates", "policies", "models", "Cargo.toml",
                            "Cargo.lock"], cwd=ROOT, capture_output=True, text=True).stdout.strip() != ""
    return {"commit": sha, "dirty": dirty}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--log", required=True)
    ap.add_argument("--exit-code", type=int, required=True)
    ap.add_argument("--duration-s", type=float, default=None)
    ap.add_argument("--bench", default="artifacts/aegis_phase1_telemetry.json")
    ap.add_argument("--out", default="artifacts/aegis_telemetry.json")
    args = ap.parse_args()

    text = (ROOT / args.log).read_text(errors="replace")
    suites = parse(text, target_map())
    bench_path = ROOT / args.bench
    bench = json.loads(bench_path.read_text()) if bench_path.exists() else None
    doc = {
        "schema": "aegisflow.telemetry.v1",
        "generated_at": now_iso(),
        "git": git_info(),
        "tests": {
            "command": "cargo test --release --workspace",
            "exit_code": args.exit_code,
            "finished_at": now_iso(),
            "duration_s": args.duration_s,
            "passed": sum(s["passed"] for s in suites),
            "failed": sum(s["failed"] for s in suites),
            "ignored": sum(s["ignored"] for s in suites),
            "suites": suites,
            "failed_tests": FAILED_TEST.findall(text),
            "compiled": bool(suites) or args.exit_code == 0,
        },
        "bench": bench,
    }
    out = ROOT / args.out
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(doc, indent=2) + "\n")
    t = doc["tests"]
    print(f"recorded: {t['passed']} passed, {t['failed']} failed, {t['ignored']} ignored across {len(suites)} suites "
          f"(exit {args.exit_code}) -> {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
