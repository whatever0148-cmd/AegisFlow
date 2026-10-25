#!/usr/bin/env python3
"""Deterministic executor for harness/TASKS.json.

Runs each task's `verify` command from the repository root in dependency order, records
exit code / wall time / output tail back into TASKS.json after every task (so the Obsidian
brain can show progress live), refreshes harness/STATE.json, then syncs the brain.

  python3 harness/run_tasks.py                 # all tasks
  python3 harness/run_tasks.py --only T03,T05  # a subset (dependencies are not re-run)
  python3 harness/run_tasks.py --no-sync       # skip the brain sync at the end

A task whose dependency failed in this run is marked `blocked` instead of being executed.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, List

ROOT = Path(__file__).resolve().parent.parent
TASKS = ROOT / "harness" / "TASKS.json"
STATE = ROOT / "harness" / "STATE.json"


def now_iso() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def child_env() -> Dict[str, str]:
    env = dict(os.environ)
    cargo_bin = str(Path.home() / ".cargo" / "bin")
    if cargo_bin not in env.get("PATH", "").split(":"):
        env["PATH"] = cargo_bin + ":" + env.get("PATH", "")
    return env


def topo_order(tasks: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    by_id = {t["id"]: t for t in tasks}
    done: List[str] = []
    pending = [t["id"] for t in tasks]
    while pending:
        progressed = False
        for tid in list(pending):
            if all(dep in done or dep not in by_id for dep in by_id[tid].get("deps", [])):
                done.append(tid)
                pending.remove(tid)
                progressed = True
        if not progressed:
            raise SystemExit(f"dependency cycle among {pending}")
    return [by_id[tid] for tid in done]


def git_commit() -> str:
    try:
        return subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True,
                              text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "uncommitted"


def save(doc: Dict[str, Any], current: str = None) -> None:
    TASKS.write_text(json.dumps(doc, indent=2) + "\n")
    tasks = doc["tasks"]
    counts = {s: sum(1 for t in tasks if t.get("status") == s) for s in ("passed", "failed", "blocked", "pending")}
    state = json.loads(STATE.read_text()) if STATE.exists() else {}
    state.update({
        "project": doc.get("project", "AegisFlow"),
        "phase": doc.get("phase", "phase-1"),
        "updated_at": now_iso(),
        "current_task": current,
        "tasks": {"total": len(tasks), **counts},
        "git_commit": git_commit(),
    })
    STATE.write_text(json.dumps(state, indent=2) + "\n")


def run(task: Dict[str, Any]) -> Dict[str, Any]:
    start = time.monotonic()
    try:
        proc = subprocess.run(task["verify"], shell=True, cwd=ROOT, env=child_env(), capture_output=True,
                              stdin=subprocess.DEVNULL, text=True, timeout=task.get("timeout_s", 1800))
        code, output = proc.returncode, (proc.stdout + proc.stderr)
    except subprocess.TimeoutExpired as exc:
        code = 124
        output = f"timeout after {exc.timeout}s\n" + ((exc.stdout or b"").decode(errors="replace") if isinstance(exc.stdout, bytes) else (exc.stdout or ""))
    duration_ms = (time.monotonic() - start) * 1000.0
    tail = output.strip().splitlines()[-25:]
    return {"exit_code": code, "duration_ms": round(duration_ms, 1), "finished_at": now_iso(), "output_tail": tail}


def main() -> int:
    ap = argparse.ArgumentParser(description="Run harness/TASKS.json verify commands")
    ap.add_argument("--only", help="comma-separated task ids")
    ap.add_argument("--no-sync", action="store_true")
    args = ap.parse_args()

    doc = json.loads(TASKS.read_text())
    selected = set(args.only.split(",")) if args.only else None
    failed_now: set = set()
    run_started = now_iso()
    for task in topo_order(doc["tasks"]):
        if selected and task["id"] not in selected:
            continue
        blockers = [d for d in task.get("deps", []) if d in failed_now]
        if blockers:
            task["status"] = "blocked"
            task["last_result"] = {"exit_code": None, "duration_ms": None, "finished_at": now_iso(),
                                   "output_tail": [f"blocked by failed dependency {', '.join(blockers)}"]}
            failed_now.add(task["id"])
            print(f"[{task['id']}] BLOCKED by {', '.join(blockers)}")
            save(doc)
            continue
        save(doc, current=task["id"])
        print(f"[{task['id']}] {task['title']} ...", flush=True)
        result = run(task)
        task["last_result"] = result
        task["status"] = "passed" if result["exit_code"] == 0 else "failed"
        if result["exit_code"] != 0:
            failed_now.add(task["id"])
            for line in result["output_tail"][-12:]:
                print(f"    | {line}")
        print(f"[{task['id']}] {task['status'].upper()} in {result['duration_ms']:.0f} ms", flush=True)
        save(doc)

    state = json.loads(STATE.read_text())
    state["last_run"] = {"started_at": run_started, "finished_at": now_iso(),
                         "selected": sorted(selected) if selected else "all",
                         "failed": sorted(failed_now)}
    STATE.write_text(json.dumps(state, indent=2) + "\n")
    if not args.no_sync:
        subprocess.run([sys.executable, str(ROOT / "harness" / "sync_obsidian_brain.py")], cwd=ROOT, check=False)
    return 1 if failed_now else 0


if __name__ == "__main__":
    sys.exit(main())
