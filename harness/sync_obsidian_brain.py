#!/usr/bin/env python3
"""Sync measured AegisFlow telemetry into the Obsidian brain vault (brain/).

Inputs (all produced by real runs, never edited by hand):
  artifacts/aegis_telemetry.json         cargo test summary + embedded bench (post-task hook)
  artifacts/aegis_phase1_telemetry.json  hardware benchmark (`cargo run --release -p aegis-bench`)
  harness/TASKS.json                     task verification results (harness/run_tasks.py)

Outputs:
  brain/**/*.md                          frontmatter (status, latency_ms, last_verified, git_commit)
                                         + a managed telemetry section per node
  brain/AegisFlow_Architecture.canvas    JSON Canvas: Sub-Agents -> Core -> Adapters -> Benchmarks
  brain/.obsidian/graph.json             colour groups (#agent #core #adapter #benchmark)
  brain/Telemetry-Log.md                 one row per benchmark run
  .kiro/specs/aegisflow-phase1/tasks.md  Kiro task list mirrored from TASKS.json

A node without data stays `status: pending` with `latency_ms: null`; values are copied from the
telemetry files or left empty. Python 3.9+, standard library only.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parent.parent
VAULT = ROOT / "brain"
ARTIFACTS = ROOT / "artifacts"
TASKS_FILE = ROOT / "harness" / "TASKS.json"
CANVAS = VAULT / "AegisFlow_Architecture.canvas"
GRAPH = VAULT / ".obsidian" / "graph.json"
LOG_NOTE = VAULT / "Telemetry-Log.md"
INDEX_NOTE = VAULT / "00-AegisFlow-Index.md"
KIRO_DIR = ROOT / ".kiro" / "specs" / "aegisflow-phase1"
OBSIDIAN_REGISTRY = Path.home() / "Library" / "Application Support" / "obsidian" / "obsidian.json"

# Paths whose changes make a benchmark result stale ("dirty"). Harness bookkeeping files are excluded.
SOURCE_PATHSPEC = [
    "crates", "policies", "models", "harness", ".claude", "Cargo.toml", "Cargo.lock",
    ":(exclude)harness/TASKS.json", ":(exclude)harness/STATE.json",
]

USER_START, USER_END = "<!-- aegis:user:start -->", "<!-- aegis:user:end -->"
TEL_START, TEL_END = "<!-- aegis:telemetry:start -->", "<!-- aegis:telemetry:end -->"
LOG_START, LOG_END = "<!-- aegis:log:start -->", "<!-- aegis:log:end -->"

NODE_W, NODE_H, ROW_H, COL_W = 440, 320, 370, 580

GROUPS: Dict[str, Dict[str, Any]] = {
    "agent": {"label": "Sub-Agents", "canvas": "6", "rgb": 0x9B59B6, "dir": "agents"},
    "core": {"label": "Core Engine Modules", "canvas": "5", "rgb": 0x00BCD4, "dir": "core"},
    "adapter": {"label": "Competition Adapters", "canvas": "2", "rgb": 0xFF9800, "dir": "adapters"},
    "benchmark": {"label": "Hardware Telemetry Proofs", "canvas": "4", "rgb": 0x4CAF50, "dir": "benchmarks"},
}

BADGE = {"verified": "🟢 verified", "failing": "🔴 failing", "target-missed": "🟡 target missed", "pending": "⚪ pending"}

NODES: List[Dict[str, Any]] = [
    # ---- Sub-agents -------------------------------------------------------------------------
    {"id": "Agent-Systems-Architect", "group": "agent",
     "summary": "Owns the harness contract: TASKS.json/STATE.json, Cedar policy design, Kiro specs and "
                "this vault's structure. Defines acceptance criteria; never writes metrics by hand.",
     "implements": [".claude/agents/systems-architect.md", "harness/TASKS.json", "policies/"]},
    {"id": "Agent-Core-Builder", "group": "agent",
     "summary": "Implements the Rust + Swift crates under crates/ and the CoreML model builder, keeping "
                "`cargo test --release` green.",
     "implements": [".claude/agents/core-builder.md", "crates/", "models/build_model.py"]},
    {"id": "Agent-QA-Reviewer", "group": "agent",
     "summary": "Runs the no-stub gate, the release test suite and the hardware benchmark. A task only "
                "counts as done when its verify command exits 0.",
     "implements": [".claude/agents/qa-reviewer.md", ".claude/hooks/verify_no_stubs.sh"]},
    {"id": "Agent-Telemetry-UI", "group": "agent",
     "summary": "Publishes measured telemetry into this vault (frontmatter, canvas, log) and the "
                "AG Grid / Bryntum row exports from the PayPal adapter.",
     "implements": [".claude/agents/telemetry-ui.md", "harness/sync_obsidian_brain.py",
                    ".claude/hooks/post_task_brain_sync.sh"]},
    # ---- Core engine ------------------------------------------------------------------------
    {"id": "Core-APFS-Clonefile", "group": "core", "crate": "aegis_storage",
     "summary": "Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` "
                "clones the live tree before an agent touches it. Branches that are dropped without a "
                "commit are detached and reclaimed automatically (Rust `Drop`).",
     "implements": ["crates/aegis-storage/src/lib.rs", "crates/aegis-storage/src/sys.rs"]},
    {"id": "Core-Memory-Snapshot", "group": "core", "crate": "aegis_storage",
     "summary": "Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live "
                "trees in one syscall; the previous live tree is kept as a CoW version snapshot that "
                "`restore()` can bring back. Phase 1 snapshots workspace state on disk only (no process-memory snapshots).",
     "implements": ["crates/aegis-storage/src/lib.rs"]},
    {"id": "Core-ANE-Classifier", "group": "core", "crate": "aegis_ane_scorer",
     "summary": "CoreML FP16 MLP (64 -> 32 -> 16 -> 1, sigmoid x 100) loaded with `.cpuAndNeuralEngine` "
                "through the Swift `@_cdecl(\"aegis_ane_score_workflow\")` bridge. Device placement is read "
                "back from `MLComputePlan`. Weights are trained on synthetic data labelled by the documented "
                "heuristic in models/build_model.py; this is not a validated production risk model.",
     "implements": ["crates/aegis-ane-scorer/swift/AegisANEBridge.swift", "crates/aegis-ane-scorer/src/lib.rs",
                    "models/build_model.py", "models/feature_schema.json"]},
    {"id": "Core-Cedar-Rules", "group": "core", "crate": "aegis_governance",
     "summary": "Deterministic governance: `cedar-policy` 4.x evaluates policies/aegis_rules.cedar (schema-"
                "validated), `regex` redaction flags unformatted sensitive fields, `sha2` + `ed25519-dalek` "
                "produce `c2pa:ed25519:` receipts.",
     "implements": ["crates/aegis-governance/src/lib.rs", "policies/aegis_rules.cedar", "policies/aegis.cedarschema"]},
    # ---- Adapters ---------------------------------------------------------------------------
    {"id": "Adapter-Nebius-Nemotron", "group": "adapter", "crate": "adapter_nebius_nemotron",
     "summary": "3-tier build-recovery router. Tier 0: local ANE score + Cedar. Tier 1: "
                "nvidia/Llama-3_3-Nemotron-Super-49B-v1 on Nebius. Tier 2: Tavily docs search + "
                "nvidia/Llama-3_1-Nemotron-Ultra-253B-v1. Every patch is verified inside a fresh APFS CoW branch.",
     "implements": ["crates/adapter-nebius-nemotron/src/lib.rs"]},
    {"id": "Adapter-PayPal-Commerce", "group": "adapter", "crate": "adapter_paypal",
     "summary": "PayPal Sandbox `/v2/checkout/orders` middleware: `[REDACTED]` formatting of sensitive strings, "
                "ed25519 receipt (`c2pa:ed25519:...`) in `custom_id`, Cedar budget rules, AG Grid + Bryntum telemetry rows.",
     "implements": ["crates/adapter-paypal/src/lib.rs"]},
    {"id": "Adapter-Switch-MCP", "group": "adapter", "crate": "adapter_switch_mcp",
     "summary": "MCP stdio server (branch / score / policy / commit / rollback / approval tools) and the Switch "
                "chat bridge that renders workspace-diff + team-approval cards for Mattermost and Slack webhooks.",
     "implements": ["crates/adapter-switch-mcp/src/mcp.rs", "crates/adapter-switch-mcp/src/cards.rs"]},
    {"id": "Adapter-Kiro-Specs", "group": "adapter",
     "summary": "Kiro spec-driven development: requirements (EARS), design and an implementation plan whose "
                "checkboxes mirror harness/TASKS.json verification results.",
     "implements": [".kiro/specs/aegisflow-phase1/requirements.md", ".kiro/specs/aegisflow-phase1/design.md",
                    ".kiro/specs/aegisflow-phase1/tasks.md"]},
    # ---- Benchmarks -------------------------------------------------------------------------
    {"id": "Bench-APFS-Latency", "group": "benchmark",
     "summary": "clonefile branch, renameatx_np RENAME_SWAP commit and rollback latency on generated "
                "fixtures of increasing size (APFS volume of this Mac).",
     "implements": ["crates/aegis-bench/src/main.rs"]},
    {"id": "Bench-ANE-FP16", "group": "benchmark",
     "summary": "CoreML FP16 inference latency per compute-unit setting, MLComputePlan per-operation device "
                "placement, and parity against a NumPy FP32 reference.",
     "implements": ["crates/aegis-bench/src/main.rs", "models/parity_vectors.json"]},
    {"id": "Bench-Cedar-Eval", "group": "benchmark",
     "summary": "Cedar `is_authorized` latency and the policy decision matrix (expected vs actual).",
     "implements": ["crates/aegis-bench/src/main.rs"]},
    {"id": "Bench-EndToEnd-Rollback", "group": "benchmark",
     "summary": "Full transaction: branch -> agent edit -> diff -> features -> ANE -> Cedar -> rollback or "
                "commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.",
     "implements": ["crates/aegis-bench/src/main.rs"]},
]

EDGES: List[Tuple[str, str, str]] = [
    ("Agent-Systems-Architect", "Core-Cedar-Rules", "designs policy"),
    ("Agent-Systems-Architect", "Core-Memory-Snapshot", "designs versioning"),
    ("Agent-Systems-Architect", "Adapter-Kiro-Specs", "writes specs"),
    ("Agent-Core-Builder", "Core-APFS-Clonefile", "builds"),
    ("Agent-Core-Builder", "Core-Memory-Snapshot", "builds"),
    ("Agent-Core-Builder", "Core-ANE-Classifier", "builds"),
    ("Agent-Core-Builder", "Core-Cedar-Rules", "builds"),
    ("Agent-QA-Reviewer", "Core-APFS-Clonefile", "tests"),
    ("Agent-QA-Reviewer", "Core-ANE-Classifier", "tests"),
    ("Agent-Telemetry-UI", "Adapter-PayPal-Commerce", "dashboard rows"),
    ("Core-APFS-Clonefile", "Adapter-Nebius-Nemotron", "verifies patches in CoW branch"),
    ("Core-APFS-Clonefile", "Adapter-Switch-MCP", "branch tools"),
    ("Core-Memory-Snapshot", "Adapter-Switch-MCP", "commit / restore"),
    ("Core-ANE-Classifier", "Adapter-Nebius-Nemotron", "tier-0 risk"),
    ("Core-ANE-Classifier", "Adapter-PayPal-Commerce", "order risk"),
    ("Core-Cedar-Rules", "Adapter-PayPal-Commerce", "budget rules"),
    ("Core-Cedar-Rules", "Adapter-Switch-MCP", "approval gate"),
    ("Core-Cedar-Rules", "Adapter-Nebius-Nemotron", "commit gate"),
    ("Core-APFS-Clonefile", "Bench-APFS-Latency", "measured by"),
    ("Core-Memory-Snapshot", "Bench-APFS-Latency", "measured by"),
    ("Core-ANE-Classifier", "Bench-ANE-FP16", "measured by"),
    ("Core-Cedar-Rules", "Bench-Cedar-Eval", "measured by"),
    ("Adapter-Nebius-Nemotron", "Bench-EndToEnd-Rollback", "tier-0 routing"),
    ("Adapter-PayPal-Commerce", "Bench-EndToEnd-Rollback", "receipt signing"),
    ("Adapter-Switch-MCP", "Bench-EndToEnd-Rollback", "MCP round-trip"),
    ("Agent-QA-Reviewer", "Bench-EndToEnd-Rollback", "gates on"),
    ("Agent-Telemetry-UI", "Bench-ANE-FP16", "publishes"),
]

# Task ids each agent owns (mirrors the `owner` field in TASKS.json).
AGENT_ROLE = {
    "Agent-Systems-Architect": "systems-architect",
    "Agent-Core-Builder": "core-builder",
    "Agent-QA-Reviewer": "qa-reviewer",
    "Agent-Telemetry-UI": "telemetry-ui",
}


# ---------------------------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------------------------
def now_iso() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def load_json(path: Path) -> Optional[Any]:
    try:
        return json.loads(path.read_text())
    except FileNotFoundError:
        return None
    except json.JSONDecodeError as exc:
        print(f"warning: {path.relative_to(ROOT)} is not valid JSON ({exc})", file=sys.stderr)
        return None


def dig(obj: Any, *keys: Any, default: Any = None) -> Any:
    for key in keys:
        if isinstance(obj, dict) and key in obj:
            obj = obj[key]
        elif isinstance(obj, list) and isinstance(key, int) and -len(obj) <= key < len(obj):
            obj = obj[key]
        else:
            return default
    return obj


def git_info() -> Tuple[str, bool]:
    try:
        sha = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"],
                             capture_output=True, text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "uncommitted", True
    porcelain = subprocess.run(["git", "-C", str(ROOT), "status", "--porcelain", "--", *SOURCE_PATHSPEC],
                               capture_output=True, text=True).stdout.strip()
    return sha, bool(porcelain)


def num(value: Any) -> str:
    if value is None:
        return "—"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        if abs(value) >= 100:
            return f"{value:.1f}"
        if abs(value) >= 1:
            return f"{value:.3f}"
        return f"{value:.4f}"
    return str(value)


def fmt_stats(stats: Any, unit: str) -> str:
    if not isinstance(stats, dict) or stats.get("p50") is None:
        return "—"
    return f"p50 {num(stats['p50'])} {unit} · p95 {num(stats.get('p95'))} {unit} · n={stats.get('n', '?')}"


def p50(stats: Any) -> Optional[float]:
    value = dig(stats, "p50")
    return float(value) if isinstance(value, (int, float)) else None


def note_rel(node: Dict[str, Any]) -> str:
    return f"{GROUPS[node['group']]['dir']}/{node['id']}.md"


def links_for(node_id: str) -> Tuple[List[Tuple[str, str]], List[Tuple[str, str]]]:
    upstream = [(a, label) for a, b, label in EDGES if b == node_id]
    downstream = [(b, label) for a, b, label in EDGES if a == node_id]
    return upstream, downstream


def between(text: str, start: str, end: str) -> Optional[str]:
    i, j = text.find(start), text.find(end)
    if i == -1 or j == -1 or j < i:
        return None
    return text[i + len(start):j]


# ---------------------------------------------------------------------------------------------
# node state from telemetry
# ---------------------------------------------------------------------------------------------
class Ctx:
    def __init__(self) -> None:
        self.tel = load_json(ARTIFACTS / "aegis_telemetry.json") or {}
        bench_file = load_json(ARTIFACTS / "aegis_phase1_telemetry.json")
        self.bench = bench_file or self.tel.get("bench") or {}
        tasks_doc = load_json(TASKS_FILE) or {}
        self.tasks: List[Dict[str, Any]] = tasks_doc.get("tasks", []) if isinstance(tasks_doc, dict) else []
        self.git_sha, self.git_dirty = git_info()
        bench_git = dig(self.bench, "git", "commit")
        if bench_git:
            self.commit = bench_git + ("-dirty" if dig(self.bench, "git", "dirty") else "")
        else:
            self.commit = self.git_sha + ("-dirty" if self.git_dirty else "")
        self.bench_time = self.bench.get("generated_at")
        self.test_time = dig(self.tel, "tests", "finished_at")
        self.targets = self.bench.get("targets", {})

    def crate_tests(self, crate: Optional[str]) -> Tuple[Optional[str], str]:
        if not crate:
            return None, "n/a"
        suites = [s for s in (dig(self.tel, "tests", "suites", default=[]) or []) if s.get("crate") == crate]
        if not suites:
            if dig(self.tel, "tests", "compiled") is False:
                return "failing", "workspace did not compile (see artifacts/last_test.log)"
            return None, "no `cargo test --release` run recorded yet"
        passed = sum(int(s.get("passed", 0)) for s in suites)
        failed = sum(int(s.get("failed", 0)) for s in suites)
        status = "failing" if failed else ("verified" if passed else "pending")
        return status, f"{passed} passed, {failed} failed ({len(suites)} suites)"

    def fixture(self, name: str) -> Optional[Dict[str, Any]]:
        for fx in dig(self.bench, "apfs", "fixtures", default=[]) or []:
            if fx.get("name") == name:
                return fx
        return None


class State:
    def __init__(self, status: str = "pending", latency_ms: Optional[float] = None,
                 rows: Optional[List[Tuple[str, str]]] = None, verified_at: Optional[str] = None,
                 notes: Optional[List[str]] = None) -> None:
        self.status = status
        self.latency_ms = latency_ms
        # Keep the caller's list object: state_for() keeps appending to it after construction.
        self.rows = rows if rows is not None else []
        self.verified_at = verified_at
        self.notes = notes if notes is not None else []


def worst(*statuses: Optional[str]) -> str:
    order = ["failing", "target-missed", "pending", "verified"]
    present = [s for s in statuses if s]
    if not present:
        return "pending"
    return min(present, key=order.index)


def target_status(value: Optional[float], target: Optional[float]) -> Optional[str]:
    if value is None:
        return None
    if target is None:
        return "verified"
    return "verified" if value < target else "target-missed"


def state_for(node: Dict[str, Any], ctx: Ctx) -> State:
    nid = node["id"]
    group = node["group"]
    if group == "agent":
        return agent_state(nid, ctx)
    test_status, test_text = ctx.crate_tests(node.get("crate"))
    rows: List[Tuple[str, str]] = []
    if node.get("crate"):
        rows.append(("cargo test --release", test_text))
    bench = ctx.bench
    small = ctx.fixture("small")
    st = State(rows=rows)

    if nid == "Core-APFS-Clonefile":
        for fx in dig(bench, "apfs", "fixtures", default=[]) or []:
            rows.append((f"clonefile · {fx.get('name')} ({fx.get('files')} files)", fmt_stats(fx.get("clonefile_ms"), "ms")))
        slope = dig(bench, "apfs", "clonefile_us_per_entry")
        if slope is not None:
            rows.append(("clonefile cost per tree entry (fit)", f"{num(slope)} µs"))
        st.latency_ms = p50(dig(small, "clonefile_ms"))
        st.status = worst(test_status, "verified" if st.latency_ms is not None else None)
    elif nid == "Core-Memory-Snapshot":
        for fx in dig(bench, "apfs", "fixtures", default=[]) or []:
            rows.append((f"renameatx_np swap · {fx.get('name')}", fmt_stats(fx.get("renameatx_swap_ms"), "ms")))
        st.latency_ms = p50(dig(small, "renameatx_swap_ms"))
        st.status = worst(test_status, "verified" if st.latency_ms is not None else None)
    elif nid == "Core-ANE-Classifier":
        ane = bench.get("ane", {})
        rows.append(("inference · cpuAndNeuralEngine", fmt_stats(dig(ane, "runs", "cpu_and_neural_engine"), "ms")))
        rows.append(("parity max abs diff vs NumPy FP32 (points)", num(dig(ane, "parity", "max_abs_diff"))))
        if ane.get("placement_note"):
            rows.append(("device placement (MLComputePlan)", ane["placement_note"]))
        st.latency_ms = p50(dig(ane, "runs", "cpu_and_neural_engine"))
        st.status = worst(test_status, "verified" if st.latency_ms is not None else None)
    elif nid == "Core-Cedar-Rules":
        cedar = bench.get("cedar", {})
        rows.append(("is_authorized", fmt_stats(cedar.get("is_authorized_us"), "µs")))
        rows.append(("decision matrix", f"{num(cedar.get('decision_matrix_passed'))}/{num(cedar.get('decision_matrix_total'))} as expected"))
        us = p50(cedar.get("is_authorized_us"))
        st.latency_ms = us / 1000.0 if us is not None else None
        matrix_ok = cedar.get("decision_matrix_total") is not None and cedar.get("decision_matrix_passed") == cedar.get("decision_matrix_total")
        st.status = worst(test_status, ("verified" if matrix_ok else "failing") if cedar else None)
    elif nid == "Adapter-Nebius-Nemotron":
        router = bench.get("nebius_router", {})
        rows.append(("tier-0 route (ANE + Cedar)", fmt_stats(router.get("tier0_route_us"), "µs")))
        rows.append(("live Nebius / Tavily calls", live_text(router.get("live_calls"))))
        us = p50(router.get("tier0_route_us"))
        st.latency_ms = us / 1000.0 if us is not None else None
        st.status = worst(test_status, "verified" if us is not None else None)
    elif nid == "Adapter-PayPal-Commerce":
        pp = bench.get("paypal_receipt", {})
        rows.append(("prepare order (redact+sign+verify+Cedar)", fmt_stats(pp.get("prepare_order_us"), "µs")))
        rows.append(("ed25519 sign", fmt_stats(pp.get("ed25519_sign_us"), "µs")))
        rows.append(("Sandbox submission", live_text(pp.get("sandbox_submission"))))
        us = p50(pp.get("prepare_order_us"))
        st.latency_ms = us / 1000.0 if us is not None else None
        st.status = worst(test_status, "verified" if us is not None else None)
    elif nid == "Adapter-Switch-MCP":
        mcp = bench.get("mcp", {})
        rows.append(("tools/call round-trip (in-process)", fmt_stats(mcp.get("tools_call_roundtrip_us"), "µs")))
        rows.append(("chat webhooks", live_text(mcp.get("webhooks"))))
        us = p50(mcp.get("tools_call_roundtrip_us"))
        st.latency_ms = us / 1000.0 if us is not None else None
        st.status = worst(test_status, "verified" if us is not None else None)
    elif nid == "Adapter-Kiro-Specs":
        files = [KIRO_DIR / n for n in ("requirements.md", "design.md", "tasks.md")]
        present = [f for f in files if f.exists() and f.stat().st_size > 0]
        rows.append(("spec files", f"{len(present)}/3 present"))
        synced = sum(1 for t in ctx.tasks if t.get("status") == "passed")
        rows.append(("tasks verified by harness", f"{synced}/{len(ctx.tasks)}"))
        st.status = "verified" if len(present) == 3 else "pending"
        st.verified_at = now_iso() if len(present) == 3 else None
    elif nid == "Bench-APFS-Latency":
        tgt = ctx.targets.get("clonefile_branch_ms")
        rows.clear()
        for fx in dig(bench, "apfs", "fixtures", default=[]) or []:
            name = f"{fx.get('name')} · {fx.get('files')} files / {num((fx.get('bytes') or 0) / 1048576.0)} MiB"
            rows.append((name + " · clonefile", fmt_stats(fx.get("clonefile_ms"), "ms")))
            rows.append((name + " · RENAME_SWAP", fmt_stats(fx.get("renameatx_swap_ms"), "ms")))
            rows.append((name + " · rollback (detach)", fmt_stats(fx.get("rollback_detach_ms"), "ms")))
            rows.append((name + " · rollback (reclaim)", fmt_stats(fx.get("rollback_reclaim_ms"), "ms")))
        if small:
            checks = [
                target_status(p50(small.get("clonefile_ms")), tgt),
                target_status(p50(small.get("renameatx_swap_ms")), ctx.targets.get("renameatx_swap_ms")),
                target_status(p50(small.get("rollback_detach_ms")), ctx.targets.get("rollback_ms")),
            ]
            st.status = worst(*checks)
            st.latency_ms = p50(small.get("clonefile_ms"))
            st.notes.append(f"Targets are checked on the `small` fixture ({small.get('files')} files): "
                            f"clonefile < {num(tgt)} ms, swap < {num(ctx.targets.get('renameatx_swap_ms'))} ms, "
                            f"rollback < {num(ctx.targets.get('rollback_ms'))} ms.")
    elif nid == "Bench-ANE-FP16":
        ane = bench.get("ane", {})
        rows.clear()
        for key, label in (("cpu_and_neural_engine", "cpuAndNeuralEngine"), ("cpu_only", "cpuOnly"), ("all", "all")):
            rows.append((f"inference · {label}", fmt_stats(dig(ane, "runs", key), "ms")))
        plan = ane.get("compute_plan", {})
        pref = plan.get("preferred", {})
        if plan:
            rows.append(("MLComputePlan preferred device",
                         f"ANE {num(pref.get('neural_engine'))} · CPU {num(pref.get('cpu'))} · GPU {num(pref.get('gpu'))} "
                         f"of {num(plan.get('operations'))} ops"))
        rows.append(("Neural Engine cores (MLNeuralEngineComputeDevice)", num(ane.get("neural_engine_cores"))))
        rows.append(("parity max abs diff vs NumPy FP32 (points)", num(dig(ane, "parity", "max_abs_diff"))))
        batch = ane.get("batch", {})
        if batch:
            size = batch.get("batch_size")
            bpref = dig(batch, "compute_plan", "preferred", default={}) or {}
            rows.append((f"batch model ({num(size)} ops/prediction) MLComputePlan",
                         f"ANE {num(bpref.get('neural_engine'))} · CPU {num(bpref.get('cpu'))} · GPU {num(bpref.get('gpu'))} "
                         f"of {num(dig(batch, 'compute_plan', 'operations'))} ops"))
            for key, label in (("cpu_and_neural_engine", "cpuAndNeuralEngine"), ("cpu_only", "cpuOnly")):
                run = dig(batch, "runs", key, default={}) or {}
                rows.append((f"batch prediction · {label}",
                             f"{fmt_stats(run.get('batch_ms'), 'ms')} · {num(run.get('amortized_us_per_operation_p50'))} µs/op"))
        st.latency_ms = p50(dig(ane, "runs", "cpu_and_neural_engine"))
        if st.latency_ms is not None:
            on_ane = plan.get("operations") and pref.get("neural_engine") == plan.get("operations")
            st.status = worst(target_status(st.latency_ms, ctx.targets.get("ane_inference_ms")),
                              "verified" if on_ane else "target-missed")
            if ane.get("placement_note"):
                st.notes.append(ane["placement_note"])
            if not on_ane:
                st.notes.append("The online (one operation per prediction) model is not on the Neural Engine, so the "
                                "'inference on the ANE' target is marked missed even though its latency target is met.")
    elif nid == "Bench-Cedar-Eval":
        cedar = bench.get("cedar", {})
        rows.clear()
        rows.append(("is_authorized", fmt_stats(cedar.get("is_authorized_us"), "µs")))
        rows.append(("build request + authorize", fmt_stats(cedar.get("build_request_and_authorize_us"), "µs")))
        for case in cedar.get("decision_matrix", []) or []:
            ok = "✔" if case.get("expected") == case.get("actual") else "✘"
            rows.append((f"{ok} {case.get('case')}", f"{case.get('actual')} ({', '.join(case.get('policies') or []) or 'default deny'})"))
        us = p50(cedar.get("is_authorized_us"))
        st.latency_ms = us / 1000.0 if us is not None else None
        if us is not None:
            ok = cedar.get("decision_matrix_passed") == cedar.get("decision_matrix_total")
            st.status = "verified" if ok else "failing"
    elif nid == "Bench-EndToEnd-Rollback":
        e2e = bench.get("end_to_end", {})
        pp = bench.get("paypal_receipt", {})
        rows.clear()
        rows.append(("rollback path (branch→edit→diff→ANE→Cedar deny→rollback)", fmt_stats(e2e.get("rollback_path_ms"), "ms")))
        rows.append(("commit path (branch→edit→diff→ANE→Cedar allow→swap)", fmt_stats(e2e.get("commit_path_ms"), "ms")))
        for stage, value in (e2e.get("stage_p50_us") or {}).items():
            rows.append((f"stage p50 · {stage}", f"{num(value)} µs"))
        rows.append(("live tree byte-identical after every rollback", num(e2e.get("live_integrity_verified"))))
        rows.append(("PayPal prepare + ed25519 receipt", fmt_stats(pp.get("prepare_order_us"), "µs")))
        rows.append(("ed25519 verify", fmt_stats(pp.get("ed25519_verify_us"), "µs")))
        st.latency_ms = p50(e2e.get("rollback_path_ms"))
        if st.latency_ms is not None:
            st.status = "verified" if e2e.get("live_integrity_verified") else "failing"

    if st.latency_ms is not None or st.status != "pending":
        st.verified_at = st.verified_at or ctx.bench_time or ctx.test_time
    return st


def live_text(info: Any) -> str:
    if not isinstance(info, dict):
        return "—"
    if info.get("attempted"):
        return f"attempted · {info.get('result', 'see telemetry')}"
    return f"not called · {info.get('reason', 'no reason recorded')}"


def agent_state(nid: str, ctx: Ctx) -> State:
    role = AGENT_ROLE[nid]
    owned = [t for t in ctx.tasks if t.get("owner") == role]
    rows = []
    total_ms = 0.0
    timed = False
    last = None
    statuses = []
    for task in owned:
        res = task.get("last_result") or {}
        statuses.append(task.get("status", "pending"))
        if isinstance(res.get("duration_ms"), (int, float)):
            total_ms += float(res["duration_ms"])
            timed = True
        if res.get("finished_at") and (last is None or res["finished_at"] > last):
            last = res["finished_at"]
        rows.append((f"{task.get('id')} · {task.get('title')}",
                     f"{task.get('status', 'pending')}" + (f" · {num(res.get('duration_ms'))} ms" if res.get("duration_ms") is not None else "")))
    if not owned:
        status = "pending"
    elif any(s == "failed" for s in statuses):
        status = "failing"
    elif all(s == "passed" for s in statuses):
        status = "verified"
    else:
        status = "pending"
    st = State(status=status, latency_ms=round(total_ms, 3) if timed else None, rows=rows, verified_at=last)
    st.notes.append("`latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).")
    return st


# ---------------------------------------------------------------------------------------------
# writers
# ---------------------------------------------------------------------------------------------
def frontmatter(node: Dict[str, Any], st: State, ctx: Ctx) -> str:
    lines = ["---", "tags:", f"  - {node['group']}", f"status: {st.status}",
             f"latency_ms: {'null' if st.latency_ms is None else repr(round(st.latency_ms, 6))}",
             f"last_verified: {st.verified_at or 'null'}",
             f"git_commit: {ctx.commit if st.verified_at else 'null'}"]
    if node.get("crate"):
        lines.append(f"crate: {node['crate']}")
    lines.append("---")
    return "\n".join(lines)


def telemetry_block(st: State, ctx: Ctx) -> str:
    out = [TEL_START, "## Live telemetry",
           f"_Synced {now_iso()} · bench {ctx.bench_time or 'not run yet'} · tests {ctx.test_time or 'not run yet'} · commit `{ctx.commit}`_",
           ""]
    if st.rows:
        out += ["| Metric | Measured |", "|---|---|"]
        out += [f"| {k} | {v} |" for k, v in st.rows]
    else:
        out.append("No measurements recorded yet.")
    for note in st.notes:
        out += ["", f"> {note}"]
    out.append(TEL_END)
    return "\n".join(out)


def write_note(node: Dict[str, Any], st: State, ctx: Ctx) -> None:
    path = VAULT / note_rel(node)
    path.parent.mkdir(parents=True, exist_ok=True)
    user_section = "\n_Your notes here are preserved across syncs._\n"
    if path.exists():
        kept = between(path.read_text(), USER_START, USER_END)
        if kept is not None:
            user_section = kept
    up, down = links_for(node["id"])
    body = [frontmatter(node, st, ctx), "", f"# {node['id']}", "",
            f"**Status:** {BADGE.get(st.status, st.status)} · **p50:** "
            f"{num(st.latency_ms) + ' ms' if st.latency_ms is not None else '—'} · **commit:** `{ctx.commit}`", "",
            f"> {node['summary']}", ""]
    if node.get("implements"):
        body.append("**Implemented in:** " + " · ".join(f"`{p}`" for p in node["implements"]))
        body.append("")
    if up:
        body.append("**Upstream:** " + " · ".join(f"[[{n}]] ({label})" for n, label in up))
    if down:
        body.append("**Downstream:** " + " · ".join(f"[[{n}]] ({label})" for n, label in down))
    body += ["", "Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]", "",
             telemetry_block(st, ctx), "", "## Notes", USER_START + user_section + USER_END, ""]
    path.write_text("\n".join(body))


def summary_markdown(states: Dict[str, State], ctx: Ctx) -> str:
    tests = ctx.tel.get("tests") or {}
    lines = ["# AegisFlow · live telemetry",
             f"commit `{ctx.commit}` · bench `{ctx.bench_time or 'not run'}` · "
             f"tests {num(tests.get('passed'))} passed / {num(tests.get('failed'))} failed", "",
             "| Node | Status | p50 |", "|---|---|---|"]
    for node in NODES:
        st = states[node["id"]]
        if node["group"] == "agent":
            continue
        lat = f"{num(st.latency_ms)} ms" if st.latency_ms is not None else "—"
        lines.append(f"| [[{node['id']}]] | {BADGE.get(st.status, st.status)} | {lat} |")
    return "\n".join(lines)


def write_index(states: Dict[str, State], ctx: Ctx) -> None:
    lines = ["---", "tags:", "  - aegisflow", f"last_synced: {now_iso()}", f"git_commit: {ctx.commit}", "---", "",
             "# AegisFlow brain", "",
             "Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] "
             "or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).",
             "", "Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; "
             "empty means not measured yet.", ""]
    for gkey, g in GROUPS.items():
        lines += [f"## {g['label']}", "", "| Node | Status | latency_ms | last_verified |", "|---|---|---|---|"]
        for node in NODES:
            if node["group"] != gkey:
                continue
            st = states[node["id"]]
            lines.append(f"| [[{node['id']}]] | {BADGE.get(st.status, st.status)} | {num(st.latency_ms)} | {st.verified_at or '—'} |")
        lines.append("")
    lines += ["See also: [[Telemetry-Log]]", ""]
    INDEX_NOTE.write_text("\n".join(lines))


def append_log(ctx: Ctx) -> bool:
    """Adds the row for the current benchmark run, or refreshes it when more data (e.g. test results)
    arrived later. Returns True when the table changed."""
    existing = LOG_NOTE.read_text() if LOG_NOTE.exists() else ""
    kept = between(existing, LOG_START, LOG_END) or ""
    rows: List[str] = [ln for ln in kept.strip().splitlines() if ln.startswith("| 2")]
    changed = False
    if ctx.bench_time:
        b = ctx.bench
        small = ctx.fixture("small") or {}
        tests = ctx.tel.get("tests") or {}
        row = "| {t} | `{c}` | {tp}/{tf} | {clone} | {swap} | {rb} | {ane} | {cedar} | {e2e} |".format(
            t=ctx.bench_time, c=ctx.commit, tp=num(tests.get("passed")), tf=num(tests.get("failed")),
            clone=num(p50(small.get("clonefile_ms"))), swap=num(p50(small.get("renameatx_swap_ms"))),
            rb=num(p50(small.get("rollback_detach_ms"))),
            ane=num(p50(dig(b, "ane", "runs", "cpu_and_neural_engine"))),
            cedar=num(p50(dig(b, "cedar", "is_authorized_us"))),
            e2e=num(p50(dig(b, "end_to_end", "rollback_path_ms"))))
        same_run = [i for i, ln in enumerate(rows) if ln.startswith(f"| {ctx.bench_time} ")]
        if not same_run:
            rows.append(row)
            changed = True
        elif rows[same_run[0]] != row:
            rows[same_run[0]] = row
            changed = True
    added = changed
    header = ["---", "tags:", "  - benchmark-log", "---", "", "# Telemetry log", "",
              "One row per `aegis-bench` run (p50 values; `small` APFS fixture). Source: `artifacts/aegis_phase1_telemetry.json`.",
              "", "Back to [[00-AegisFlow-Index]]", "", LOG_START,
              "| bench generated_at | commit | tests pass/fail | clonefile ms | RENAME_SWAP ms | rollback ms | ANE ms | Cedar µs | e2e rollback ms |",
              "|---|---|---|---|---|---|---|---|---|"]
    LOG_NOTE.write_text("\n".join(header + rows + [LOG_END, ""]))
    return added


def write_graph() -> None:
    GRAPH.parent.mkdir(parents=True, exist_ok=True)
    graph = {
        "collapse-filter": False, "search": "", "showTags": False, "showAttachments": False,
        "hideUnresolved": True, "showOrphans": True, "collapse-color-groups": False,
        "collapse-display": True, "showArrow": True, "textFadeMultiplier": 0, "nodeSizeMultiplier": 1.2,
        "lineSizeMultiplier": 1, "collapse-forces": True, "centerStrength": 0.5, "repelStrength": 12,
        "linkStrength": 1, "linkDistance": 220, "scale": 1, "close": False,
    }
    existing = load_json(GRAPH)
    if isinstance(existing, dict):
        graph.update(existing)
    graph["colorGroups"] = [{"query": f"tag:#{key}", "color": {"a": 1, "rgb": g["rgb"]}} for key, g in GROUPS.items()]
    GRAPH.write_text(json.dumps(graph, indent=2) + "\n")


def write_canvas(states: Dict[str, State], ctx: Ctx) -> None:
    ours: List[Dict[str, Any]] = []
    for col, (gkey, g) in enumerate(GROUPS.items()):
        members = [n for n in NODES if n["group"] == gkey]
        x = col * COL_W
        ours.append({"id": f"group-{gkey}", "type": "group", "label": g["label"], "x": x - 30, "y": -70,
                     "width": NODE_W + 60, "height": len(members) * ROW_H + 70, "color": g["canvas"]})
        for row, node in enumerate(members):
            color = "1" if states[node["id"]].status == "failing" else g["canvas"]
            ours.append({"id": node["id"], "type": "file", "file": note_rel(node), "x": x, "y": row * ROW_H,
                         "width": NODE_W, "height": NODE_H, "color": color})
    ours.append({"id": "live-telemetry", "type": "text", "text": summary_markdown(states, ctx),
                 "x": -30, "y": -620, "width": 3 * COL_W + NODE_W + 60, "height": 520})
    edges = [{"id": f"aegis-e-{a}-{b}", "fromNode": a, "fromSide": "right", "toNode": b, "toSide": "left",
              "toEnd": "arrow", "label": label} for a, b, label in EDGES]

    existing = load_json(CANVAS)
    if isinstance(existing, dict):
        prev = {n.get("id"): n for n in existing.get("nodes", []) if isinstance(n, dict)}
        our_ids = {n["id"] for n in ours}
        for node in ours:
            old = prev.get(node["id"])
            if old:  # keep the user's layout, refresh content
                for key in ("x", "y", "width", "height"):
                    if key in old:
                        node[key] = old[key]
        extra_nodes = [n for nid, n in prev.items() if nid not in our_ids]
        extra_edges = [e for e in existing.get("edges", []) if isinstance(e, dict) and not str(e.get("id", "")).startswith("aegis-e-")]
        ours += extra_nodes
        edges += extra_edges
    CANVAS.write_text(json.dumps({"nodes": ours, "edges": edges}, indent=2, ensure_ascii=False) + "\n")


def write_kiro_tasks(ctx: Ctx) -> None:
    if not KIRO_DIR.exists():
        return
    lines = ["# Implementation Plan", "",
             "_Generated by harness/sync_obsidian_brain.py from harness/TASKS.json. A box is ticked only when the "
             "task's verify command last exited 0._", ""]
    for i, task in enumerate(ctx.tasks, start=1):
        box = "x" if task.get("status") == "passed" else " "
        lines.append(f"- [{box}] {i}. {task.get('title')} (`{task.get('id')}`, owner: {task.get('owner')})")
        for crit in task.get("acceptance", []):
            lines.append(f"  - {crit}")
        lines.append(f"  - Verify: `{task.get('verify')}`")
        reqs = task.get("requirements") or []
        if reqs:
            lines.append(f"  - _Requirements: {', '.join(reqs)}_")
        lines.append("")
    (KIRO_DIR / "tasks.md").write_text("\n".join(lines))


# ---------------------------------------------------------------------------------------------
# checks + Obsidian launch
# ---------------------------------------------------------------------------------------------
def check_vault() -> List[str]:
    problems: List[str] = []
    for node in NODES:
        path = VAULT / note_rel(node)
        if not path.exists():
            problems.append(f"missing note {note_rel(node)}")
            continue
        text = path.read_text()
        if not text.startswith("---\n"):
            problems.append(f"{note_rel(node)}: no YAML frontmatter")
        for key in ("status:", "latency_ms:", "last_verified:", "git_commit:", f"  - {node['group']}"):
            if key not in text.split("\n---", 1)[0] + "\n":
                problems.append(f"{note_rel(node)}: frontmatter lacks `{key.strip()}`")
    canvas = load_json(CANVAS)
    if not isinstance(canvas, dict):
        problems.append("canvas missing or invalid JSON")
    else:
        ids = {n.get("id") for n in canvas.get("nodes", [])}
        for n in canvas.get("nodes", []):
            if n.get("type") not in ("text", "file", "link", "group"):
                problems.append(f"canvas node {n.get('id')} has invalid type {n.get('type')}")
            for key in ("id", "x", "y", "width", "height"):
                if key not in n:
                    problems.append(f"canvas node {n.get('id')} lacks {key}")
            if n.get("type") == "file" and not (VAULT / n.get("file", "")).exists():
                problems.append(f"canvas file node {n.get('id')} points to missing {n.get('file')}")
        for e in canvas.get("edges", []):
            if e.get("fromNode") not in ids or e.get("toNode") not in ids:
                problems.append(f"canvas edge {e.get('id')} references unknown node")
        for node in NODES:
            if node["id"] not in ids:
                problems.append(f"canvas lacks node {node['id']}")
    graph = load_json(GRAPH)
    queries = [g.get("query") for g in (graph or {}).get("colorGroups", [])] if isinstance(graph, dict) else []
    for key in GROUPS:
        if f"tag:#{key}" not in queries:
            problems.append(f"graph.json lacks colour group tag:#{key}")
    return problems


def vault_registered() -> bool:
    reg = load_json(OBSIDIAN_REGISTRY)
    if not isinstance(reg, dict):
        return False
    return any(str(v.get("path", "")).rstrip("/") == str(VAULT) for v in (reg.get("vaults") or {}).values())


def open_obsidian() -> None:
    subprocess.run(["open", "-a", "Obsidian", str(VAULT)], check=False)
    for _ in range(20):
        if vault_registered():
            break
        time.sleep(0.5)
    if vault_registered():
        uri = "obsidian://open?path=" + urllib.parse.quote(str(CANVAS))
        subprocess.run(["open", uri], check=False)
        print(f"obsidian: vault registered and opened ({VAULT}); canvas requested")
    else:
        print("obsidian: launched with `open -a Obsidian brain`, but the vault is not in Obsidian's registry yet; "
              "use 'Open folder as vault' on brain/ once and later syncs will appear live.")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--open", action="store_true", help="open the vault in Obsidian after syncing")
    ap.add_argument("--check", action="store_true", help="validate vault structure only (no writes)")
    args = ap.parse_args()

    if args.check:
        problems = check_vault()
        for p in problems:
            print(f"check: {p}")
        print("check: OK" if not problems else f"check: {len(problems)} problem(s)")
        return 1 if problems else 0

    ctx = Ctx()
    VAULT.mkdir(parents=True, exist_ok=True)
    states = {node["id"]: state_for(node, ctx) for node in NODES}
    for node in NODES:
        write_note(node, states[node["id"]], ctx)
    write_index(states, ctx)
    added = append_log(ctx)
    write_graph()
    write_canvas(states, ctx)
    write_kiro_tasks(ctx)

    counts: Dict[str, int] = {}
    for st in states.values():
        counts[st.status] = counts.get(st.status, 0) + 1
    print("brain sync: " + ", ".join(f"{k}={v}" for k, v in sorted(counts.items()))
          + f" · commit {ctx.commit} · log {'updated' if added else 'unchanged'}")
    problems = check_vault()
    for p in problems:
        print(f"check: {p}")
    if args.open:
        open_obsidian()
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
