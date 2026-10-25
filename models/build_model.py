#!/usr/bin/env python3
"""Train, export and compile the AegisWorkflowFP16 context-window risk model (CoreML, FP16).

SYNTHETIC DATA. Every training and validation example comes from the seeded scenario simulator in
this file and is labelled by the explicit teacher documented below. None of it is real telemetry or
incident data; the model reproduces the teacher and demonstrates the on-device pipeline.

Model. Input "features" FP16 (1, 64, 1, 64): channels are the 64 features of
models/feature_schema.json, the width holds the last 64 events of one agent session, oldest ->
newest, zero-padded on the left. A shared temporal convolution tower (1x1 stem, residual blocks of
1xk + 1x1 convolutions, ReLU) runs over K recency views of the window stacked on the height axis
(view k keeps only the newest HORIZONS[k] events). Kernel sizes and dilations come from the sweep
candidate named by SELECTED_ARCH (the selected mh128_b5_v8_nd tower is undilated 1x3 + 1x1; the sweep
also measures dilated towers); model_card.json architecture records the shipped values and the
receptive field of the newest-position readout. The readout takes the newest position and
the window mean of every view, mixes them with a (K x 1) convolution and a 1x1 convolution, and
returns risk_score = 100 * sigmoid(logit). The views reuse the tower weights K times; on Apple M2
the Neural Engine is bound by weight traffic at 64 positions, so this reuse is what lets a 128-channel
network run there faster than on the CPU (see the sweep table in model_card.json).

Teacher (window label). With e_T the newest event and H the earlier events inside the window:
    Z     = z(e_T) + T_velocity + T_spend + T_secrets + T_deletes + T_entropy
    label = sigmoid(slope * (Z - bias))            (slope, bias from feature_schema.json)
z(e) is the phase-1 per-event teacher (linear weights + pairwise interactions, feature_schema.json).
The temporal terms (WINDOW_TEACHER) are zero when H is empty, so a session with no history gets
exactly the per-event label:
    T_velocity  newest event is an order: w * clip((orders among the last 16 earlier events - 2) / 6)
    T_spend     newest event is an order: w * clip((log10(1 + earlier spend in USD) - 2) / 1.5)
    T_secrets   w * clip(earlier events with secret_pattern_hits > 0 / 3)
    T_deletes   w * clip((sum of normalised files_deleted over the last 8 earlier events - 0.25) / 1)
    T_entropy   w * clip((H(types of the last 8 events) - H(types of the 24 before them) - 1 bit) / 1 bit)
where clip(.) clamps to [0, 1] and an event's type is the first match of payment, secret, delete,
build, shell/network, config, edit, idle.

Pipeline (deterministic for a given seed and toolchain):
  1. simulate SIM_CONFIG["sessions"] sessions; cut windows; label them with the window teacher
  2. split by session into train / validation
  3. train the network with PyTorch (CPU, float32)
  4. build FP16 ML Programs with coremltools' MIL Builder from the trained weights:
       online AegisWorkflowFP16       (1, 64, 1, 64) -> 1 score         metadata aegis_input_layout=window
       batch  AegisWorkflowFP16Batch  (B, 64, 1, 64) -> B scores        metadata aegis_input_layout=window_batch
     save the .mlpackage files and compile them to .mlmodelc
     (model_card.json "validation" scores the PyTorch FP32 reference; "validation_coreml_fp16" scores the
     shipped FP16 package on the same held-out windows; package metadata carries val_mae_points_fp32)
  5. parity: CoreML FP16 vs the PyTorch FP32 forward pass on held-out windows -> parity_vectors.json
  6. MLComputePlan placement and interleaved .cpuAndNeuralEngine / .cpuOnly latency of the built
     models through the production Swift bridge (models/tools/ane_probe.swift) -> model_card.json

Usage:  .venv/bin/python models/build_model.py [--verify-only | --sweep [--sweep-only NAME,...]]
  --verify-only  check existing artifacts: shapes, layout metadata, parity, placement, packages load
  --sweep        architecture search with seeded random weights -> models/arch_sweep.json
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Dict, List, Optional, Tuple

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
SCHEMA_PATH = HERE / "feature_schema.json"
MLPACKAGE = HERE / "AegisWorkflowFP16.mlpackage"
MLMODELC = HERE / "AegisWorkflowFP16.mlmodelc"
BATCH_MLPACKAGE = HERE / "AegisWorkflowFP16Batch.mlpackage"
BATCH_MLMODELC = HERE / "AegisWorkflowFP16Batch.mlmodelc"
PARITY = HERE / "parity_vectors.json"
CARD = HERE / "model_card.json"
SWEEP = HERE / "arch_sweep.json"
BRIDGE_SWIFT = ROOT / "crates" / "aegis-ane-scorer" / "swift" / "AegisANEBridge.swift"
PROBE_SWIFT = HERE / "tools" / "ane_probe.swift"

FEATURES = 64
WINDOW = 64
LAYOUT_KEY = "aegis_input_layout"

# Online architecture: the candidate `--sweep` selected (smallest that passes SELECTION). The sweep
# table and its selection are copied into model_card.json by the full build.
SELECTED_ARCH = "mh128_b5_v8_nd"
# Batch model: the same weights over (BATCH_WINDOWS, 64, 1, 64), one session per batch entry.
BATCH_WINDOWS = 32

# A candidate passes when every dispatched op is preferred on the Neural Engine, its
# .cpuAndNeuralEngine p50 is below max_p50_ms, and cpuOnly p50 / ANE p50 >= min_speedup. The speedup
# floor sits above 1.0 because the machine is shared during the sweep and a quiet re-run can move
# the ratio by a few percent.
SELECTION = {"max_p50_ms": 0.6, "min_speedup": 1.15, "allowed_conv_layers": [12, 16]}
PROBE_SETTINGS = {"rounds": 40, "per_round": 60, "warmup": 300}

SIM_CONFIG = {
    "seed": 20261025,
    "sessions": 10000,
    "val_fraction": 0.15,
    "extra_cuts_per_session": 2,
    "first_event_cut_probability": 0.4,
    "zero_history_copy_probability": 0.25,
    "scenario_mix": {
        "benign_edit": 0.15, "refactor": 0.10, "secret_leak": 0.10, "mass_delete": 0.08,
        "build_break_loop": 0.10, "procurement": 0.15, "mixed": 0.12, "single_event": 0.20,
    },
    "single_event_mix": {"workspace_edit": 0.40, "build_failure": 0.20, "order": 0.30, "uniform": 0.10},
}
TRAIN_CONFIG = {
    "seed": 20261025,
    "epochs": 14,
    "batch_size": 128,
    "lr": 2e-3,
    "weight_decay": 1e-4,
    "zero_history_weight": 2.0,
    "torch_threads": 4,
}
WINDOW_TEACHER = {
    "payment_velocity": {"weight": 1.2, "lookback_events": 16, "free_orders": 2, "saturate_orders": 8,
                         "gated_by_newest_order": True},
    "cumulative_spend": {"weight": 1.5, "lookback": "window", "log10_dollars_start": 2.0, "log10_dollars_span": 1.5,
                         "gated_by_newest_order": True},
    "repeated_secret_hits": {"weight": 2.0, "lookback": "window", "saturate_events": 3},
    "deletion_burst": {"weight": 1.5, "lookback_events": 8, "start": 0.25, "span": 1.0},
    "command_entropy_spike": {"weight": 1.0, "recent_events": 8, "baseline_events": 24,
                              "free_bits": 1.0, "span_bits": 1.0},
}
EVENT_TYPES = ["payment", "secret", "delete", "build", "shell_network", "config", "edit", "idle"]
PARITY_TOLERANCE = 1.0  # score points (0..100 scale)

# Phase-1 per-event sampler settings (used for single-event sessions and background events).
P_ACTIVE = {
    "workspace_edit": {"change_footprint": 0.35, "critical_paths": 0.06, "content_signals": 0.06,
                       "execution_context": 0.25, "commerce": 0.0},
    "build_failure": {"change_footprint": 0.30, "critical_paths": 0.08, "content_signals": 0.06,
                      "execution_context": 0.40, "commerce": 0.0},
    "order": {"change_footprint": 0.0, "critical_paths": 0.0, "content_signals": 0.05,
              "execution_context": 0.15, "commerce": 0.30},
}
ALWAYS = {
    "workspace_edit": ["files_touched", "bytes_changed", "lines_added", "agent_uncertainty"],
    "build_failure": ["files_touched", "bytes_changed", "lines_added", "agent_uncertainty"],
    "order": ["order_amount_cents", "line_items", "quantity_total", "budget_utilization"],
}
UNIFORM_DENSITY = 0.12
BINARY_RATE = 0.25

# The four PayPal bench orders (crates/aegis-bench/src/main.rs bench_paypal), as the PayPal
# middleware turns them into CommerceContext with an empty spend history: budget 20000 cents
# (adapter-paypal DEFAULT_BUDGET_CENTS), USD, one line item, receipt verified, one PII field.
PAYPAL_CASES = [
    {"case": "$42.00 known payee", "amount_cents": 4200, "quantity": 2, "new_payee": False,
     "payee_country_risk": 0.05, "unformatted_sensitive_fields": False, "phase1_decision": "Allow"},
    {"case": "$120.00 known payee", "amount_cents": 12000, "quantity": 2, "new_payee": False,
     "payee_country_risk": 0.05, "unformatted_sensitive_fields": False, "phase1_decision": "Allow"},
    {"case": "$480.00 new payee, high-risk country", "amount_cents": 48000, "quantity": 8, "new_payee": True,
     "payee_country_risk": 0.9, "unformatted_sensitive_fields": False, "phase1_decision": "Deny"},
    {"case": "$25.00 with redaction disabled", "amount_cents": 2500, "quantity": 1, "new_payee": False,
     "payee_country_risk": 0.05, "unformatted_sensitive_fields": True, "phase1_decision": "Deny"},
]
PAYPAL_BUDGET_CENTS = 20000


# ---------------------------------------------------------------------------------------------
# Schema helpers
# ---------------------------------------------------------------------------------------------
def load_schema() -> Dict:
    schema = json.loads(SCHEMA_PATH.read_text())
    feats = schema["features"]
    assert len(feats) == schema["feature_count"] == FEATURES, "schema must declare 64 features"
    assert [f["index"] for f in feats] == list(range(FEATURES)), "feature indices must be 0..63 in order"
    return schema


class Normalizer:
    """Raw measurement -> normalised feature value, following feature_schema.json (and features.rs)."""

    def __init__(self, schema: Dict):
        self.feats = {f["name"]: f for f in schema["features"]}
        self.index = {f["name"]: f["index"] for f in schema["features"]}

    def value(self, name: str, raw: float) -> float:
        f = self.feats[name]
        raw = max(float(raw), 0.0) if math.isfinite(raw) else 0.0
        kind = f["kind"]
        if kind == "binary":
            v = 1.0 if raw > 0 else 0.0
        elif kind == "ratio":
            v = raw
        elif kind == "count":
            v = raw / float(f["cap"])
        else:
            v = math.log1p(raw) / math.log1p(float(f["cap"]))
        return min(max(v, 0.0), 1.0)

    def event(self, raw: Dict[str, float]) -> np.ndarray:
        e = np.zeros(FEATURES, dtype=np.float32)
        for name, r in raw.items():
            e[self.index[name]] = self.value(name, r)
        return e


def event_logits(E: np.ndarray, schema: Dict) -> np.ndarray:
    """Per-event teacher logit z(e) (before slope / bias), phase-1 definition."""
    w = np.array([f["w"] for f in schema["features"]], dtype=np.float64)
    E = E.astype(np.float64)
    z = E @ w
    idx = {f["name"]: f["index"] for f in schema["features"]}
    for it in schema["interactions"]:
        z += it["w"] * E[:, idx[it["a"]]] * E[:, idx[it["b"]]]
    return z


def squash(z: np.ndarray, schema: Dict) -> np.ndarray:
    t = schema["teacher"]
    return 1.0 / (1.0 + np.exp(-t["slope"] * (z - t["bias"])))


# ---------------------------------------------------------------------------------------------
# Phase-1 per-event sampler (single-event sessions, background noise)
# ---------------------------------------------------------------------------------------------
def sample_values(rng: np.random.Generator, feat: Dict, n: int) -> np.ndarray:
    kind = feat["kind"]
    if kind == "binary":
        return np.ones(n, dtype=np.float32)
    if kind == "ratio":
        return rng.beta(1.5, 3.0, size=n).astype(np.float32)
    if kind == "count":
        cap = float(feat["cap"])
        k = rng.poisson(max(cap * 0.15, 1.0), size=n) + 1
        return (np.minimum(k, cap) / cap).astype(np.float32)
    if kind == "log":
        return rng.beta(2.0, 2.5, size=n).astype(np.float32)
    raise ValueError(f"unknown feature kind {kind}")


def sample_events(rng: np.random.Generator, n: int, schema: Dict, mix: Dict[str, float]) -> np.ndarray:
    feats = schema["features"]
    idx = {f["name"]: f["index"] for f in feats}
    names = list(mix.keys())
    scenario = rng.choice(len(names), size=n, p=[mix[k] for k in names])
    X = np.zeros((n, FEATURES), dtype=np.float32)
    for s_i, s_name in enumerate(names):
        rows = np.where(scenario == s_i)[0]
        if rows.size == 0:
            continue
        if s_name == "uniform":
            block = rng.random((rows.size, FEATURES)).astype(np.float32)
            for f in feats:
                if f["kind"] == "binary":
                    block[:, f["index"]] = (block[:, f["index"]] > 0.5).astype(np.float32)
            block *= (rng.random((rows.size, FEATURES)) < UNIFORM_DENSITY)
            X[rows] = block
            continue
        intensity = 0.25 + 1.5 * rng.beta(1.3, 3.5, size=rows.size)
        for f in feats:
            base = P_ACTIVE[s_name][f["group"]] * (BINARY_RATE if f["kind"] == "binary" else 1.0)
            if f["name"] in ("test_files_ratio", "docs_files_ratio") and s_name != "order":
                base = 0.4
            if f["name"] in ALWAYS[s_name]:
                active = rows
            else:
                active = rows[rng.random(rows.size) < np.minimum(base * intensity, 1.0)]
            if active.size:
                X[active, f["index"]] = sample_values(rng, f, active.size)
        if s_name == "build_failure":
            X[rows, idx["build_failed"]] = (rng.random(rows.size) < 0.9).astype(np.float32)
    return X


# ---------------------------------------------------------------------------------------------
# Scenario simulator (synthetic agent sessions)
# ---------------------------------------------------------------------------------------------
class SessionSimulator:
    """Seeded generator of synthetic agent sessions; each session is a [T, 64] array of events."""

    def __init__(self, schema: Dict, rng: np.random.Generator):
        self.schema = schema
        self.norm = Normalizer(schema)
        self.rng = rng

    # -- context shared by the events of one session ------------------------------------------
    def _context(self) -> Dict:
        r = self.rng
        return {
            "minutes": float(r.uniform(5, 600)),
            "review_hours": float(r.uniform(0, 48)),
            "branches": int(r.integers(1, 4)),
            "off_hours": bool(r.random() < 0.15),
            "uncertainty": float(r.beta(2, 7)),
            "retry": 0,
            "orders": [],  # (minute, cents)
            "budget": float(r.choice([20000, 50000, 100000])),
            "fraud": bool(r.random() < 0.3),
        }

    def _exec(self, ctx: Dict, raw: Dict[str, float]) -> Dict[str, float]:
        r = self.rng
        ctx["minutes"] += float(r.exponential(4.0))
        ctx["review_hours"] += float(r.exponential(0.05))
        raw.setdefault("agent_uncertainty", float(np.clip(ctx["uncertainty"] + r.normal(0, 0.05), 0, 1)))
        raw["branch_age_minutes"] = ctx["minutes"]
        raw["hours_since_human_review"] = ctx["review_hours"]
        raw["concurrent_branches"] = ctx["branches"]
        if ctx["off_hours"]:
            raw["off_hours"] = 1
        return raw

    def _edit(self, ctx: Dict) -> np.ndarray:
        r = self.rng
        files = 1 + int(r.poisson(1.2))
        added = int(r.lognormal(3.0, 1.2))
        removed = int(r.lognormal(2.0, 1.3)) if r.random() < 0.7 else 0
        raw = {
            "files_touched": files, "lines_added": added, "lines_removed": removed,
            "bytes_changed": 42 * (added + removed) + int(r.integers(0, 200)),
            "max_path_depth": int(r.integers(2, 7)),
            "files_added": int(r.random() < 0.15),
        }
        if r.random() < 0.4:
            raw["test_files_ratio"] = float(r.beta(2, 3))
        if r.random() < 0.2:
            raw["docs_files_ratio"] = float(r.beta(2, 3))
        for name, p in (("auth_code_touched", 0.03), ("payment_code_touched", 0.02), ("migration_touched", 0.02),
                        ("dependency_manifest_touched", 0.05), ("lockfile_touched", 0.05), ("ci_config_touched", 0.02),
                        ("stub_marker_delta", 0.05), ("network_endpoint_delta", 0.04), ("unsafe_block_delta", 0.02)):
            if r.random() < p:
                raw[name] = 1 if name.endswith("touched") else int(r.integers(1, 3))
        return self.norm.event(self._exec(ctx, raw))

    def _refactor(self, ctx: Dict) -> np.ndarray:
        r = self.rng
        files = int(r.integers(5, 61))
        added = int(r.lognormal(5.5, 0.8))
        removed = int(r.lognormal(5.2, 0.9))
        raw = {
            "files_touched": files, "files_added": int(r.integers(0, 7)), "files_deleted": int(r.integers(0, 4)),
            "lines_added": added, "lines_removed": removed, "bytes_changed": 40 * (added + removed),
            "max_path_depth": int(r.integers(3, 10)), "test_files_ratio": float(r.uniform(0.1, 0.5)),
        }
        if r.random() < 0.3:
            raw["generated_files_ratio"] = float(r.uniform(0, 0.3))
        for name, p in (("dependency_manifest_touched", 0.15), ("lockfile_touched", 0.15), ("build_script_touched", 0.05),
                        ("unsafe_block_delta", 0.05), ("auth_code_touched", 0.05), ("crypto_code_touched", 0.03)):
            if r.random() < p:
                raw[name] = 1 if name.endswith("touched") else int(r.integers(1, 4))
        return self.norm.event(self._exec(ctx, raw))

    def _secret(self, ctx: Dict) -> np.ndarray:
        r = self.rng
        raw = {"files_touched": int(r.integers(1, 4)), "lines_added": int(r.integers(1, 30)),
               "bytes_changed": int(r.integers(40, 2000)), "secret_pattern_hits": int(r.integers(1, 5))}
        if r.random() < 0.7:
            raw["env_file_touched"] = 1
        if r.random() < 0.8:
            raw["unformatted_sensitive_fields"] = 1
        if r.random() < 0.5:
            raw["network_endpoint_delta"] = int(r.integers(1, 4))
        if r.random() < 0.3:
            raw["shell_exec_delta"] = int(r.integers(1, 3))
        if r.random() < 0.1:
            raw["vcs_internal_touched"] = 1
        if r.random() < 0.2:
            raw["crypto_code_touched"] = 1
        if r.random() < 0.2:
            raw["email_hits"] = int(r.integers(1, 4))
        return self.norm.event(self._exec(ctx, raw))

    def _delete(self, ctx: Dict) -> np.ndarray:
        r = self.rng
        deleted = int(r.integers(4, 41))
        removed = int(r.lognormal(6.5, 0.8))
        raw = {"files_deleted": deleted, "files_touched": deleted + int(r.integers(0, 4)), "lines_removed": removed,
               "bytes_changed": 40 * removed, "max_path_depth": int(r.integers(2, 8))}
        if r.random() < 0.1:
            raw["permission_changes"] = 1
        if r.random() < 0.05:
            raw["symlink_changes"] = 1
        if r.random() < 0.05:
            raw["vcs_internal_touched"] = 1
        return self.norm.event(self._exec(ctx, raw))

    def _build_fail(self, ctx: Dict) -> np.ndarray:
        r = self.rng
        ctx["uncertainty"] = float(min(1.0, ctx["uncertainty"] + 0.04))
        raw = {"build_failed": 1, "compile_errors": int(r.lognormal(1.5 + 0.3 * ctx["retry"], 1.0)),
               "compile_warnings": int(r.lognormal(1.5, 1.2)), "retry_count": ctx["retry"],
               "files_touched": int(r.integers(1, 7)), "lines_added": int(r.lognormal(2.5, 1.0)),
               "bytes_changed": int(r.lognormal(6.0, 1.0))}
        if r.random() < 0.4:
            raw["tests_failed_ratio"] = float(r.beta(2, 4))
        ctx["retry"] = min(ctx["retry"] + 1, 6)
        return self.norm.event(self._exec(ctx, raw))

    def _panic(self, ctx: Dict) -> np.ndarray:
        """A command of a random kind; runs of these produce command-entropy spikes."""
        r = self.rng
        choice = int(r.integers(0, 7))
        raw: Dict[str, float] = {"files_touched": int(r.integers(1, 4)), "lines_added": int(r.integers(1, 40))}
        if choice == 0:
            raw["shell_exec_delta"] = int(r.integers(1, 5))
        elif choice == 1:
            raw["network_endpoint_delta"] = int(r.integers(1, 5))
        elif choice == 2:
            raw["ci_config_touched"] = 1
        elif choice == 3:
            raw["infra_config_touched"] = 1
        elif choice == 4:
            raw["permission_changes"] = 1
        elif choice == 5:
            raw["files_deleted"] = int(r.integers(1, 6))
        else:
            raw["build_failed"] = 1
            raw["compile_errors"] = int(r.integers(1, 30))
        raw["agent_uncertainty"] = float(np.clip(ctx["uncertainty"] + 0.3, 0, 1))
        return self.norm.event(self._exec(ctx, raw))

    def _order(self, ctx: Dict, rising: bool) -> np.ndarray:
        r = self.rng
        gap = float(r.exponential(4.0 if rising else 45.0))
        ctx["minutes"] += gap
        now = ctx["minutes"]
        last_hour = sum(1 for m, _ in ctx["orders"] if now - m <= 60)
        spend_24h = sum(c for m, c in ctx["orders"] if now - m <= 1440)
        cents = float(np.clip(r.lognormal(np.log(6000) + (0.4 if ctx["fraud"] else 0.0), 0.9), 100, 2_000_000))
        quantity = int(r.integers(1, 9))
        raw = {
            "order_amount_cents": cents, "budget_utilization": cents / ctx["budget"],
            "line_items": int(r.integers(1, 6)), "quantity_total": quantity,
            "orders_last_hour": last_hour, "spend_last_24h_cents": spend_24h,
            "customer_pii_fields": int(r.integers(0, 4)),
        }
        fraud = ctx["fraud"]
        if r.random() < (0.45 if fraud else 0.08):
            raw["new_payee"] = 1
        raw["payee_country_risk"] = float(r.beta(5, 2) if (fraud and r.random() < 0.5) else r.beta(1, 12))
        if r.random() < (0.25 if fraud else 0.02):
            raw["unformatted_sensitive_fields"] = 1
            raw["customer_pii_fields"] = int(r.integers(2, 7))
        for name, p in (("receipt_unverified", 0.05), ("currency_non_usd", 0.1), ("shipping_mismatch", 0.03),
                        ("digital_goods", 0.2), ("manual_override_requested", 0.02)):
            if r.random() < p * (3 if fraud and name != "currency_non_usd" else 1):
                raw[name] = 1
        if r.random() < 0.3:
            raw["discount_ratio"] = float(r.beta(1, 6))
        if r.random() < 0.2:
            raw["merchant_refund_rate"] = float(r.beta(1, 10))
        ctx["orders"].append((now, cents))
        return self.norm.event(self._exec(ctx, raw))

    def _idle(self, ctx: Dict) -> np.ndarray:
        return np.zeros(FEATURES, dtype=np.float32)

    # -- session scenarios ----------------------------------------------------------------------
    def _pick(self, options: List[Tuple[float, object]], ctx: Dict) -> np.ndarray:
        probs = np.array([p for p, _ in options], dtype=np.float64)
        fn = options[int(self.rng.choice(len(options), p=probs / probs.sum()))][1]
        return fn(ctx)

    def session(self, scenario: str) -> np.ndarray:
        r = self.rng
        ctx = self._context()
        events: List[np.ndarray] = []
        if scenario == "single_event":
            return sample_events(r, 1, self.schema, SIM_CONFIG["single_event_mix"])
        if scenario == "mixed":
            parts = ["benign_edit", "refactor", "secret_leak", "mass_delete", "build_break_loop", "procurement"]
            for name in r.choice(parts, size=int(r.integers(2, 5)), replace=True):
                events.extend(self._segment(str(name), ctx, int(r.integers(3, 30))))
            return np.stack(events)
        bounds = {"benign_edit": (1, 90), "refactor": (8, 90), "secret_leak": (4, 80), "mass_delete": (4, 80),
                  "build_break_loop": (4, 80), "procurement": (1, 64)}[scenario]
        events.extend(self._segment(scenario, ctx, int(r.integers(bounds[0], bounds[1] + 1))))
        return np.stack(events)

    def _segment(self, scenario: str, ctx: Dict, length: int) -> List[np.ndarray]:
        r = self.rng
        out: List[np.ndarray] = []
        background = lambda c: sample_events(r, 1, self.schema, {"workspace_edit": 1.0})[0]
        if scenario == "benign_edit":
            for _ in range(length):
                out.append(self._pick([(0.82, self._edit), (0.06, self._build_fail), (0.05, self._idle),
                                       (0.07, background)], ctx))
                if r.random() < 0.5:
                    ctx["retry"] = 0
        elif scenario == "refactor":
            for _ in range(length):
                out.append(self._pick([(0.6, self._refactor), (0.3, self._edit), (0.1, self._build_fail)], ctx))
                if r.random() < 0.6:
                    ctx["retry"] = 0
        elif scenario == "secret_leak":
            start = int(r.integers(0, length))
            for i in range(length):
                if i < start:
                    out.append(self._edit(ctx))
                else:
                    out.append(self._pick([(0.5, self._secret), (0.4, self._edit), (0.1, self._panic)], ctx))
        elif scenario == "mass_delete":
            start = int(r.integers(0, length))
            burst = int(r.integers(2, 11))
            for i in range(length):
                out.append(self._delete(ctx) if start <= i < start + burst else self._edit(ctx))
        elif scenario == "build_break_loop":
            panic_at = int(r.integers(length // 2, length + 1)) if r.random() < 0.5 else length + 1
            for i in range(length):
                if i >= panic_at:
                    out.append(self._pick([(0.7, self._panic), (0.3, self._build_fail)], ctx))
                elif r.random() < 0.7:
                    out.append(self._build_fail(ctx))
                else:
                    out.append(self._edit(ctx))
                    if r.random() < 0.3:
                        ctx["retry"] = 0
        elif scenario == "procurement":
            rising = bool(r.random() < 0.4)
            for _ in range(length):
                out.append(self._order(ctx, rising) if r.random() < 0.8 else self._pick(
                    [(0.6, self._idle), (0.4, self._edit)], ctx))
        else:
            raise ValueError(f"unknown scenario {scenario}")
        return out


# ---------------------------------------------------------------------------------------------
# Window teacher
# ---------------------------------------------------------------------------------------------
class WindowTeacher:
    """Explicit window label (see the module docstring); works per session on precomputed arrays."""

    def __init__(self, schema: Dict):
        self.schema = schema
        self.idx = {f["name"]: f["index"] for f in schema["features"]}
        self.amount_scale = math.log1p(float(next(f["cap"] for f in schema["features"] if f["name"] == "order_amount_cents")))
        self.groups = {
            "secret": [self.idx[n] for n in ("secret_pattern_hits", "env_file_touched", "vcs_internal_touched")],
            "build": [self.idx[n] for n in ("build_failed", "compile_errors", "tests_failed_ratio")],
            "shell_network": [self.idx[n] for n in ("shell_exec_delta", "network_endpoint_delta")],
            "config": [self.idx[n] for n in ("ci_config_touched", "infra_config_touched", "build_script_touched",
                                              "dependency_manifest_touched", "lockfile_touched", "policy_file_touched",
                                              "migration_touched", "permission_changes", "symlink_changes")],
        }

    def event_arrays(self, events: np.ndarray) -> Dict[str, np.ndarray]:
        i = self.idx
        is_order = events[:, i["order_amount_cents"]] > 0
        cents = np.where(is_order, np.expm1(events[:, i["order_amount_cents"]].astype(np.float64) * self.amount_scale), 0.0)
        types = np.full(len(events), EVENT_TYPES.index("idle"), dtype=np.int64)
        any_active = events.max(axis=1) > 0
        types[any_active] = EVENT_TYPES.index("edit")
        for name in ("config", "shell_network", "build"):
            types[events[:, self.groups[name]].max(axis=1) > 0] = EVENT_TYPES.index(name)
        types[events[:, i["files_deleted"]] > 0] = EVENT_TYPES.index("delete")
        types[events[:, self.groups["secret"]].max(axis=1) > 0] = EVENT_TYPES.index("secret")
        types[is_order] = EVENT_TYPES.index("payment")
        return {
            "z": event_logits(events, self.schema),
            "is_order": is_order.astype(np.float64),
            "cents": cents,
            "secret": (events[:, i["secret_pattern_hits"]] > 0).astype(np.float64),
            "deleted": events[:, i["files_deleted"]].astype(np.float64),
            "types": types,
        }

    @staticmethod
    def _entropy(types: np.ndarray) -> float:
        if types.size == 0:
            return 0.0
        counts = np.bincount(types, minlength=len(EVENT_TYPES)).astype(np.float64)
        p = counts[counts > 0] / types.size
        return float(-(p * np.log2(p)).sum())

    def terms(self, a: Dict[str, np.ndarray], t: int) -> Dict[str, float]:
        """Temporal terms for the window ending at event t (window = the last <= WINDOW events)."""
        cfg = WINDOW_TEACHER
        lo = max(0, t - (WINDOW - 1))
        clip = lambda x: float(min(max(x, 0.0), 1.0))
        newest_order = a["is_order"][t] > 0
        v = cfg["payment_velocity"]
        orders = a["is_order"][max(lo, t - v["lookback_events"]):t].sum()
        velocity = v["weight"] * clip((orders - v["free_orders"]) / (v["saturate_orders"] - v["free_orders"]))
        s = cfg["cumulative_spend"]
        usd = a["cents"][lo:t].sum() / 100.0
        spend = s["weight"] * clip((math.log10(1.0 + usd) - s["log10_dollars_start"]) / s["log10_dollars_span"])
        k = cfg["repeated_secret_hits"]
        secrets = k["weight"] * clip(a["secret"][lo:t].sum() / k["saturate_events"])
        d = cfg["deletion_burst"]
        deletes = d["weight"] * clip((a["deleted"][max(lo, t - d["lookback_events"]):t].sum() - d["start"]) / d["span"])
        e = cfg["command_entropy_spike"]
        recent_lo = max(lo, t - e["recent_events"] + 1)
        recent = a["types"][recent_lo:t + 1]
        base = a["types"][max(lo, recent_lo - e["baseline_events"]):recent_lo]
        spike = self._entropy(recent) - self._entropy(base)
        entropy = e["weight"] * clip((spike - e["free_bits"]) / e["span_bits"])
        return {
            "payment_velocity": velocity if newest_order else 0.0,
            "cumulative_spend": spend if newest_order else 0.0,
            "repeated_secret_hits": secrets,
            "deletion_burst": deletes,
            "command_entropy_spike": entropy,
        }

    def label(self, a: Dict[str, np.ndarray], t: int) -> Tuple[float, float, Dict[str, float]]:
        """(window label, newest-event-only label, terms) for the window ending at event t."""
        terms = self.terms(a, t)
        z_event = a["z"][t]
        z = z_event + sum(terms.values())
        return float(squash(np.array([z]), self.schema)[0]), float(squash(np.array([z_event]), self.schema)[0]), terms


def window_tensor(events: np.ndarray, t: int) -> np.ndarray:
    """[64 features, 64 positions] float16: the last <= 64 events ending at t, right-aligned."""
    lo = max(0, t - (WINDOW - 1))
    seg = events[lo:t + 1]
    out = np.zeros((FEATURES, WINDOW), dtype=np.float16)
    out[:, WINDOW - len(seg):] = seg.T
    return out


def build_dataset(schema: Dict) -> Dict:
    cfg = SIM_CONFIG
    rng = np.random.default_rng(cfg["seed"])
    sim = SessionSimulator(schema, rng)
    teacher = WindowTeacher(schema)
    names = list(cfg["scenario_mix"].keys())
    probs = np.array([cfg["scenario_mix"][k] for k in names])
    scenario_ids = rng.choice(len(names), size=cfg["sessions"], p=probs / probs.sum())
    val_sessions = set(rng.choice(cfg["sessions"], size=int(cfg["sessions"] * cfg["val_fraction"]), replace=False).tolist())
    split = {"train": {"X": [], "y": [], "y_event": [], "length": [], "scenario": [], "session": [], "cut": []},
             "val": {"X": [], "y": [], "y_event": [], "length": [], "scenario": [], "session": [], "cut": []}}
    sessions: Dict[int, np.ndarray] = {}
    term_totals = {k: 0 for k in WINDOW_TEACHER}
    events_total = 0
    for s, sc in enumerate(scenario_ids):
        scenario = names[sc]
        events = sim.session(scenario)
        events_total += len(events)
        arrays = teacher.event_arrays(events)
        T = len(events)
        cuts = {T - 1}
        if T > 1:
            cuts.update(int(c) for c in rng.integers(0, T, size=cfg["extra_cuts_per_session"]))
            if rng.random() < cfg["first_event_cut_probability"]:
                cuts.add(0)
        part = split["val" if s in val_sessions else "train"]
        if s in val_sessions:
            sessions[s] = events
        for t in sorted(cuts):
            y, y_event, terms = teacher.label(arrays, t)
            for k, v in terms.items():
                term_totals[k] += int(v > 0)
            samples = [(window_tensor(events, t), y, min(t + 1, WINDOW))]
            if t > 0 and rng.random() < cfg["zero_history_copy_probability"]:
                # the same newest event as a session with no history (label = per-event teacher)
                samples.append((window_tensor(events[t:t + 1], 0), y_event, 1))
            for x, label, length in samples:
                part["X"].append(x)
                part["y"].append(label)
                part["y_event"].append(y_event)
                part["length"].append(length)
                part["scenario"].append(sc)
                part["session"].append(s)
                part["cut"].append(t)
    out = {"scenarios": names, "val_sessions": sessions, "events_total": events_total,
           "windows_with_term": term_totals}
    for key, part in split.items():
        out[key] = {
            "X": np.stack(part["X"]),
            "y": np.array(part["y"], dtype=np.float64),
            "y_event": np.array(part["y_event"], dtype=np.float64),
            "length": np.array(part["length"], dtype=np.int64),
            "scenario": np.array(part["scenario"], dtype=np.int64),
            "session": np.array(part["session"], dtype=np.int64),
            "cut": np.array(part["cut"], dtype=np.int64),
        }
    return out


# ---------------------------------------------------------------------------------------------
# Architecture: shared definition for PyTorch (training, FP32 reference) and MIL (CoreML export)
# ---------------------------------------------------------------------------------------------
def horizon_mask(arch: Dict) -> np.ndarray:
    """[1, 1, K, W]: view k keeps the newest horizons[k] positions."""
    horizons = arch["horizons"]
    mask = np.zeros((1, 1, len(horizons), WINDOW), dtype=np.float32)
    for k, h in enumerate(horizons):
        mask[0, 0, k, WINDOW - h:] = 1.0
    return mask


def conv_layers(arch: Dict) -> List[Tuple[str, Tuple[int, int, int, int], int]]:
    """(name, weight shape [out, in, kh, kw], width dilation) of every convolution, in order."""
    H, K, R = arch["hidden"], len(arch["horizons"]), arch["readout_hidden"]
    layers = [("stem", (H, FEATURES, 1, 1), 1)]
    for i, blk in enumerate(arch["blocks"]):
        layers.append((f"blocks.{i}.a", (H, H, 1, blk["k"]), blk["dilation"]))
        layers.append((f"blocks.{i}.b", (H, H, 1, blk.get("k_b", 1)), 1))
    readout_in = 2 * H if arch["readout"] == "newest+mean" else H
    layers.append(("head.0", (R, readout_in, K, 1), 1))
    layers.append(("head.1", (1, R, 1, 1), 1))
    return layers


def arch_stats(arch: Dict, batch: int = 1) -> Dict:
    K = len(arch["horizons"])
    params = 0
    macs = 0
    for name, (o, i, kh, kw), _ in conv_layers(arch):
        params += o * i * kh * kw + o
        positions = 1 if name.startswith("head") else K * WINDOW
        macs += o * i * kh * kw * positions
    return {"conv_layers": len(conv_layers(arch)), "parameters": params, "macs_per_window": macs,
            "macs_per_prediction": macs * batch, "views": K}


def random_params(arch: Dict, seed: int) -> Dict[str, np.ndarray]:
    """He-initialised weights (sweep only: placement and latency do not depend on weight values)."""
    rng = np.random.default_rng(seed)
    params = {}
    for name, shape, _ in conv_layers(arch):
        fan_in = shape[1] * shape[2] * shape[3]
        scale = math.sqrt(2.0 / fan_in) * (0.2 if name.endswith(".b") else 1.0)
        params[f"{name}.weight"] = rng.normal(0, scale, size=shape).astype(np.float32)
        params[f"{name}.bias"] = np.zeros(shape[0], dtype=np.float32)
    return params


def torch_model(arch: Dict):
    """PyTorch twin of the MIL program (training and the FP32 reference).

    `forward` computes every 1xK (dilated) convolution as a channels-last GEMM over K shifted taps,
    which is several times faster on the CPU than torch's conv2d for these shapes; `conv_forward`
    is the direct nn.functional.conv2d form of the same network (identical to the MIL ops) and the
    build checks that both agree.
    """
    import torch
    from torch import nn
    import torch.nn.functional as F

    layers = {name: (shape, dil) for name, shape, dil in conv_layers(arch)}

    class WindowNet(nn.Module):
        def __init__(self):
            super().__init__()
            self.register_buffer("mask", torch.from_numpy(horizon_mask(arch)))

            def conv(name):
                (o, i, kh, kw), _ = layers[name]
                return nn.Conv2d(i, o, (kh, kw))

            self.stem = conv("stem")
            self.blocks = nn.ModuleList(
                nn.ModuleDict({"a": conv(f"blocks.{i}.a"), "b": conv(f"blocks.{i}.b")}) for i in range(len(arch["blocks"])))
            self.head = nn.ModuleList([conv("head.0"), conv("head.1")])
            with torch.no_grad():  # residual branches start small
                for blk in self.blocks:
                    blk["b"].weight.mul_(0.2)

        def _modules_in_order(self):
            yield "stem", self.stem
            for i, blk in enumerate(self.blocks):
                yield f"blocks.{i}.a", blk["a"]
                yield f"blocks.{i}.b", blk["b"]

        @staticmethod
        def _gemm(h, module, dilation):
            w, b = module.weight, module.bias
            o, i, _, kw = w.shape
            if kw > 1:
                width, pad = h.shape[2], dilation * (kw - 1) // 2
                hp = F.pad(h, (0, 0, pad, pad))
                h = torch.cat([hp[:, :, j * dilation:j * dilation + width] for j in range(kw)], dim=-1)
                w = w.permute(0, 3, 1, 2).reshape(o, kw * i)
            else:
                w = w.reshape(o, i)
            return torch.addmm(b, h.reshape(-1, h.shape[-1]), w.t()).reshape(*h.shape[:-1], o)

        def forward(self, x):  # x [N, 64, 1, W] -> logit [N]
            h = x.squeeze(2).transpose(1, 2).unsqueeze(1) * self.mask.permute(0, 2, 3, 1)  # [N, K, W, C]
            h = torch.relu(self._gemm(h, self.stem, 1))
            for i, blk in enumerate(self.blocks):
                y = torch.relu(self._gemm(h, blk["a"], layers[f"blocks.{i}.a"][1]))
                h = torch.relu(h + self._gemm(y, blk["b"], 1))
            z = torch.cat([h[:, :, -1], h.mean(dim=2)], dim=-1) if arch["readout"] == "newest+mean" else h[:, :, -1]
            w0 = self.head[0].weight  # [R, C_readout, K, 1]
            z = torch.relu(torch.addmm(self.head[0].bias, z.reshape(z.shape[0], -1),
                                       w0.permute(0, 2, 1, 3).reshape(w0.shape[0], -1).t()))
            return torch.addmm(self.head[1].bias, z, self.head[1].weight.reshape(1, -1).t()).reshape(-1)

        def conv_forward(self, x):  # same network with nn.functional.conv2d, op for op like the MIL program
            def conv(h, name, module):
                (_, _, kh, kw), dil = layers[name]
                return F.conv2d(h, module.weight, module.bias, padding=(0, dil * (kw - 1) // 2), dilation=(1, dil))
            h = torch.relu(conv(x * self.mask, "stem", self.stem))
            for i, blk in enumerate(self.blocks):
                y = torch.relu(conv(h, f"blocks.{i}.a", blk["a"]))
                h = torch.relu(h + conv(y, f"blocks.{i}.b", blk["b"]))
            z = h[..., -1:]
            if arch["readout"] == "newest+mean":
                z = torch.cat([z, h.mean(dim=3, keepdim=True)], dim=1)
            z = torch.relu(F.conv2d(z, self.head[0].weight, self.head[0].bias))
            return F.conv2d(z, self.head[1].weight, self.head[1].bias).reshape(-1)

    return WindowNet()


def gemm_conv_agreement(net, X: np.ndarray) -> float:
    """max |GEMM forward - conv2d forward| over windows X (logit units)."""
    import torch
    net.eval()
    with torch.no_grad():
        xb = torch.from_numpy(X.astype(np.float32)).unsqueeze(2)
        return float((net(xb) - net.conv_forward(xb)).abs().max())


def params_from_torch(net) -> Dict[str, np.ndarray]:
    return {k: v.detach().cpu().numpy().astype(np.float32) for k, v in net.state_dict().items() if k != "mask"}


def build_coreml(params: Dict[str, np.ndarray], arch: Dict, schema: Dict, batch: int, metadata: Dict[str, str]):
    """FP16 ML Program from weights. batch == 1 -> online window layout, batch > 1 -> window_batch."""
    import coremltools as ct
    from coremltools.converters.mil import Builder as mb
    from coremltools.converters.mil.mil import types

    H, K = arch["hidden"], len(arch["horizons"])
    p16 = {k: v.astype(np.float16) for k, v in params.items()}
    mask = horizon_mask(arch).astype(np.float16)
    dil = {name: d for name, _, d in conv_layers(arch)}
    assert schema["input_name"] == "features" and schema["output_name"] == "risk_score"

    def conv(x, name):
        w = p16[f"{name}.weight"]
        kw, d = w.shape[3], dil[name]
        pad = d * (kw - 1) // 2
        return mb.conv(x=x, weight=w, bias=p16[f"{name}.bias"], strides=[1, 1], dilations=[1, d],
                       pad_type="custom", pad=[0, 0, pad, pad], name=name.replace(".", "_"))

    @mb.program(input_specs=[mb.TensorSpec(shape=(batch, FEATURES, 1, WINDOW), dtype=types.fp16)],
                opset_version=ct.target.macOS14)
    def prog(features):
        x = mb.mul(x=features, y=mask, name="horizon_views")
        h = mb.relu(x=conv(x, "stem"), name="stem_relu")
        for i in range(len(arch["blocks"])):
            y = mb.relu(x=conv(h, f"blocks.{i}.a"), name=f"blocks_{i}_a_relu")
            y = conv(y, f"blocks.{i}.b")
            h = mb.relu(x=mb.add(x=h, y=y, name=f"blocks_{i}_add"), name=f"blocks_{i}_relu")
        z = mb.slice_by_index(x=h, begin=[0, 0, 0, WINDOW - 1], end=[batch, H, K, WINDOW], name="newest")
        if arch["readout"] == "newest+mean":
            mean = mb.reduce_mean(x=h, axes=[3], keep_dims=True, name="window_mean")
            z = mb.concat(values=[z, mean], axis=1, name="readout_features")
        z = mb.relu(x=conv(z, "head.0"), name="head_0_relu")
        prob = mb.sigmoid(x=conv(z, "head.1"), name="probability")
        return mb.mul(x=prob, y=np.float16(100.0), name="risk_score")

    model = ct.convert(prog, convert_to="mlprogram", minimum_deployment_target=ct.target.macOS14,
                       compute_precision=ct.precision.FLOAT16, compute_units=ct.ComputeUnit.CPU_AND_NE)
    model.author = "AegisFlow harness (models/build_model.py)"
    model.version = f"{schema['version']}.1"
    model.short_description = (
        "AegisFlow context-window risk scorer: the last 64 events of an agent session (64 normalised features "
        "each) -> risk_score 0..100 for the newest event. FP16 ML Program. Trained on SYNTHETIC sessions "
        "labelled by a documented teacher (models/build_model.py); not real telemetry.")
    if batch == 1:
        model.input_description["features"] = ("[0, feature, 0, position]: last 64 events of one session, oldest -> "
                                               "newest, zero-padded on the left; feature order models/feature_schema.json")
        model.output_description["risk_score"] = "risk score 0..100 for the newest event (>= 35 is high impact)"
    else:
        model.input_description["features"] = (f"[session, feature, 0, position]: {batch} independent sessions, each "
                                               "the last 64 events, oldest -> newest, zero-padded on the left")
        model.output_description["risk_score"] = "risk score 0..100 for the newest event of each session"
    model.user_defined_metadata.update(metadata)
    model.user_defined_metadata[LAYOUT_KEY] = "window" if batch == 1 else "window_batch"
    model.user_defined_metadata["window_length"] = str(WINDOW)
    model.user_defined_metadata["feature_schema_sha256"] = hashlib.sha256(SCHEMA_PATH.read_bytes()).hexdigest()
    return model


def save_and_compile(model, package: Path, compiled: Path) -> None:
    from coremltools.models.utils import compile_model
    if package.exists():
        shutil.rmtree(package)
    model.save(str(package))
    if compiled.exists():
        shutil.rmtree(compiled)
    compile_model(str(package), destination_path=str(compiled))


# ---------------------------------------------------------------------------------------------
# Placement + latency
# ---------------------------------------------------------------------------------------------
def plan_counts(compiled: Path) -> Dict:
    """MLComputePlan (coremltools API) preferred device per dispatched op for .cpuAndNeuralEngine."""
    import coremltools as ct
    from coremltools.models.compute_plan import MLComputePlan
    plan = MLComputePlan.load_from_path(path=str(compiled), compute_units=ct.ComputeUnit.CPU_AND_NE)
    counts = {"neural_engine": 0, "cpu": 0, "gpu": 0}
    ops = []
    names = {"MLNeuralEngineComputeDevice": "neural_engine", "MLCPUComputeDevice": "cpu", "MLGPUComputeDevice": "gpu"}
    for fn in plan.model_structure.program.functions.values():
        for op in fn.block.operations:
            usage = plan.get_compute_device_usage_for_mlprogram_operation(op)
            if usage is None:
                continue
            device = names.get(type(usage.preferred_compute_device).__name__, "unknown")
            counts[device] = counts.get(device, 0) + 1
            ops.append({"operator": op.operator_name, "preferred": device})
    dispatched = sum(counts.values())
    return {"dispatched": dispatched, **counts, "all_on_neural_engine": dispatched > 0 and counts["neural_engine"] == dispatched,
            "ops": ops}


def probe_binary() -> Path:
    """Compiles the production Swift bridge + models/tools/ane_probe.swift (cached by source hash)."""
    digest = hashlib.sha256(BRIDGE_SWIFT.read_bytes() + PROBE_SWIFT.read_bytes()).hexdigest()[:16]
    out_dir = Path(tempfile.gettempdir()) / f"aegis_ane_probe_{digest}"
    binary = out_dir / "ane_probe"
    if binary.exists():
        return binary
    out_dir.mkdir(parents=True, exist_ok=True)
    cmd = ["xcrun", "swiftc", "-O", "-wmo", "-parse-as-library", "-module-name", "AegisANEProbe",
           "-module-cache-path", str(out_dir / "module-cache"), str(BRIDGE_SWIFT), str(PROBE_SWIFT), "-o", str(binary)]
    subprocess.run(cmd, check=True)
    return binary


def probe(compiled: List[Path]) -> List[Dict]:
    cmd = [str(probe_binary()), *map(str, compiled), "--rounds", str(PROBE_SETTINGS["rounds"]),
           "--per-round", str(PROBE_SETTINGS["per_round"]), "--warmup", str(PROBE_SETTINGS["warmup"])]
    res = subprocess.run(cmd, capture_output=True, text=True)
    reports = [json.loads(line) for line in res.stdout.splitlines() if line.strip()]
    if len(reports) != len(compiled):
        raise RuntimeError(f"ane_probe failed (exit {res.returncode}): {res.stderr.strip()[:500]}")
    return reports


def summarize_probe(report: Dict) -> Dict:
    if "error" in report:
        return {"error": report["error"]}
    plan, lat = report["plan"], report["latency_ms"]
    return {
        "placement": {k: plan[k] for k in ("operations_dispatched", "neural_engine", "cpu", "gpu", "all_on_neural_engine")},
        "not_on_neural_engine_by_operator": {
            op: sum(1 for o in plan["not_on_neural_engine"] if o["operator"] == op)
            for op in sorted({o["operator"] for o in plan["not_on_neural_engine"]})},
        "cpu_and_neural_engine_ms": {k: lat["cpu_and_neural_engine"][k] for k in ("n", "p50", "p95", "mean")},
        "cpu_only_ms": {k: lat["cpu_only"][k] for k in ("n", "p50", "p95", "mean")},
        "speedup_p50": report["speedup_p50_cpu_only_over_ane"],
        "load_average_1m": round(report["load_average_1m_at_end"], 2),
    }


def passes(summary: Dict, stats: Dict) -> Dict[str, bool]:
    if "error" in summary:
        return {"all_on_neural_engine": False, "p50_below_limit": False, "faster_than_cpu_only": False,
                "speedup_margin": False, "conv_layers_in_range": False}
    lo, hi = SELECTION["allowed_conv_layers"]
    return {
        "all_on_neural_engine": bool(summary["placement"]["all_on_neural_engine"]),
        "p50_below_limit": summary["cpu_and_neural_engine_ms"]["p50"] < SELECTION["max_p50_ms"],
        "faster_than_cpu_only": summary["speedup_p50"] > 1.0,
        "speedup_margin": summary["speedup_p50"] >= SELECTION["min_speedup"],
        "conv_layers_in_range": lo <= stats["conv_layers"] <= hi,
    }


# ---------------------------------------------------------------------------------------------
# Architecture sweep
# ---------------------------------------------------------------------------------------------
def candidate(name: str, hidden: int, blocks: int, horizons: List[int], kernels: Tuple[int, int] = (3, 1),
              dilated: bool = True, readout: str = "newest+mean") -> Dict:
    return {
        "name": name, "hidden": hidden,
        "blocks": [{"k": kernels[0], "k_b": kernels[1],
                    "dilation": (min(2 ** i, WINDOW // 2) if dilated and kernels[0] > 1 else 1)} for i in range(blocks)],
        "horizons": horizons, "readout": readout, "readout_hidden": 64,
    }


V2, V4, V7, V8 = [64, 1], [64, 32, 16, 8], [64, 32, 16, 8, 4, 2, 1], [64, 48, 32, 24, 16, 8, 4, 1]
SWEEP_CANDIDATES = [
    # single view (plain temporal conv network); first design = p128_b5
    candidate("p128_b5", 128, 5, [64]),
    candidate("p64_b5", 64, 5, [64]),
    candidate("p192_b5", 192, 5, [64]),
    candidate("p256_b5", 256, 5, [64]),
    candidate("p384_b5", 384, 5, [64]),
    candidate("p512_b5", 512, 5, [64]),
    candidate("p512_b6", 512, 6, [64]),
    candidate("p512_b7", 512, 7, [64]),
    candidate("p128_b3", 128, 3, [64]),
    candidate("p256_b3", 256, 3, [64]),
    candidate("p128_b7", 128, 7, [64]),
    candidate("p256_b5_k11", 256, 5, [64], kernels=(1, 1)),
    candidate("p256_b5_k33", 256, 5, [64], kernels=(3, 3)),
    candidate("p256_b5_newest", 256, 5, [64], readout="newest"),
    # multi-horizon views sharing the tower
    candidate("mh64_b5_v4", 64, 5, V4),
    candidate("mh64_b5_v8", 64, 5, V8),
    candidate("mh128_b5_v2", 128, 5, V2),
    candidate("mh128_b5_v4", 128, 5, V4),
    candidate("mh128_b5_v7", 128, 5, V7),
    candidate("mh128_b5_v8", 128, 5, V8),
    candidate("mh128_b6_v7", 128, 6, V7),
    candidate("mh128_b6_v8", 128, 6, V8),
    candidate("mh128_b5_v7_newest", 128, 5, V7, readout="newest"),
    candidate("mh128_b5_v7_k33", 128, 5, V7, kernels=(3, 3)),
    candidate("mh192_b5_v4", 192, 5, V4),
    candidate("mh192_b5_v7", 192, 5, V7),
    candidate("mh256_b5_v4", 256, 5, V4),
    # undilated towers (the dilated 1x3 convolutions above are slow on the Neural Engine)
    candidate("p512_b7_nd", 512, 7, [64], dilated=False),
    candidate("p768_b5_nd", 768, 5, [64], dilated=False),
    candidate("mh128_b5_v4_nd", 128, 5, V4, dilated=False),
    candidate("mh128_b5_v7_nd", 128, 5, V7, dilated=False),
    candidate("mh128_b5_v8_nd", 128, 5, V8, dilated=False),
    candidate("mh128_b6_v8_nd", 128, 6, V8, dilated=False),
    candidate("mh128_b5_v8_k33_nd", 128, 5, V8, kernels=(3, 3), dilated=False),
    candidate("mh160_b5_v4_nd", 160, 5, V4, dilated=False),
    candidate("mh160_b5_v7_nd", 160, 5, V7, dilated=False),
    candidate("mh192_b5_v4_nd", 192, 5, V4, dilated=False),
    candidate("mh192_b6_v4_nd", 192, 6, V4, dilated=False),
    candidate("mh256_b5_v2_nd", 256, 5, V2, dilated=False),
    candidate("mh256_b5_v4_nd", 256, 5, V4, dilated=False),
]
ARCH = next(c for c in SWEEP_CANDIDATES if c["name"] == SELECTED_ARCH)


def newest_receptive_field(arch: Dict) -> int:
    """Events the newest-position readout sees: 1x1 stem, then every block widens it by
    dilation * (k - 1) + (k_b - 1); capped by the window (the window-mean readout covers each whole view)."""
    return min(WINDOW, 1 + sum(b["dilation"] * (b["k"] - 1) + b.get("k_b", 1) - 1 for b in arch["blocks"]))


def arch_description(arch: Dict) -> str:
    """Network summary generated from the architecture dict, so the card cannot claim a layout the build
    did not emit (e.g. 'dilated' for an undilated tower)."""
    dilations = [b["dilation"] for b in arch["blocks"]]
    conv_a = "|".join(f"conv1x{k}" for k in sorted({b["k"] for b in arch["blocks"]}))
    conv_b = "|".join(f"conv1x{k}" for k in sorted({b.get("k_b", 1) for b in arch["blocks"]}))
    dilation = f"dilations {dilations}" if max(dilations, default=1) > 1 else "undilated (dilation 1 in every block)"
    k = len(arch["horizons"])
    readout = ("concat(newest position, mean over width)" if arch["readout"] == "newest+mean"
               else "newest position")
    return (f"x * horizon_mask -> {k} views on the height axis (horizons {arch['horizons']}); 1x1 stem conv; "
            f"{len(arch['blocks'])} residual blocks relu(h + {conv_b}(relu({conv_a}(h)))), {dilation}; "
            f"newest-position receptive field {newest_receptive_field(arch)} events; readout {readout} per view "
            f"-> ({k} x 1) conv -> relu -> 1x1 conv -> sigmoid x 100")


def arch_config(arch: Dict) -> Dict:
    return {
        "hidden": arch["hidden"],
        "residual_blocks": len(arch["blocks"]),
        "block_kernels": sorted({f"1x{b['k']}+1x{b.get('k_b', 1)}" for b in arch["blocks"]}),
        "dilations": [b["dilation"] for b in arch["blocks"]],
        "horizons": arch["horizons"],
        "readout": arch["readout"],
        "readout_hidden": arch["readout_hidden"],
    }


def run_sweep(schema: Dict, only: Optional[List[str]]) -> int:
    cands = [c for c in SWEEP_CANDIDATES if not only or c["name"] in only]
    rows = []
    with tempfile.TemporaryDirectory(prefix="aegis_sweep_") as tmp:
        for arch in cands:
            i = next(n for n, c in enumerate(SWEEP_CANDIDATES) if c["name"] == arch["name"])
            stats = arch_stats(arch)
            package, compiled = Path(tmp) / f"{arch['name']}.mlpackage", Path(tmp) / f"{arch['name']}.mlmodelc"
            model = build_coreml(random_params(arch, seed=SIM_CONFIG["seed"] + i), arch, schema, 1, {"sweep": "random weights"})
            save_and_compile(model, package, compiled)
            summary = summarize_probe(probe([compiled])[0])
            checks = passes(summary, stats)
            row = {"name": arch["name"], "config": arch_config(arch), **stats, **summary, "checks": checks,
                   "passes": checks["all_on_neural_engine"] and checks["p50_below_limit"] and checks["speedup_margin"]
                   and checks["conv_layers_in_range"]}
            rows.append(row)
            if "error" in summary:
                print(f"  {arch['name']:22s} error: {summary['error']}", flush=True)
            else:
                pl, a, c = summary["placement"], summary["cpu_and_neural_engine_ms"], summary["cpu_only_ms"]
                print(f"  {arch['name']:22s} convs {stats['conv_layers']:2d} params {stats['parameters']:9d} "
                      f"MACs {stats['macs_per_window'] / 1e6:7.1f}M  ANE {pl['neural_engine']:2d} CPU {pl['cpu']:2d} "
                      f"GPU {pl['gpu']}  p50/p95 ane {a['p50']:.4f}/{a['p95']:.4f} cpu {c['p50']:.4f}/{c['p95']:.4f} "
                      f"speedup {summary['speedup_p50']:.3f} load {summary['load_average_1m']}  "
                      f"{'PASS' if row['passes'] else 'fail'}", flush=True)
            shutil.rmtree(package, ignore_errors=True)
            shutil.rmtree(compiled, ignore_errors=True)
    passing = sorted((r for r in rows if r["passes"]), key=lambda r: (r["macs_per_window"], r["parameters"]))
    selected = passing[0]["name"] if passing else None
    previous = json.loads(SWEEP.read_text()) if (only and SWEEP.exists()) else {"rows": []}
    run = {"at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
           "candidates": [r["name"] for r in rows], "probe_settings": dict(PROBE_SETTINGS), "host": host_facts()}
    doc = sweep_document(previous, rows, run)
    SWEEP.write_text(json.dumps(doc, indent=1) + "\n")
    print(f"sweep: {len(rows)} candidates measured, {len(passing)} pass; smallest passing in this run: {selected}; "
          f"overall selection: {doc['selected']} -> {SWEEP.name}")
    return 0


def sweep_document(previous: Dict, rows: List[Dict], run: Dict) -> Dict:
    """arch_sweep.json: the rows measured by `run` merged by name into `previous` (the old file for --sweep-only,
    {"rows": []} otherwise).

    Rows of one file can come from different runs, so every row carries the measured_at and probe_settings of the
    run that measured it and `runs` lists each run with its host. Rows merged from a file written before rows had
    their own stamp keep measured_at None plus measured_no_later_than (when that old file was written)."""
    stamped = [{**r, "measured_at": run["at"], "probe_settings": run["probe_settings"]} for r in rows]
    legacy_written_at = previous.get("last_run_at") or previous.get("measured_at")
    merged = {}
    for r in previous.get("rows", []):
        if "measured_at" not in r:
            r = {**r, "measured_at": None, "measured_no_later_than": legacy_written_at, "probe_settings": None}
        merged[r["name"]] = r
    merged.update({r["name"]: r for r in stamped})
    all_rows = [merged[c["name"]] for c in SWEEP_CANDIDATES if c["name"] in merged]
    passing_all = sorted((r for r in all_rows if r["passes"]), key=lambda r: (r["macs_per_window"], r["parameters"]))
    stamps = {r.get("measured_at") for r in all_rows}
    runs = [r for r in [*previous.get("runs", []), run] if r["at"] in stamps]
    return {
        "description": ("Architecture sweep for the online window model. Each candidate is built with seeded random "
                        "(He) weights; MLComputePlan placement and latency do not depend on weight values for dense "
                        "FP16 convolutions, and the final trained model is re-measured by the full build. Latency is "
                        "the wall-clock time of aegis_ane_score_windows through the production Swift bridge "
                        "(models/tools/ane_probe.swift), .cpuAndNeuralEngine and .cpuOnly interleaved in alternating "
                        "rounds. Measured on a shared machine: preliminary numbers. --sweep-only runs merge into the "
                        "previous file, so rows may come from different runs: each row's measured_at and "
                        "load_average_1m say when and under what load it was measured; compare rows from different "
                        "runs with care."),
        "selection_rule": {**SELECTION, "order": "fewest MACs per window, then fewest parameters, among passing rows"},
        "last_run_at": run["at"],
        "runs": runs,
        "rows_without_own_timestamp": [r["name"] for r in all_rows if r.get("measured_at") is None],
        "selected": passing_all[0]["name"] if passing_all else None,
        "rows": all_rows,
    }


def host_facts() -> Dict:
    def sysctl(name: str) -> Optional[str]:
        try:
            return subprocess.run(["sysctl", "-n", name], capture_output=True, text=True, check=True).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            return None
    try:
        macos = subprocess.run(["sw_vers", "-productVersion"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        macos = None
    return {"cpu": sysctl("machdep.cpu.brand_string"), "model": sysctl("hw.model"), "macos": macos}


# ---------------------------------------------------------------------------------------------
# Training
# ---------------------------------------------------------------------------------------------
def train(data: Dict, arch: Dict) -> Tuple[object, List[Dict]]:
    import torch
    cfg = TRAIN_CONFIG
    torch.manual_seed(cfg["seed"])
    torch.set_num_threads(cfg["torch_threads"])
    torch.use_deterministic_algorithms(True)
    net = torch_model(arch)
    X, y = data["train"]["X"], data["train"]["y"]
    agreement = gemm_conv_agreement(net, X[:64])
    assert agreement < 1e-4, f"GEMM and conv2d forms disagree by {agreement}"
    weights = np.where(data["train"]["length"] == 1, cfg["zero_history_weight"], 1.0)
    steps_per_epoch = math.ceil(len(X) / cfg["batch_size"])
    opt = torch.optim.AdamW(net.parameters(), lr=cfg["lr"], weight_decay=cfg["weight_decay"])
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=cfg["lr"], total_steps=cfg["epochs"] * steps_per_epoch,
                                                pct_start=0.15)
    rng = np.random.default_rng(cfg["seed"])
    history = []
    for epoch in range(cfg["epochs"]):
        net.train()
        started = time.time()
        order = rng.permutation(len(X))
        total = 0.0
        for start in range(0, len(X), cfg["batch_size"]):
            b = order[start:start + cfg["batch_size"]]
            xb = torch.from_numpy(X[b].astype(np.float32)).unsqueeze(2)
            yb = torch.from_numpy(y[b].astype(np.float32))
            wb = torch.from_numpy(weights[b].astype(np.float32))
            loss = (torch.nn.functional.binary_cross_entropy_with_logits(net(xb), yb, reduction="none") * wb).mean()
            opt.zero_grad()
            loss.backward()
            opt.step()
            sched.step()
            total += loss.item() * len(b)
        val = predict_torch(net, data["val"]["X"])
        mae = float(np.mean(np.abs(val - data["val"]["y"] * 100.0)))
        history.append({"epoch": epoch + 1, "train_loss": round(total / len(X), 5), "val_mae_points": round(mae, 4),
                        "seconds": round(time.time() - started, 1)})
        print(f"  epoch {epoch + 1:2d}  loss {total / len(X):.5f}  val MAE {mae:.3f} pts  ({time.time() - started:.0f}s)",
              flush=True)
    return net, history


def predict_torch(net, X: np.ndarray, batch: int = 512) -> np.ndarray:
    """FP32 reference scores (0..100) for windows [N, 64, 64]."""
    import torch
    net.eval()
    out = []
    with torch.no_grad():
        for start in range(0, len(X), batch):
            xb = torch.from_numpy(X[start:start + batch].astype(np.float32)).unsqueeze(2)
            out.append(torch.sigmoid(net(xb)).numpy().astype(np.float64) * 100.0)
    return np.concatenate(out)


def roc_auc(scores: np.ndarray, positive: np.ndarray) -> Optional[float]:
    pos, neg = int(positive.sum()), int((~positive).sum())
    if pos == 0 or neg == 0:
        return None
    order = np.argsort(scores, kind="mergesort")
    ranks = np.empty(len(scores), dtype=np.float64)
    sorted_scores = scores[order]
    i = 0
    while i < len(scores):
        j = i
        while j + 1 < len(scores) and sorted_scores[j + 1] == sorted_scores[i]:
            j += 1
        ranks[order[i:j + 1]] = (i + j) / 2.0 + 1.0
        i = j + 1
    return float((ranks[positive].sum() - pos * (pos + 1) / 2.0) / (pos * neg))


def metrics(pred: np.ndarray, target: np.ndarray, threshold: float) -> Dict:
    p, t = pred >= threshold, target >= threshold
    tp, fp, fn = int(np.sum(p & t)), int(np.sum(p & ~t)), int(np.sum(~p & t))
    return {
        "windows": int(len(pred)),
        "mae_points": round(float(np.mean(np.abs(pred - target))), 4),
        "rmse_points": round(float(np.sqrt(np.mean((pred - target) ** 2))), 4),
        "max_abs_error_points": round(float(np.max(np.abs(pred - target))), 4),
        "accuracy_at_threshold": round(float(np.mean(p == t)), 5),
        "precision": round(tp / (tp + fp), 5) if tp + fp else None,
        "recall": round(tp / (tp + fn), 5) if tp + fn else None,
        "auc_vs_teacher_high_impact": None if roc_auc(pred, t) is None else round(roc_auc(pred, t), 5),
        "teacher_high_impact_rate": round(float(np.mean(t)), 5),
    }


def split_metrics(pred: np.ndarray, target: np.ndarray, zero: np.ndarray, threshold: float) -> Dict:
    """metrics() over all windows, zero-history windows (zero == True) and windows with history."""
    return {"all": metrics(pred, target, threshold),
            "zero_history": metrics(pred[zero], target[zero], threshold),
            "with_history": metrics(pred[~zero], target[~zero], threshold)}


def coreml_validation(fp16: np.ndarray, fp32: np.ndarray, target: np.ndarray, zero: np.ndarray,
                      threshold: float) -> Dict:
    """Validation block for the shipped FP16 CoreML model (scores fp16) next to the FP32 reference (fp32)."""
    return {"scored_by": (f"{MLPACKAGE.name}: shipped CoreML FP16 ML Program, coremltools predict, "
                          ".cpuAndNeuralEngine, same held-out windows as validation"),
            **split_metrics(fp16, target, zero, threshold),
            "max_abs_diff_points_vs_fp32": round(float(np.max(np.abs(fp16 - fp32))), 5),
            "mean_abs_diff_points_vs_fp32": round(float(np.mean(np.abs(fp16 - fp32))), 5)}


# ---------------------------------------------------------------------------------------------
# Parity
# ---------------------------------------------------------------------------------------------
def coreml_scores(model_path: Path, windows: np.ndarray, compute_units, batch: int = 1) -> np.ndarray:
    """CoreML predictions for windows [N, 64, 64] (coremltools Python API)."""
    import coremltools as ct
    model = ct.models.MLModel(str(model_path), compute_units=compute_units)
    out = []
    for start in range(0, len(windows), batch):
        chunk = windows[start:start + batch]
        x = np.zeros((batch, FEATURES, 1, WINDOW), dtype=np.float16)
        x[:len(chunk), :, 0, :] = chunk.astype(np.float16)
        res = model.predict({"features": x})
        out.extend(np.asarray(res["risk_score"], dtype=np.float64).reshape(-1)[:len(chunk)].tolist())
    return np.array(out, dtype=np.float64)


def paypal_vectors(schema: Dict) -> np.ndarray:
    """CommerceContext -> features exactly as commerce_features() in crates/aegis-ane-scorer/src/features.rs."""
    norm = Normalizer(schema)
    rows = []
    for c in PAYPAL_CASES:
        raw = {
            "order_amount_cents": c["amount_cents"], "budget_utilization": c["amount_cents"] / PAYPAL_BUDGET_CENTS,
            "new_payee": int(c["new_payee"]), "payee_country_risk": c["payee_country_risk"], "line_items": 1,
            "quantity_total": c["quantity"], "customer_pii_fields": 1,
            "unformatted_sensitive_fields": int(c["unformatted_sensitive_fields"]),
        }
        rows.append(norm.event(raw))
    return np.stack(rows)


def single_event_windows(vectors: np.ndarray) -> np.ndarray:
    w = np.zeros((len(vectors), FEATURES, WINDOW), dtype=np.float32)
    w[:, :, -1] = vectors
    return w


def sparse_events(window: np.ndarray, length: int) -> List[List[List[float]]]:
    """Real events (oldest -> newest) of a [64, 64] window as [[feature index, value], ...] lists."""
    events = []
    for pos in range(WINDOW - length, WINDOW):
        col = window[:, pos]
        events.append([[int(i), round(float(col[i]), 5)] for i in np.nonzero(col)[0]])
    return events


def dense_from_sparse(events: List[List[List[float]]]) -> np.ndarray:
    w = np.zeros((FEATURES, WINDOW), dtype=np.float32)
    for pos, ev in enumerate(events):
        for i, v in ev:
            w[int(i), WINDOW - len(events) + pos] = v
    return w


def choose_parity_windows(data: Dict, rng: np.random.Generator) -> Tuple[np.ndarray, np.ndarray, List[str]]:
    val = data["val"]
    multi = np.where(val["length"] > 1)[0]
    zero = np.where(val["length"] == 1)[0]
    picks_multi = rng.choice(multi, size=min(96, len(multi)), replace=False)
    picks_zero = rng.choice(zero, size=min(56, len(zero)), replace=False)
    scen = [data["scenarios"][s] for s in val["scenario"][picks_multi]]
    return picks_multi, picks_zero, scen


# ---------------------------------------------------------------------------------------------
# Verify-only
# ---------------------------------------------------------------------------------------------
def verify_only() -> int:
    import coremltools as ct
    problems = []
    for path in (MLPACKAGE, MLMODELC, BATCH_MLPACKAGE, BATCH_MLMODELC, PARITY, CARD):
        if not path.exists():
            problems.append(f"missing {path.name}")
    if problems:
        print("verify: " + "; ".join(problems) + " (run harness/build_model.sh)")
        return 1
    ok = True
    parity = json.loads(PARITY.read_text())
    if parity.get("format") != 2:
        print("verify: parity_vectors.json is not format 2 (rebuild with harness/build_model.sh)")
        return 1
    tolerance = float(parity["tolerance_points"])
    vectors = np.array(parity["vectors"], dtype=np.float32)
    ref_vec = np.array(parity["reference_fp32"], dtype=np.float64)
    windows = np.stack([dense_from_sparse(w["events"]) for w in parity["windows"]])
    ref_win = np.array([w["reference_fp32"] for w in parity["windows"]], dtype=np.float64)
    units = ct.ComputeUnit.CPU_AND_NE
    got_vec = coreml_scores(MLPACKAGE, single_event_windows(vectors), units)
    got_win = coreml_scores(MLPACKAGE, windows, units)
    batch = json.loads(CARD.read_text())["batch_model"]["windows_per_prediction"]
    got_batch = coreml_scores(BATCH_MLPACKAGE, np.concatenate([single_event_windows(vectors), windows]), units, batch=batch)
    d_vec, d_win = np.abs(got_vec - ref_vec), np.abs(got_win - ref_win)
    d_batch = np.abs(got_batch - np.concatenate([ref_vec, ref_win]))
    worst = max(d_vec.max(), d_win.max(), d_batch.max())
    print(f"verify: parity (CoreML FP16 .cpuAndNeuralEngine vs PyTorch FP32): {len(vectors)} zero-history events max "
          f"|diff| {d_vec.max():.4f}, {len(windows)} windows max |diff| {d_win.max():.4f} (mean {d_win.mean():.4f}), "
          f"batch model max |diff| {d_batch.max():.4f}; tolerance {tolerance} points")
    ok &= bool(worst <= tolerance)
    for path, expected_shape, layout in ((MLPACKAGE, [1, FEATURES, 1, WINDOW], "window"),
                                         (BATCH_MLPACKAGE, [batch, FEATURES, 1, WINDOW], "window_batch")):
        spec = ct.models.MLModel(str(path), skip_model_load=True).get_spec()
        inp = spec.description.input[0]
        shape = list(inp.type.multiArrayType.shape)
        meta = dict(spec.description.metadata.userDefined)
        fp16 = inp.type.multiArrayType.dataType == ct.proto.FeatureTypes_pb2.ArrayFeatureType.FLOAT16
        good = shape == expected_shape and meta.get(LAYOUT_KEY) == layout and fp16 and inp.name == "features" \
            and spec.description.output[0].name == "risk_score"
        print(f"verify: {path.name}: input {inp.name} {shape} fp16={fp16}, output {spec.description.output[0].name}, "
              f"{LAYOUT_KEY}={meta.get(LAYOUT_KEY)} -> {'ok' if good else 'MISMATCH'}")
        ok &= good
    for compiled in (MLMODELC, BATCH_MLMODELC):
        pc = plan_counts(compiled)
        print(f"verify: {compiled.name} MLComputePlan (.cpuAndNeuralEngine): neural_engine {pc['neural_engine']}, "
              f"cpu {pc['cpu']}, gpu {pc['gpu']} of {pc['dispatched']} dispatched ops")
        ok &= pc["all_on_neural_engine"]
        ct.models.CompiledMLModel(str(compiled), compute_units=units)
    print(f"verify: packages and compiled models load -> {'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


# ---------------------------------------------------------------------------------------------
# Full build
# ---------------------------------------------------------------------------------------------
def main() -> int:
    ap = argparse.ArgumentParser(description="build the AegisWorkflowFP16 CoreML context-window model")
    ap.add_argument("--verify-only", action="store_true", help="check existing artifacts, do not retrain")
    ap.add_argument("--sweep", action="store_true", help="architecture search (random weights) -> arch_sweep.json")
    ap.add_argument("--sweep-only", default="", help="comma-separated candidate names for --sweep")
    ap.add_argument("--smoke", metavar="DIR", help="tiny end-to-end run (300 sessions, 1 epoch) writing into DIR")
    args = ap.parse_args()
    if args.smoke:
        out = Path(args.smoke).resolve()
        out.mkdir(parents=True, exist_ok=True)
        g = globals()
        for name in ("MLPACKAGE", "MLMODELC", "BATCH_MLPACKAGE", "BATCH_MLMODELC", "PARITY", "CARD"):
            g[name] = out / g[name].name
        SIM_CONFIG["sessions"] = 300
        TRAIN_CONFIG["epochs"] = 1
    if args.verify_only:
        return verify_only()
    schema = load_schema()
    if args.sweep:
        return run_sweep(schema, [n for n in args.sweep_only.split(",") if n] or None)

    import coremltools as ct
    import torch

    t_start = time.time()
    thr = float(schema["high_impact_threshold"])
    data = build_dataset(schema)
    tr, va = data["train"], data["val"]
    print(f"dataset (SYNTHETIC): {SIM_CONFIG['sessions']} sessions, {data['events_total']} events; windows train "
          f"{len(tr['X'])} / val {len(va['X'])}; teacher high-impact rate val {np.mean(va['y'] * 100 >= thr):.3f}; "
          f"zero-history windows val {int(np.sum(va['length'] == 1))}", flush=True)

    net, history = train(data, ARCH)
    params = params_from_torch(net)
    val_pred = predict_torch(net, va["X"])
    target = va["y"] * 100.0
    zero = va["length"] == 1
    val_metrics = {
        "scored_by": ("PyTorch FP32 reference: forward pass of the trained weights in torch float32, not the shipped "
                      "FP16 CoreML model (see validation_coreml_fp16 for that)"),
        **split_metrics(val_pred, target, zero, thr),
        "per_scenario_mae_points": {
            name: round(float(np.mean(np.abs(val_pred[va["scenario"] == i] - target[va["scenario"] == i]))), 4)
            for i, name in enumerate(data["scenarios"]) if np.any(va["scenario"] == i)},
        "newest_event_only_teacher_vs_window_teacher": metrics(va["y_event"] * 100.0, target, thr),
        "history": history,
    }
    print(f"trained (PyTorch FP32 reference): val MAE {val_metrics['all']['mae_points']} pts, accuracy@{thr:g} "
          f"{val_metrics['all']['accuracy_at_threshold']}, AUC {val_metrics['all']['auc_vs_teacher_high_impact']}; "
          f"zero-history MAE {val_metrics['zero_history']['mae_points']}", flush=True)

    # The package is written before CoreML can score anything, so the embedded figure is the FP32 one and
    # its key says so; the FP16 validation of the shipped model is in model_card.json validation_coreml_fp16.
    meta = {"seed": str(SIM_CONFIG["seed"]), "architecture": ARCH["name"],
            "val_mae_points_fp32": str(val_metrics["all"]["mae_points"]), "training_data": "synthetic"}
    for batch, package, compiled in ((1, MLPACKAGE, MLMODELC), (BATCH_WINDOWS, BATCH_MLPACKAGE, BATCH_MLMODELC)):
        save_and_compile(build_coreml(params, ARCH, schema, batch, meta), package, compiled)
        print(f"saved {package.name}; compiled -> {compiled.name}", flush=True)

    val_fp16 = coreml_scores(MLPACKAGE, va["X"], ct.ComputeUnit.CPU_AND_NE)
    val_metrics_fp16 = coreml_validation(val_fp16, val_pred, target, zero, thr)
    print(f"validation (CoreML FP16 {MLPACKAGE.name}, .cpuAndNeuralEngine): MAE "
          f"{val_metrics_fp16['all']['mae_points']} pts, accuracy@{thr:g} "
          f"{val_metrics_fp16['all']['accuracy_at_threshold']}; max |FP16 - FP32| "
          f"{val_metrics_fp16['max_abs_diff_points_vs_fp32']} pts", flush=True)

    rng = np.random.default_rng(SIM_CONFIG["seed"] + 1)
    picks_multi, picks_zero, scen = choose_parity_windows(data, rng)
    win_dense = np.stack([dense_from_sparse(sparse_events(va["X"][i].astype(np.float32), int(va["length"][i])))
                          for i in picks_multi])
    vec_dense = np.vstack([va["X"][picks_zero][:, :, -1].astype(np.float32),
                           np.zeros((1, FEATURES), dtype=np.float32), np.ones((1, FEATURES), dtype=np.float32),
                           paypal_vectors(schema)])
    vec_dense = np.round(vec_dense, 5).astype(np.float32)
    teacher = WindowTeacher(schema)
    vec_teacher = squash(event_logits(vec_dense, schema), schema) * 100.0
    win_teacher = va["y"][picks_multi] * 100.0
    units = {"ne": ct.ComputeUnit.CPU_AND_NE, "cpu": ct.ComputeUnit.CPU_ONLY}
    ref_vec = predict_torch(net, single_event_windows(vec_dense))
    ref_win = predict_torch(net, win_dense)
    ne_vec = coreml_scores(MLPACKAGE, single_event_windows(vec_dense), units["ne"])
    ne_win = coreml_scores(MLPACKAGE, win_dense, units["ne"])
    cpu_vec = coreml_scores(MLPACKAGE, single_event_windows(vec_dense), units["cpu"])
    cpu_win = coreml_scores(MLPACKAGE, win_dense, units["cpu"])
    all_windows = np.concatenate([single_event_windows(vec_dense), win_dense])
    batch_scores = coreml_scores(BATCH_MLPACKAGE, all_windows, units["ne"], batch=BATCH_WINDOWS)
    d_vec, d_win = np.abs(ne_vec - ref_vec), np.abs(ne_win - ref_win)
    d_batch = np.abs(batch_scores - np.concatenate([ref_vec, ref_win]))
    d_cpu = np.abs(np.concatenate([cpu_vec, cpu_win]) - np.concatenate([ref_vec, ref_win]))
    worst = float(max(d_vec.max(), d_win.max(), d_batch.max()))
    r5 = lambda xs: [round(float(x), 5) for x in xs]
    parity = {
        "format": 2,
        "description": ("Held-out SYNTHETIC validation data. 'vectors' are single events scored as sessions with no "
                        "history (AneScorer::score); 'windows' are multi-event sessions (AneScorer::score_window), "
                        "events oldest -> newest as sparse [feature index, value] lists. reference_fp32 is the "
                        "PyTorch FP32 forward pass of the trained weights; coreml_* are CoreML FP16 predictions "
                        "(coremltools). The last vectors are all-zeros, all-ones and the four PayPal bench orders."),
        "feature_schema_sha256": hashlib.sha256(SCHEMA_PATH.read_bytes()).hexdigest(),
        "window": WINDOW,
        "tolerance_points": PARITY_TOLERANCE,
        "vectors": [[round(float(v), 5) for v in row] for row in vec_dense],
        "reference_fp32": r5(ref_vec),
        "coreml_fp16_cpu_and_ne": r5(ne_vec),
        "coreml_fp16_cpu_only": r5(cpu_vec),
        "coreml_fp16_batch_cpu_and_ne": r5(batch_scores[:len(vec_dense)]),
        "teacher": r5(vec_teacher),
        "named_vectors": {"all_zeros": len(picks_zero), "all_ones": len(picks_zero) + 1,
                          **{c["case"]: len(picks_zero) + 2 + i for i, c in enumerate(PAYPAL_CASES)}},
        "windows": [
            {"scenario": scen[j], "length": int(va["length"][i]),
             "events": sparse_events(va["X"][i].astype(np.float32), int(va["length"][i])),
             "reference_fp32": round(float(ref_win[j]), 5), "coreml_fp16_cpu_and_ne": round(float(ne_win[j]), 5),
             "coreml_fp16_cpu_only": round(float(cpu_win[j]), 5),
             "coreml_fp16_batch_cpu_and_ne": round(float(batch_scores[len(vec_dense) + j]), 5),
             "teacher": round(float(win_teacher[j]), 5)}
            for j, i in enumerate(picks_multi)],
    }
    PARITY.write_text(json.dumps(parity, separators=(",", ":")) + "\n")
    print(f"parity: max |CoreML FP16 (cpuAndNE) - PyTorch FP32| zero-history {d_vec.max():.4f} pts, windows "
          f"{d_win.max():.4f} pts (mean {d_win.mean():.4f}), batch {d_batch.max():.4f} pts, cpuOnly {d_cpu.max():.4f} pts; "
          f"tolerance {PARITY_TOLERANCE}", flush=True)

    paypal_start = len(picks_zero) + 2
    paypal = []
    for i, c in enumerate(PAYPAL_CASES):
        score = float(ne_vec[paypal_start + i])
        decision = "Allow" if score < thr else "Deny"
        paypal.append({"case": c["case"], "zero_history_score_coreml_fp16": round(score, 4),
                       "reference_fp32": round(float(ref_vec[paypal_start + i]), 4),
                       "teacher": round(float(vec_teacher[paypal_start + i]), 4),
                       "risk_policy_decision": decision, "phase1_decision": c["phase1_decision"],
                       "matches_phase1": decision == c["phase1_decision"]})
        print(f"paypal: {c['case']:38s} score {score:7.3f} -> {decision} (phase 1 {c['phase1_decision']})")

    print("measuring placement + interleaved latency of the built models (shared machine: preliminary)", flush=True)
    online_probe, batch_probe = (summarize_probe(r) for r in probe([MLMODELC, BATCH_MLMODELC]))
    online_stats, batch_stats = arch_stats(ARCH), arch_stats(ARCH, BATCH_WINDOWS)
    online_checks = passes(online_probe, online_stats)
    online_plan, batch_plan = plan_counts(MLMODELC), plan_counts(BATCH_MLMODELC)
    print(f"online: {online_probe}\nbatch: {batch_probe}", flush=True)

    sweep = json.loads(SWEEP.read_text()) if SWEEP.exists() else None
    if sweep and sweep.get("selected") != ARCH["name"]:
        print(f"note: arch_sweep.json selects {sweep.get('selected')}, this build uses {ARCH['name']}")
    card = {
        "model": MLPACKAGE.name,
        "compiled": MLMODELC.name,
        "built_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "training_data": ("SYNTHETIC: sessions from the seeded scenario simulator in models/build_model.py, labelled "
                          "by the explicit window teacher documented there. Not real telemetry or incident data."),
        "input": {"name": "features", "dtype": "float16", "shape": [1, FEATURES, 1, WINDOW],
                  "layout": "window (aegis_input_layout metadata)",
                  "semantics": "channel = feature (models/feature_schema.json order), width = last 64 events of one "
                               "session, oldest -> newest, zero-padded on the left"},
        "output": {"name": "risk_score", "shape": [1, 1, 1, 1], "range": "0..100 (>= 35 high impact)",
                   "semantics": "risk of the newest event given the session history in the window"},
        "architecture": {
            "name": ARCH["name"], "sweep_selected": sweep.get("selected") if sweep else None,
            **arch_config(ARCH), **online_stats,
            "description": arch_description(ARCH),
            "newest_position_receptive_field_events": newest_receptive_field(ARCH),
            "ops_in_program": [op["operator"] for op in online_plan["ops"]],
        },
        "precision": "float16 weights, activations, input and output (ML Program, macOS 14 opset)",
        "weights_fp16_sha256": hashlib.sha256(b"".join(params[k].astype(np.float16).tobytes() for k in sorted(params))).hexdigest(),
        "placement_mlcomputeplan_cpu_and_neural_engine": {k: online_plan[k] for k in ("dispatched", "neural_engine", "cpu", "gpu", "all_on_neural_engine")},
        "latency_at_build": {
            "note": ("aegis_ane_score_windows through the production Swift bridge, one full window per prediction, "
                     ".cpuAndNeuralEngine and .cpuOnly interleaved in alternating rounds on a shared machine; "
                     "preliminary, a quiet benchmark is authoritative"),
            **online_probe,
        },
        "goals": {
            "all_ops_on_neural_engine": online_checks["all_on_neural_engine"],
            "p50_below_limit": online_checks["p50_below_limit"],
            "faster_than_cpu_only": online_checks["faster_than_cpu_only"],
            "limits": {"max_p50_ms": SELECTION["max_p50_ms"]},
        },
        "batch_model": {
            "package": BATCH_MLPACKAGE.name, "compiled": BATCH_MLMODELC.name,
            "input_shape": [BATCH_WINDOWS, FEATURES, 1, WINDOW], "windows_per_prediction": BATCH_WINDOWS,
            "semantics": ("same weights as the online model; scores BATCH independent sessions per prediction "
                          "(one per batch entry, aegis_input_layout=window_batch). AneScorer::score_batch(vectors) "
                          "scores every vector as a session with no history, identical to AneScorer::score; "
                          "AneScorer::score_windows scores whole sessions."),
            "placement_mlcomputeplan_cpu_and_neural_engine": {k: batch_plan[k] for k in ("dispatched", "neural_engine", "cpu", "gpu", "all_on_neural_engine")},
            "latency_at_build": batch_probe,
            "max_abs_diff_points_vs_fp32": round(float(d_batch.max()), 5),
        },
        "teacher": {
            "event": "feature_schema.json weights + interactions, slope/bias (phase-1 teacher) on the newest event",
            "temporal_terms": WINDOW_TEACHER,
            "event_types": EVENT_TYPES,
            "windows_with_nonzero_term": data["windows_with_term"],
            "zero_history_equals_event_teacher": True,
        },
        "simulation": SIM_CONFIG,
        "training": {**TRAIN_CONFIG, "framework": f"torch {torch.__version__} (CPU, float32)",
                     "train_windows": int(len(tr["X"])), "val_windows": int(len(va["X"])),
                     "split": "by session"},
        "validation": val_metrics,
        "validation_coreml_fp16": val_metrics_fp16,
        "parity": {
            "reference": "PyTorch FP32 forward pass of the trained weights",
            "tolerance_points": PARITY_TOLERANCE,
            "zero_history_vectors": int(len(vec_dense)), "windows": int(len(win_dense)),
            "max_abs_diff_points_cpu_and_ne": round(float(max(d_vec.max(), d_win.max())), 5),
            "mean_abs_diff_points_cpu_and_ne": round(float(np.concatenate([d_vec, d_win]).mean()), 5),
            "max_abs_diff_points_cpu_only": round(float(d_cpu.max()), 5),
            "max_abs_diff_points_batch": round(float(d_batch.max()), 5),
        },
        "paypal_bench_cases_zero_history": paypal,
        "architecture_sweep": sweep,
        "toolchain": {"python": sys.version.split()[0], "numpy": np.__version__, "coremltools": ct.__version__,
                      "torch": torch.__version__},
        "limitations": [
            "Training data is synthetic and the labels come from a hand-written teacher; the model has not been "
            "validated on real agent sessions or incidents.",
            "Scores are only as meaningful as the feature extraction feeding them.",
            "Neural Engine placement and the ANE-vs-CPU latency ratio were measured on one Apple M2; MLComputePlan "
            "decides placement per machine and OS.",
        ],
        "build_seconds": round(time.time() - t_start, 2),
    }
    CARD.write_text(json.dumps(card, indent=2) + "\n")
    print(f"model card -> {CARD.name}")
    return 0 if worst <= PARITY_TOLERANCE else 1


if __name__ == "__main__":
    sys.exit(main())
