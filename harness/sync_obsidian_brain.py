#!/usr/bin/env python3
"""Sync measured AegisFlow telemetry into the Obsidian brain vault (brain/).

Inputs (all produced by real runs, never edited by hand):
  artifacts/aegis_telemetry.json         cargo test summary + embedded bench (post-task hook)
  artifacts/aegis_phase1_telemetry.json  hardware benchmark (`cargo run --release -p aegis-bench`)
  models/model_card.json                 architecture facts of the CoreML model (written by the build)
  harness/TASKS.json                     task verification results (harness/run_tasks.py)

Outputs:
  brain/**/*.md                          frontmatter (status, latency_ms, last_verified, git_commit)
                                         + a managed telemetry section per node
  brain/AegisFlow_Architecture.canvas    JSON Canvas: Sub-Agents -> Core -> Adapters -> Benchmarks
                                         (node positions and sizes the user set are kept)
  brain/.obsidian/graph.json             colour groups (#agent #core #adapter #benchmark)
  brain/Telemetry-Log.md                 one row per benchmark run
  .kiro/specs/aegisflow-phase{1,2}/tasks.md  Kiro task lists mirrored from TASKS.json (by `phase`)

Pass/fail verdicts are copied from the benchmark's `targets_summary` / `informational` rows; this
script never re-judges a measurement. A node without data stays `status: pending` with
`latency_ms: null`; values are copied from the telemetry files or left empty. A node whose gated
rows exist but none match it stays `pending` (it is never `verified` just because a latency exists).
A failed live-tree integrity row marks the APFS nodes `failing`. A preliminary run (reduced scale,
busy machine; `run_conditions` in the artifact) is labelled PRELIMINARY in every note, the canvas
card and the log.

Every sync compares the vault before and after writing: canvas node positions and sizes, user
nodes and edges, and every `<!-- aegis:user:start -->` block (notes and Kiro tasks.md) must be
unchanged, otherwise the sync exits 1. `--require-phase2` additionally requires a phase-2 artifact
rendered into the notes, the canvas card and the log (gated table and targets_note).

  python3 harness/sync_obsidian_brain.py                                 # sync brain/
  python3 harness/sync_obsidian_brain.py --vault /tmp/v --bench target/bench-smoke.json \\
      --kiro-root /tmp/kiro --no-open                                    # dry run elsewhere
  python3 harness/sync_obsidian_brain.py --check [--vault DIR]           # validate only

Python 3.9+, standard library only.
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
BENCH_FILE = ARTIFACTS / "aegis_phase1_telemetry.json"
TELEMETRY_FILE = ARTIFACTS / "aegis_telemetry.json"
MODEL_CARD = ROOT / "models" / "model_card.json"
TASKS_FILE = ROOT / "harness" / "TASKS.json"
CANVAS = VAULT / "AegisFlow_Architecture.canvas"
GRAPH = VAULT / ".obsidian" / "graph.json"
LOG_NOTE = VAULT / "Telemetry-Log.md"
INDEX_NOTE = VAULT / "00-AegisFlow-Index.md"
KIRO_ROOT = ROOT / ".kiro" / "specs"
KIRO_DIR = KIRO_ROOT / "aegisflow-phase1"
# Kiro spec directory per TASKS.json `phase` (tasks without a phase belong to phase 1).
KIRO_PHASES = {"phase-1": "aegisflow-phase1", "phase-2": "aegisflow-phase2"}
OBSIDIAN_REGISTRY = Path.home() / "Library" / "Application Support" / "obsidian" / "obsidian.json"


def configure(vault: Optional[Path] = None, bench: Optional[Path] = None, kiro_root: Optional[Path] = None) -> None:
    """Points the writers at another vault / bench file / Kiro root (dry runs and tests)."""
    global VAULT, CANVAS, GRAPH, LOG_NOTE, INDEX_NOTE, BENCH_FILE, KIRO_ROOT, KIRO_DIR
    if vault is not None:
        VAULT = vault.resolve()
        CANVAS = VAULT / "AegisFlow_Architecture.canvas"
        GRAPH = VAULT / ".obsidian" / "graph.json"
        LOG_NOTE = VAULT / "Telemetry-Log.md"
        INDEX_NOTE = VAULT / "00-AegisFlow-Index.md"
    if bench is not None:
        BENCH_FILE = bench.resolve()
    if kiro_root is not None:
        KIRO_ROOT = kiro_root.resolve()
        KIRO_DIR = KIRO_ROOT / KIRO_PHASES["phase-1"]

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
                "clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps "
                "pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation "
                "bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches "
                "dropped without a commit are detached into trash and deleted by a background reaper.",
     "implements": ["crates/aegis-storage/src/lib.rs", "crates/aegis-storage/src/pool.rs",
                    "crates/aegis-storage/src/reaper.rs", "crates/aegis-storage/src/fsevents.rs",
                    "crates/aegis-storage/src/sys.rs"]},
    {"id": "Core-Memory-Snapshot", "group": "core", "crate": "aegis_storage",
     "summary": "Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live "
                "trees in one syscall; the previous live tree is kept as a CoW version snapshot that "
                "`restore()` can bring back. A persisted generation number makes stale branches and standbys "
                "conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only "
                "(no process-memory snapshots).",
     "implements": ["crates/aegis-storage/src/lib.rs", "crates/aegis-storage/src/pool.rs"]},
    {"id": "Core-ANE-Classifier", "group": "core", "crate": "aegis_ane_scorer",
     "summary": "CoreML FP16 context-window risk model: one prediction scores the newest event of an agent "
                "session given the events before it (`ContextWindow` -> `score_window`), loaded with "
                "`.cpuAndNeuralEngine` through the Swift `@_cdecl` bridge. Device placement is read back from "
                "`MLComputePlan`. Weights are trained on synthetic sessions labelled by the documented teacher "
                "in models/build_model.py; this is not a validated production risk model.",
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
     "implements": [".kiro/specs/aegisflow-phase1/", ".kiro/specs/aegisflow-phase2/"]},
    # ---- Benchmarks -------------------------------------------------------------------------
    {"id": "Bench-APFS-Latency", "group": "benchmark",
     "summary": "Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel "
                "time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, "
                "caller-visible rollback with background reaping, the APFS collision cases and live-tree "
                "integrity after every phase.",
     "implements": ["crates/aegis-bench/src/apfs.rs", "crates/aegis-bench/src/main.rs"]},
    {"id": "Bench-ANE-FP16", "group": "benchmark",
     "summary": "Online context-window model latency per compute unit (units interleaved in rounds), "
                "cpuOnly / cpuAndNeuralEngine speedup, MLComputePlan per-operation placement, parity "
                "against the FP32 reference in models/parity_vectors.json, and the batch model.",
     "implements": ["crates/aegis-bench/src/ane.rs", "models/parity_vectors.json"]},
    {"id": "Bench-Cedar-Eval", "group": "benchmark",
     "summary": "Cedar `is_authorized` latency and the policy decision matrix (expected vs actual).",
     "implements": ["crates/aegis-bench/src/flows.rs"]},
    {"id": "Bench-EndToEnd-Rollback", "group": "benchmark",
     "summary": "Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> "
                "Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation "
                "and receipt signing.",
     "implements": ["crates/aegis-bench/src/flows.rs"]},
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
        shown = path.relative_to(ROOT) if path.is_relative_to(ROOT) else path
        print(f"warning: {shown} is not valid JSON ({exc})", file=sys.stderr)
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


def user_block(text: str) -> Optional[str]:
    """Content of the last user block whose start marker begins a line. Generated text may quote
    the marker inline (e.g. acceptance criteria in tasks.md); such mentions are not blocks."""
    start, pos = -1, 0
    while True:
        i = text.find(USER_START, pos)
        if i == -1:
            break
        if i == 0 or text[i - 1] == "\n":
            start = i
        pos = i + 1
    if start == -1:
        return None
    j = text.find(USER_END, start + len(USER_START))
    return None if j == -1 else text[start + len(USER_START):j]


def run_label(bench: Dict[str, Any]) -> Optional[str]:
    """`PRELIMINARY: reasons` for a preliminary run (from `run_conditions`, or a reduced scale in an
    artifact that predates it); None for a quiet full-scale run or no artifact."""
    if not bench:
        return None
    run = bench.get("run_conditions")
    if isinstance(run, dict) and isinstance(run.get("preliminary"), bool):
        if not run["preliminary"]:
            return None
        reasons = [str(r) for r in (run.get("reasons") or [])]
        return "PRELIMINARY: " + ("; ".join(reasons) or "see run_conditions")
    scale = bench.get("scale")
    if isinstance(scale, (int, float)) and not isinstance(scale, bool) and scale < 1:
        return f"PRELIMINARY: reduced scale {scale}"
    return None


# ---------------------------------------------------------------------------------------------
# node state from telemetry
# ---------------------------------------------------------------------------------------------
def infer_section(metric: str) -> str:
    """Section of a summary row from an artifact written before rows carried `section`
    (same rule as aegis-bench's tag_sections)."""
    if "integrity" in metric:
        return "integrity"
    if "CoreML" in metric or "model" in metric:
        return "ane"
    if metric.startswith("tier-0"):
        return "router"
    if metric.startswith("end-to-end"):
        return "e2e"
    return "apfs"


def cell(value: Any) -> str:
    """Markdown table cell text: pipes escaped, newlines flattened."""
    return str(value).replace("|", "\\|").replace("\n", " ")


class Ctx:
    def __init__(self) -> None:
        self.tel = load_json(TELEMETRY_FILE) or {}
        bench_file = load_json(BENCH_FILE)
        self.bench = bench_file or self.tel.get("bench") or {}
        self.card = load_json(MODEL_CARD) or {}
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
        self.gated: List[Dict[str, Any]] = [r for r in (self.bench.get("targets_summary") or []) if isinstance(r, dict)]
        self.info: List[Dict[str, Any]] = [r for r in (self.bench.get("informational") or []) if isinstance(r, dict)]
        for row in self.gated + self.info:
            row.setdefault("section", infer_section(str(row.get("metric", ""))))
        self.run_label = run_label(self.bench)

    def integrity_status(self) -> Optional[str]:
        """`failing` when the artifact's live-tree integrity row did not pass; None otherwise."""
        rows = self.info_rows("integrity")
        return "failing" if any(r.get("pass") is False for r in rows) else None

    def gated_status(self, rows: List[Dict[str, Any]], measured: bool) -> Optional[str]:
        """Status copied from the matching gated rows. Without matching rows: `pending` when the
        artifact has a targets_summary (no verdict to copy); `verified` for a measured value only in
        artifacts that predate targets_summary."""
        if rows:
            return rows_status(rows)
        if self.gated:
            return "pending"
        return "verified" if measured else None

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

    def fixtures(self) -> List[Dict[str, Any]]:
        return [fx for fx in (dig(self.bench, "apfs", "fixtures", default=[]) or []) if isinstance(fx, dict)]

    def fixture(self, name: str) -> Optional[Dict[str, Any]]:
        for fx in self.fixtures():
            if fx.get("name") == name:
                return fx
        return None

    def gated_rows(self, section: Optional[str] = None, prefix: Optional[str] = None) -> List[Dict[str, Any]]:
        return [r for r in self.gated if (section is None or r.get("section") == section)
                and (prefix is None or str(r.get("metric", "")).startswith(prefix))]

    def info_rows(self, section: Optional[str] = None) -> List[Dict[str, Any]]:
        return [r for r in self.info if section is None or r.get("section") == section]


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
        # Extra markdown blocks rendered inside the telemetry section (tables, lists).
        self.extra: List[str] = []


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


PLAN_LABELS = {"neural_engine": "ANE", "cpu": "CPU", "gpu": "GPU"}


def measured_text(row: Dict[str, Any]) -> str:
    """Renders a summary row's measured* fields the way the console does (unit from the key)."""
    parts: List[str] = []
    for key, value in row.items():
        if not key.startswith("measured"):
            continue
        if key == "measured_hit_rate":
            parts.append(f"hit rate {num(value)}")
        elif isinstance(value, (int, float)) and not isinstance(value, bool):
            unit = "ms" if key.endswith("_ms") else "µs" if key.endswith("_us") else ""
            parts.append(f"{num(value)} {unit}".strip())
        elif isinstance(value, dict):
            parts.append(" · ".join(f"{PLAN_LABELS.get(k, k)} {num(v)}" for k, v in value.items()))
        else:
            parts.append(num(value))
    if isinstance(row.get("p95_ms"), (int, float)):
        parts.append(f"p95 {num(row['p95_ms'])} ms")
    if isinstance(row.get("n"), int) and not isinstance(row.get("n"), bool):
        parts.append(f"n={row['n']}")
    return "; ".join(parts) or "—"


def verdict(row: Dict[str, Any], gated: bool) -> str:
    """The row's own pass flag, as written by the benchmark (never re-judged here). A gated row
    without a boolean pass counts as missed, as in the T18 gate and the console."""
    p = row.get("pass")
    if gated:
        return "✅ PASS" if p is True else ("❌ FAIL (target missed)" if p is False else "❌ FAIL (not measured)")
    return "not gated" + (" (would pass)" if p is True else " (would fail)" if p is False else "")


def rows_status(rows: List[Dict[str, Any]]) -> Optional[str]:
    """`verified` when every gated row passed, `target-missed` otherwise, None without rows."""
    if not rows:
        return None
    return "verified" if all(r.get("pass") is True for r in rows) else "target-missed"


def summary_table(rows: List[Dict[str, Any]], gated: bool) -> List[str]:
    if not rows:
        return []
    out = ["| Target row | Target | Measured | Result |", "|---|---|---|---|"]
    for r in rows:
        out.append(f"| {cell(r.get('metric', '?'))} | {cell(r.get('target', '—'))} | {cell(measured_text(r))} | {verdict(r, gated)} |")
    return out


def ratio_text(a: Optional[float], b: Optional[float]) -> str:
    if a is None or b is None or b <= 0:
        return ""
    return f" ({a / b:.1f}x)"


def fx_label(fx: Dict[str, Any]) -> str:
    return f"{fx.get('name')} ({fx.get('files')} files)"


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
    small = ctx.fixture("small") or {}
    st = State(rows=rows)

    if nid == "Core-APFS-Clonefile":
        for fx in ctx.fixtures():
            hit = p50(dig(fx, "pool", "acquire_hit_ms"))
            cold = p50(fx.get("clonefile_ms"))
            if hit is not None:
                rows.append((f"branch · {fx_label(fx)} · pool warm hit vs cold clonefile",
                             f"{num(hit)} ms vs {num(cold)} ms p50{ratio_text(cold, hit)} · "
                             f"hits {num(dig(fx, 'pool', 'hits'))} / misses {num(dig(fx, 'pool', 'misses'))}"))
            else:
                rows.append((f"clonefile · {fx_label(fx)}", fmt_stats(fx.get("clonefile_ms"), "ms")))
        for fx in ctx.fixtures():
            k = fx.get("kernel_clonefile") or {}
            if k:
                rows.append((f"kernel time per clonefile · {fx.get('name')} ({fx.get('entries')} entries)",
                             f"sys p50 {num(p50(k.get('sys_us')))} µs = {num(k.get('sys_us_per_entry_p50'))} µs/entry · "
                             f"sys share of wall {num(k.get('sys_share_of_wall_p50'))}"))
        slope = dig(bench, "apfs", "clonefile_us_per_entry")
        if slope is not None:
            rows.append(("cold clonefile wall cost per tree entry (fit)",
                         f"{num(slope)} µs/entry + {num(dig(bench, 'apfs', 'clonefile_fixed_us'))} µs"))
        kfit = dig(bench, "apfs", "kernel_sys_us_per_entry_fit")
        if kfit is not None:
            rows.append(("kernel sys time per tree entry (fit)",
                         f"{num(kfit)} µs/entry + {num(dig(bench, 'apfs', 'kernel_sys_fixed_us_fit'))} µs"))
        for fx in ctx.fixtures():
            if dig(fx, "pool", "replenish_ms"):
                rows.append((f"standby replenish (refill thread) · {fx.get('name')}", fmt_stats(dig(fx, "pool", "replenish_ms"), "ms")))
        for fx in ctx.fixtures():
            reap = dig(fx, "pool", "background_reaper", "reap_ms")
            if reap:
                rows.append((f"background reap per detached tree · {fx.get('name')}", fmt_stats(reap, "ms")))
        # Phase-2 artifacts gate on pool acquire; phase-1 artifacts gated the cold clonefile.
        branch_rows = ctx.gated_rows(prefix="branch acquire") or ctx.gated_rows(prefix="clonefile branch")
        st.latency_ms = p50(dig(small, "pool", "acquire_hit_ms")) or p50(small.get("clonefile_ms"))
        st.status = worst(test_status, ctx.gated_status(branch_rows or ctx.gated_rows(section="apfs"), st.latency_ms is not None),
                          ctx.integrity_status())
        if bench.get("targets_note"):
            st.notes.append(bench["targets_note"])
        if ctx.gated_rows(prefix="branch acquire"):
            st.notes.append("`latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller "
                            "waits for); the cold clonefile rows above are what each refill still costs in the kernel.")
    elif nid == "Core-Memory-Snapshot":
        for fx in ctx.fixtures():
            rows.append((f"renameatx_np RENAME_SWAP · {fx.get('name')}", fmt_stats(fx.get("renameatx_swap_ms"), "ms")))
        if small.get("commit_total_ms"):
            rows.append(("commit total (lock + checks + swap + version) · small", fmt_stats(small.get("commit_total_ms"), "ms")))
        for fx in ctx.fixtures():
            if fx.get("commit_caller_ms"):
                rows.append((f"commit caller-visible · {fx.get('name')}", fmt_stats(fx.get("commit_caller_ms"), "ms")))
        for fx in ctx.fixtures():
            ac = dig(fx, "pool", "acquire_after_commit") or {}
            if ac:
                src = ac.get("sources") or {}
                rows.append((f"acquire right after commit · {fx.get('name')}",
                             f"{fmt_stats(ac.get('acquire_ms'), 'ms')} · pool {num(src.get('pool_ready'))} / "
                             f"waited for refill {num(src.get('pool_after_waiting_for_refill'))} / cold {num(src.get('cold'))}"))
        coll = dig(small, "pool", "collisions") or {}
        if coll:
            rows.append(("commit right after acquire (refill clone in flight) · small", fmt_stats(coll.get("commit_right_after_acquire_ms"), "ms")))
            rows.append(("rollback right after acquire (refill clone in flight) · small", fmt_stats(coll.get("rollback_right_after_acquire_ms"), "ms")))
        if dig(small, "pool", "rollback_caller_ms"):
            rows.append(("rollback caller-visible (background reap) · small", fmt_stats(dig(small, "pool", "rollback_caller_ms"), "ms")))
            rows.append(("rollback synchronous (detach + delete) · small", fmt_stats(small.get("rollback_sync_caller_ms"), "ms")))
        st.latency_ms = p50(small.get("renameatx_swap_ms"))
        swap_rows = ctx.gated_rows(prefix="renameatx_np") + ctx.gated_rows(prefix="rollback")
        st.status = worst(test_status, ctx.gated_status(swap_rows, st.latency_ms is not None), ctx.integrity_status())
    elif nid == "Core-ANE-Classifier":
        ane = bench.get("ane", {})
        arch = ctx.card.get("architecture") or {}
        if arch:
            rows.append(("architecture (models/model_card.json)",
                         f"{arch.get('name')} · {num(arch.get('conv_layers'))} convs · {num(arch.get('hidden'))} hidden · "
                         f"{num(arch.get('views'))} horizon views · {num(arch.get('parameters'))} parameters · "
                         f"{num(arch.get('macs_per_prediction'))} MACs/prediction"))
        if ane.get("window_len") is not None:
            rows.append(("input", f"{ane.get('layout')} layout · last {num(ane.get('window_len'))} events x {num(ane.get('feature_count'))} features"))
        b2b = " · back-to-back" if ane.get("isolated") else ""
        rows.append((f"inference · cpuAndNeuralEngine{b2b}", fmt_stats(dig(ane, "runs", "cpu_and_neural_engine"), "ms")))
        rows.append((f"inference · cpuOnly{b2b}", fmt_stats(dig(ane, "runs", "cpu_only"), "ms")))
        if ane.get("speedup_cpu_only_over_ane_p50") is not None:
            rows.append((f"speedup (cpuOnly p50 / cpuAndNeuralEngine p50){b2b}", num(ane.get("speedup_cpu_only_over_ane_p50"))))
        for key, label in (("busy_gap", "busy CPU gap"), ("idle_gap", "idle CPU gap")):
            iso = dig(ane, "isolated", key) or {}
            if iso:
                rows.append((f"inference · isolated calls ({label}) · cpuAndNeuralEngine vs cpuOnly",
                             f"{fmt_stats(dig(iso, 'runs', 'cpu_and_neural_engine'), 'ms')} vs "
                             f"{fmt_stats(dig(iso, 'runs', 'cpu_only'), 'ms')} · speedup {num(iso.get('speedup_cpu_only_over_ane_p50'))}"))
        e2e_score = dig(bench, "end_to_end", "stage_us", "ane_score")
        if e2e_score:
            rows.append(("one call per transaction (end-to-end ane_score stage)", fmt_stats(e2e_score, "µs")))
        plan = ane.get("compute_plan") or {}
        if plan.get("ops"):
            groups: Dict[Tuple[str, str], int] = {}
            for op in plan["ops"]:
                key = (str(op.get("operator", "?")).split(".")[-1], str(op.get("preferred", "?")))
                groups[key] = groups.get(key, 0) + 1
            rows.append(("MLComputePlan per op (.cpuAndNeuralEngine)",
                         ", ".join(f"{op} x{n} -> {PLAN_LABELS.get(dev, dev)}" for (op, dev), n in groups.items())))
        rows.append(("parity · single events vs FP32 reference (max |diff| points)", num(dig(ane, "parity", "max_abs_diff"))))
        if dig(ane, "parity", "windows"):
            w = dig(ane, "parity", "windows")
            rows.append(("parity · sessions (score_window) vs FP32 reference",
                         f"max {num(w.get('max_abs_diff'))} · mean {num(w.get('mean_abs_diff'))} points · "
                         f"{num(w.get('within_tolerance'))}/{num(w.get('n'))} within {num(dig(ane, 'parity', 'tolerance_points'))}"))
        if ane.get("placement_note"):
            rows.append(("device placement (MLComputePlan)", ane["placement_note"]))
        st.latency_ms = p50(dig(ane, "runs", "cpu_and_neural_engine"))
        ane_rows = ctx.gated_rows(section="ane")
        st.status = worst(test_status, ctx.gated_status(ane_rows, st.latency_ms is not None))
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
        st.status = worst(test_status, ctx.gated_status(ctx.gated_rows(section="router"), us is not None))
    elif nid == "Adapter-PayPal-Commerce":
        pp = bench.get("paypal_receipt", {})
        rows.append(("prepare order (redact+sign+verify+score+Cedar)", fmt_stats(pp.get("prepare_order_us"), "µs")))
        if pp.get("score_us"):
            rows.append(("risk score inside prepare", fmt_stats(pp.get("score_us"), "µs")))
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
        present_total, expected_total = 0, 0
        for phase, dirname in KIRO_PHASES.items():
            files = [KIRO_ROOT / dirname / n for n in ("requirements.md", "design.md", "tasks.md")]
            present = [f for f in files if f.exists() and f.stat().st_size > 0]
            present_total += len(present)
            expected_total += len(files)
            rows.append((f"spec files · {dirname}", f"{len(present)}/3 present"))
        synced = sum(1 for t in ctx.tasks if t.get("status") == "passed")
        rows.append(("tasks verified by harness", f"{synced}/{len(ctx.tasks)}"))
        st.status = "verified" if present_total == expected_total else "pending"
        st.verified_at = now_iso() if present_total == expected_total else None
    elif nid == "Bench-APFS-Latency":
        rows.clear()
        for fx in ctx.fixtures():
            name = f"{fx.get('name')} · {fx.get('files')} files / {num((fx.get('bytes') or 0) / 1048576.0)} MiB"
            pool = fx.get("pool") or {}
            if pool:
                rows.append((name + " · acquire (pool warm hit)",
                             f"{fmt_stats(pool.get('acquire_hit_ms'), 'ms')} · hits {num(pool.get('hits'))} / misses {num(pool.get('misses'))}"))
            rows.append((name + " · clonefile (cold)", fmt_stats(fx.get("clonefile_ms"), "ms")))
            k = fx.get("kernel_clonefile") or {}
            if k:
                rows.append((name + " · kernel sys per clonefile", f"{fmt_stats(k.get('sys_us'), 'µs')} · {num(k.get('sys_us_per_entry_p50'))} µs/entry"))
            if pool:
                rows.append((name + " · standby replenish", fmt_stats(pool.get("replenish_ms"), "ms")))
            rows.append((name + " · RENAME_SWAP", fmt_stats(fx.get("renameatx_swap_ms"), "ms")))
            if fx.get("commit_caller_ms"):
                rows.append((name + " · commit caller-visible", fmt_stats(fx.get("commit_caller_ms"), "ms")))
            if pool:
                rows.append((name + " · rollback caller-visible (background reap)", fmt_stats(pool.get("rollback_caller_ms"), "ms")))
                rows.append((name + " · background reap", fmt_stats(dig(pool, "background_reaper", "reap_ms"), "ms")))
                ac = pool.get("acquire_after_commit") or {}
                src = ac.get("sources") or {}
                rows.append((name + " · acquire right after commit",
                             f"{fmt_stats(ac.get('acquire_ms'), 'ms')} · cold {num(src.get('cold'))} / waited {num(src.get('pool_after_waiting_for_refill'))} / pool {num(src.get('pool_ready'))}"))
            else:
                rows.append((name + " · rollback (detach)", fmt_stats(fx.get("rollback_detach_ms"), "ms")))
                rows.append((name + " · rollback (reclaim)", fmt_stats(fx.get("rollback_reclaim_ms"), "ms")))
            integ = fx.get("integrity") or {}
            if integ:
                rows.append((name + " · live-tree integrity", f"{num(integ.get('passed'))}/{num(integ.get('checks'))} phases"))
        method = dig(small, "pool", "method")
        if method:
            rows.append(("method (pool rows)", method))
        apfs_gated = ctx.gated_rows(section="apfs")
        if apfs_gated:
            st.extra += ["", "**Gated targets (from `targets_summary`)**", ""] + summary_table(apfs_gated, True)
            st.extra += ["", "**Informational (not gated)**", ""] + summary_table(ctx.info_rows("apfs") + ctx.info_rows("integrity"), False)
            st.status = worst(rows_status(apfs_gated) or "pending", ctx.integrity_status())
            st.latency_ms = p50(dig(small, "pool", "acquire_hit_ms"))
            if bench.get("targets_note"):
                st.notes.append(bench["targets_note"])
        elif small:
            # Older (phase-1) artifact without targets_summary sections.
            st.latency_ms = p50(small.get("clonefile_ms"))
            st.status = worst(target_status(st.latency_ms, ctx.targets.get("clonefile_branch_ms")),
                              target_status(p50(small.get("renameatx_swap_ms")), ctx.targets.get("renameatx_swap_ms")))
    elif nid == "Bench-ANE-FP16":
        ane = bench.get("ane", {})
        rows.clear()
        b2b = " · back-to-back" if ane.get("isolated") else ""
        for key, label in (("cpu_and_neural_engine", "cpuAndNeuralEngine"), ("cpu_only", "cpuOnly"), ("all", "all")):
            rows.append((f"online window model · {label}{b2b}", fmt_stats(dig(ane, "runs", key), "ms")))
        method = ane.get("method") or {}
        if method:
            rows.append(("method", f"{method.get('call')}; {method.get('pattern') or 'back-to-back'}; units interleaved: "
                                   f"{num(method.get('interleaved'))} · {num(method.get('rounds'))} rounds x {num(method.get('per_round'))} per unit"))
        if ane.get("speedup_cpu_only_over_ane_p50") is not None:
            rows.append((f"speedup cpuOnly p50 / cpuAndNeuralEngine p50{b2b}", num(ane.get("speedup_cpu_only_over_ane_p50"))))
        iso = ane.get("isolated") or {}
        if iso:
            im = iso.get("method") or {}
            rows.append(("method · isolated calls", f"{im.get('call')}; gap {num(im.get('gap_ms'))} ms; busy: {im.get('busy_gap')}; "
                                                   f"idle: {im.get('idle_gap')}; {num(im.get('rounds'))} rounds x {num(im.get('per_round'))} per unit"))
            for key, label in (("busy_gap", "busy CPU gap"), ("idle_gap", "idle CPU gap")):
                for unit, ulabel in (("cpu_and_neural_engine", "cpuAndNeuralEngine"), ("cpu_only", "cpuOnly")):
                    rows.append((f"online window model · {ulabel} · isolated calls ({label})", fmt_stats(dig(iso, key, "runs", unit), "ms")))
                rows.append((f"speedup cpuOnly / cpuAndNeuralEngine · isolated calls ({label})", num(dig(iso, key, "speedup_cpu_only_over_ane_p50"))))
        e2e_score = dig(bench, "end_to_end", "stage_us", "ane_score")
        if e2e_score:
            rows.append(("one call per transaction (end-to-end ane_score stage)", fmt_stats(e2e_score, "µs")))
        plan = ane.get("compute_plan", {})
        pref = plan.get("preferred", {})
        if plan:
            rows.append(("MLComputePlan preferred device",
                         f"ANE {num(pref.get('neural_engine'))} · CPU {num(pref.get('cpu'))} · GPU {num(pref.get('gpu'))} "
                         f"of {num(plan.get('operations'))} dispatched ops ({num(plan.get('operations_total'))} incl. constants)"))
        rows.append(("Neural Engine cores (MLNeuralEngineComputeDevice)", num(ane.get("neural_engine_cores"))))
        parity = ane.get("parity") or {}
        rows.append(("parity · single events (max |diff| points)", num(parity.get("max_abs_diff"))))
        if parity.get("windows"):
            rows.append(("parity · sessions, cpuAndNeuralEngine", f"max {num(dig(parity, 'windows', 'max_abs_diff'))} · mean {num(dig(parity, 'windows', 'mean_abs_diff'))} points"))
            rows.append(("parity · sessions, cpuOnly", f"max {num(dig(parity, 'windows_cpu_only', 'max_abs_diff'))} · mean {num(dig(parity, 'windows_cpu_only', 'mean_abs_diff'))} points"))
            for scen, s in (parity.get("windows_by_scenario") or {}).items():
                rows.append((f"parity · scenario {scen}", f"max {num(s.get('max_abs_diff'))} · mean {num(s.get('mean_abs_diff'))} points (n={num(s.get('n'))})"))
        batch = ane.get("batch", {})
        if batch:
            size = batch.get("batch_size")
            bpref = dig(batch, "compute_plan", "preferred", default={}) or {}
            rows.append((f"batch model ({num(size)} windows/prediction) MLComputePlan",
                         f"ANE {num(bpref.get('neural_engine'))} · CPU {num(bpref.get('cpu'))} · GPU {num(bpref.get('gpu'))} "
                         f"of {num(dig(batch, 'compute_plan', 'operations'))} ops"))
            for key, label in (("cpu_and_neural_engine", "cpuAndNeuralEngine"), ("cpu_only", "cpuOnly")):
                run = dig(batch, "runs", key, default={}) or {}
                per = run.get("amortized_us_per_window_p50", run.get("amortized_us_per_operation_p50"))
                rows.append((f"batch prediction · {label}", f"{fmt_stats(run.get('batch_ms'), 'ms')} · {num(per)} µs/window"))
            if batch.get("speedup_cpu_only_over_ane_p50") is not None:
                rows.append(("batch speedup cpuOnly / cpuAndNeuralEngine", num(batch.get("speedup_cpu_only_over_ane_p50"))))
        if plan.get("ops"):
            st.extra += ["", "**MLComputePlan per-op placement (.cpuAndNeuralEngine, dispatched ops in program order)**", "",
                         "| # | operator | device | supported |", "|---|---|---|---|"]
            for op in plan["ops"]:
                st.extra.append(f"| {op.get('index')} | `{op.get('operator')}` | {PLAN_LABELS.get(op.get('preferred'), op.get('preferred'))} | "
                                f"{', '.join(PLAN_LABELS.get(s, s) for s in (op.get('supported') or []))} |")
        ane_gated = ctx.gated_rows(section="ane")
        st.latency_ms = p50(dig(ane, "runs", "cpu_and_neural_engine"))
        if ane_gated:
            st.extra = ["", "**Gated targets (from `targets_summary`)**", ""] + summary_table(ane_gated, True) + \
                       ["", "**Informational (not gated)**", ""] + summary_table(ctx.info_rows("ane"), False) + st.extra
            st.status = rows_status(ane_gated) or "pending"
        elif st.latency_ms is not None:
            on_ane = plan.get("operations") and pref.get("neural_engine") == plan.get("operations")
            st.status = worst(target_status(st.latency_ms, ctx.targets.get("ane_inference_ms")),
                              "verified" if on_ane else "target-missed")
        if ane.get("placement_note"):
            st.notes.append(ane["placement_note"])
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
        if e2e.get("branching"):
            rows.append(("branching", f"{e2e.get('branching')} (replenish delay {num(e2e.get('replenish_delay_ms'))} ms)"))
        if e2e.get("scoring"):
            rows.append(("scoring", e2e["scoring"]))
        rows.append(("rollback path (acquire→edit→diff→score→Cedar deny→rollback)", fmt_stats(e2e.get("rollback_path_ms"), "ms")))
        rows.append(("commit path (acquire→edit→diff→score→Cedar allow→swap)", fmt_stats(e2e.get("commit_path_ms"), "ms")))
        for path, full_key, p50_key in (("rollback", "stage_us", "stage_p50_us"), ("commit", "commit_stage_us", "commit_stage_p50_us")):
            if e2e.get(full_key):
                for stage, stats in e2e[full_key].items():
                    rows.append((f"{path} path stage · {stage}", fmt_stats(stats, "µs")))
            else:
                for stage, value in (e2e.get(p50_key) or {}).items():
                    rows.append((f"{path} path stage p50 · {stage}", f"{num(value)} µs"))
        for path, src in (dig(e2e, "acquire_sources", default={}) or {}).items():
            rows.append((f"acquire sources · {path}", f"pool {num(src.get('pool_ready'))} / waited {num(src.get('pool_after_waiting_for_refill'))} / cold {num(src.get('cold'))}"))
        for case in ("rollback_case", "commit_case"):
            c = e2e.get(case) or {}
            if c.get("risk_score_without_history"):
                rows.append((f"{case} risk p50 · with session history vs without",
                             f"{num(p50(c.get('risk_score')))} vs {num(p50(c.get('risk_score_without_history')))}"))
        rows.append(("live tree byte-identical after every rollback", num(e2e.get("live_integrity_verified"))))
        if "commit_integrity_verified" in e2e:
            rows.append(("live tree equals the last allowed commit", num(e2e.get("commit_integrity_verified"))))
        rows.append(("PayPal prepare + ed25519 receipt", fmt_stats(pp.get("prepare_order_us"), "µs")))
        rows.append(("ed25519 verify", fmt_stats(pp.get("ed25519_verify_us"), "µs")))
        st.latency_ms = p50(e2e.get("rollback_path_ms"))
        if st.latency_ms is not None:
            ok = e2e.get("live_integrity_verified") and e2e.get("commit_integrity_verified", True)
            st.status = "verified" if ok else "failing"

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
    if ctx.run_label:
        out += [f"> **{ctx.run_label}.** Only a quiet full-scale benchmark run decides the phase result.", ""]
    if st.rows:
        out += ["| Metric | Measured |", "|---|---|"]
        out += [f"| {cell(k)} | {cell(v)} |" for k, v in st.rows]
    else:
        out.append("No measurements recorded yet.")
    out += st.extra
    for note in st.notes:
        out += ["", f"> {note}"]
    out.append(TEL_END)
    return "\n".join(out)


def write_note(node: Dict[str, Any], st: State, ctx: Ctx) -> None:
    path = VAULT / note_rel(node)
    path.parent.mkdir(parents=True, exist_ok=True)
    user_section = "\n_Your notes here are preserved across syncs._\n"
    if path.exists():
        kept = user_block(path.read_text())
        if kept is not None:
            user_section = kept
    up, down = links_for(node["id"])
    body = [frontmatter(node, st, ctx), "", f"# {node['id']}", "",
            f"**Status:** {BADGE.get(st.status, st.status)} · **p50:** "
            f"{num(st.latency_ms) + ' ms' if st.latency_ms is not None else '—'} · **commit:** `{ctx.commit}`"
            + (" · **bench run: PRELIMINARY**" if ctx.run_label and node["group"] != "agent" else ""), "",
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
             f"tests {num(tests.get('passed'))} passed / {num(tests.get('failed'))} failed", ""]
    if ctx.run_label:
        lines += [f"**{ctx.run_label}.** Only a quiet full-scale benchmark run decides the phase result.", ""]
    if ctx.gated:
        passed = sum(1 for r in ctx.gated if r.get("pass") is True)
        failed = sum(1 for r in ctx.gated if r.get("pass") is False)
        lines += [f"**Gated targets: {passed}/{len(ctx.gated)} passed" + (f", {failed} missed**" if failed else "**")
                  + (f" · {len(ctx.info)} informational rows (not gated)" if ctx.info else ""), ""]
        lines += summary_table(ctx.gated, True) + [""]
        if ctx.bench.get("targets_note"):
            lines += [f"> {ctx.bench['targets_note']}", ""]
    lines += ["| Node | Status | p50 |", "|---|---|---|"]
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
        pool_rb = p50(dig(small, "pool", "rollback_caller_ms"))
        gated = f"{sum(1 for r in ctx.gated if r.get('pass') is True)}/{len(ctx.gated)}" if ctx.gated else "—"
        run = "preliminary" if ctx.run_label else ("quiet" if isinstance(b.get("run_conditions"), dict) else "—")
        row = ("| {t} | `{c}` | {tp}/{tf} | {clone} | {swap} | {rb} | {ane} | {cedar} | {e2e} "
               "| {acq} | {reap} | {speedup} | {gated} | {scale} | {run} |").format(
            t=ctx.bench_time, c=ctx.commit, tp=num(tests.get("passed")), tf=num(tests.get("failed")),
            clone=num(p50(small.get("clonefile_ms"))), swap=num(p50(small.get("renameatx_swap_ms"))),
            rb=num(pool_rb if pool_rb is not None else p50(small.get("rollback_detach_ms"))),
            ane=num(p50(dig(b, "ane", "runs", "cpu_and_neural_engine"))),
            cedar=num(p50(dig(b, "cedar", "is_authorized_us"))),
            e2e=num(p50(dig(b, "end_to_end", "rollback_path_ms"))),
            acq=num(p50(dig(small, "pool", "acquire_hit_ms"))),
            reap=num(p50(dig(small, "pool", "background_reaper", "reap_ms"))),
            speedup=num(dig(b, "ane", "speedup_cpu_only_over_ane_p50")),
            gated=gated, scale=num(b.get("scale")), run=run)
        same_run = [i for i, ln in enumerate(rows) if ln.startswith(f"| {ctx.bench_time} ")]
        if not same_run:
            rows.append(row)
            changed = True
        elif rows[same_run[0]] != row:
            rows[same_run[0]] = row
            changed = True
    added = changed
    header = ["---", "tags:", "  - benchmark-log", "---", "", "# Telemetry log", "",
              "One row per `aegis-bench` run (p50 values; `small` APFS fixture). Source: `artifacts/aegis_phase1_telemetry.json`. "
              "`clonefile` is the cold clone; `rollback` is caller-visible (detach only for phase-1 rows, which have no pool, "
              "reaper, speedup or gate columns); `gated` counts passed rows of `targets_summary`; `scale` < 1 marks a reduced run; "
              "`run` is `preliminary` for a reduced-scale or busy-machine run (`run_conditions`), `quiet` otherwise, `—` when not recorded.",
              "", "Back to [[00-AegisFlow-Index]]", "", LOG_START,
              "| bench generated_at | commit | tests pass/fail | clonefile ms | RENAME_SWAP ms | rollback ms | ANE ms | Cedar µs "
              "| e2e rollback ms | pool acquire ms | reap ms | ANE speedup | gated | scale | run |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
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
    card = summary_markdown(states, ctx)
    card_h = max(520, 28 * (card.count("\n") + 1))  # default size only; an existing card keeps the user's size
    ours.append({"id": "live-telemetry", "type": "text", "text": card,
                 "x": -30, "y": -100 - card_h, "width": 3 * COL_W + NODE_W + 60, "height": card_h})
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
        prev_edges = {e.get("id"): e for e in existing.get("edges", []) if isinstance(e, dict)}
        for edge in edges:
            old = prev_edges.get(edge["id"])
            if old:  # keep sides / colour the user picked for our edges
                for key in ("fromSide", "toSide", "color"):
                    if key in old:
                        edge[key] = old[key]
        extra_nodes = [n for nid, n in prev.items() if nid not in our_ids]
        extra_edges = [e for e in existing.get("edges", []) if isinstance(e, dict) and not str(e.get("id", "")).startswith("aegis-e-")]
        ours += extra_nodes
        edges += extra_edges
    CANVAS.write_text(json.dumps({"nodes": ours, "edges": edges}, indent=2, ensure_ascii=False) + "\n")


def write_kiro_tasks(ctx: Ctx) -> None:
    """One generated tasks.md per Kiro spec directory; a task goes to the directory of its `phase`
    (tasks without one belong to phase 1). Directories that do not exist are not created."""
    for phase, dirname in KIRO_PHASES.items():
        kiro_dir = KIRO_ROOT / dirname
        if not kiro_dir.exists():
            continue
        path = kiro_dir / "tasks.md"
        kept = user_block(path.read_text()) if path.exists() else None
        tasks = [t for t in ctx.tasks if (t.get("phase") or "phase-1") == phase]
        lines = ["# Implementation Plan", "",
                 "_Generated by harness/sync_obsidian_brain.py from harness/TASKS.json "
                 f"(tasks with phase `{phase}`). A box is ticked only when the task's verify command last exited 0._", ""]
        for i, task in enumerate(tasks, start=1):
            box = "x" if task.get("status") == "passed" else " "
            lines.append(f"- [{box}] {i}. {task.get('title')} (`{task.get('id')}`, owner: {task.get('owner')})")
            for crit in task.get("acceptance", []):
                lines.append(f"  - {crit}")
            lines.append(f"  - Verify: `{task.get('verify')}`")
            res = task.get("last_result") or {}
            if task.get("status") and task.get("status") != "pending":
                lines.append(f"  - Last result: {task.get('status')} (exit {res.get('exit_code')}, {res.get('finished_at', '—')})")
            reqs = task.get("requirements") or []
            if reqs:
                lines.append(f"  - _Requirements: {', '.join(reqs)}_")
            lines.append("")
        user = kept if kept is not None else "\n_Your notes here are preserved across syncs._\n"
        lines += ["## Notes", USER_START + user + USER_END, ""]
        path.write_text("\n".join(lines))


# ---------------------------------------------------------------------------------------------
# checks + Obsidian launch
# ---------------------------------------------------------------------------------------------
GEOMETRY = ("x", "y", "width", "height")


def user_block_files() -> List[Path]:
    return [VAULT / note_rel(n) for n in NODES] + [KIRO_ROOT / d / "tasks.md" for d in KIRO_PHASES.values()]


def snapshot() -> Dict[str, Any]:
    """What a sync must preserve: canvas geometry of every node, user edges, user blocks."""
    canvas = load_json(CANVAS)
    geometry: Dict[str, Dict[str, Any]] = {}
    edges: set = set()
    if isinstance(canvas, dict):
        for n in canvas.get("nodes", []):
            if isinstance(n, dict) and n.get("id"):
                geometry[n["id"]] = {k: n[k] for k in GEOMETRY if k in n}
        edges = {e.get("id") for e in canvas.get("edges", [])
                 if isinstance(e, dict) and e.get("id") and not str(e.get("id")).startswith("aegis-e-")}
    blocks = {}
    for path in user_block_files():
        if path.exists():
            block = user_block(path.read_text())
            if block is not None:
                blocks[str(path)] = block
    return {"geometry": geometry, "edges": edges, "blocks": blocks}


def preservation_problems(before: Dict[str, Any]) -> List[str]:
    after = snapshot()
    problems: List[str] = []
    for nid, geo in before["geometry"].items():
        now = after["geometry"].get(nid)
        if now is None:
            problems.append(f"preserve: canvas node {nid} was dropped")
            continue
        for key, value in geo.items():
            if now.get(key) != value:
                problems.append(f"preserve: canvas node {nid} {key} changed {value} -> {now.get(key)}")
    for eid in sorted(before["edges"] - after["edges"]):
        problems.append(f"preserve: user canvas edge {eid} was dropped")
    for path, block in before["blocks"].items():
        if after["blocks"].get(path) != block:
            shown = Path(path)
            problems.append(f"preserve: user block changed in {shown.parent.name}/{shown.name}")
    return problems


def phase2_problems(ctx: Ctx) -> List[str]:
    """`--require-phase2`: the phase-2 artifact is rendered into the notes, the canvas card and the log."""
    problems: List[str] = []
    if ctx.bench.get("schema") != "aegisflow.telemetry.v2":
        return [f"phase2: {BENCH_FILE} is not a phase-2 artifact (schema {ctx.bench.get('schema')})"]
    if not ctx.gated:
        problems.append("phase2: the artifact has no targets_summary rows")
    note = ctx.bench.get("targets_note")
    canvas = load_json(CANVAS) or {}
    card = next((n.get("text", "") for n in canvas.get("nodes", []) if isinstance(n, dict) and n.get("id") == "live-telemetry"), "")
    if "Gated targets:" not in card:
        problems.append("phase2: canvas card lacks the gated targets table")
    if note and note not in card:
        problems.append("phase2: canvas card lacks targets_note")
    for nid in ("Bench-APFS-Latency", "Bench-ANE-FP16"):
        path = VAULT / f"{GROUPS['benchmark']['dir']}/{nid}.md"
        if "**Gated targets (from `targets_summary`)**" not in (path.read_text() if path.exists() else ""):
            problems.append(f"phase2: {nid} lacks the gated targets table")
    for nid, group in (("Core-APFS-Clonefile", "core"), ("Bench-APFS-Latency", "benchmark")):
        path = VAULT / f"{GROUPS[group]['dir']}/{nid}.md"
        if note and note not in (path.read_text() if path.exists() else ""):
            problems.append(f"phase2: {nid} lacks targets_note")
    log = LOG_NOTE.read_text() if LOG_NOTE.exists() else ""
    if ctx.bench_time and f"| {ctx.bench_time} " not in log:
        problems.append("phase2: Telemetry-Log has no row for this benchmark run")
    return problems


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
    ap.add_argument("--no-open", action="store_true", help="never launch Obsidian (wins over --open)")
    ap.add_argument("--check", action="store_true", help="validate vault structure only (no writes)")
    ap.add_argument("--vault", "--brain", dest="vault", type=Path, help="vault directory to write (default: brain/)")
    ap.add_argument("--bench", type=Path, help="benchmark JSON to render (default: artifacts/aegis_phase1_telemetry.json)")
    ap.add_argument("--kiro-root", type=Path, help="directory holding aegisflow-phase*/ specs (default: .kiro/specs)")
    ap.add_argument("--require-phase2", action="store_true",
                    help="exit 1 unless a phase-2 artifact was rendered (gated table and targets_note in notes, card and log)")
    args = ap.parse_args()
    configure(vault=args.vault, bench=args.bench, kiro_root=args.kiro_root)
    if args.bench is not None and not BENCH_FILE.exists():
        print(f"error: --bench {BENCH_FILE} does not exist", file=sys.stderr)
        return 2

    if args.check:
        problems = check_vault()
        for p in problems:
            print(f"check: {p}")
        print("check: OK" if not problems else f"check: {len(problems)} problem(s)")
        return 1 if problems else 0

    ctx = Ctx()
    VAULT.mkdir(parents=True, exist_ok=True)
    before = snapshot()
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
          + f" · commit {ctx.commit} · log {'updated' if added else 'unchanged'}"
          + (f" · {ctx.run_label}" if ctx.run_label else ""))
    problems = check_vault() + preservation_problems(before)
    if args.require_phase2:
        problems += phase2_problems(ctx)
    for p in problems:
        print(f"check: {p}")
    if args.open and not args.no_open:
        open_obsidian()
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
