#!/usr/bin/env python3
"""Unit checks for models/build_model.py helpers that do not train, export or probe anything.

    .venv/bin/python models/tools/test_build_model.py

  - the architecture description in model_card.json is generated from the architecture dict and agrees
    with the dilations of the shipped compiled program (models/AegisWorkflowFP16.mlmodelc/model.mil);
  - arch_sweep.json rows merged from separate --sweep-only runs keep the timestamp of the run that
    measured them instead of sharing one top-level measured_at;
  - the FP16 validation block is labelled with the model that produced it.
"""
from __future__ import annotations

import re
import sys
import unittest
from pathlib import Path

import numpy as np

MODELS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(MODELS))

import build_model as bm  # noqa: E402

CANDIDATES = {c["name"]: c for c in bm.SWEEP_CANDIDATES}


def shipped_block_dilations() -> list:
    """Width dilation of every blocks.N.a conv in the shipped online program, in block order."""
    mil = (bm.MLMODELC / "model.mil").read_text()
    found = re.findall(r'"blocks_(\d+)_a_dilations_0"\), val = tensor<int32, \[2\]>\(\[(\d+), (\d+)\]\)', mil)
    return [int(w) for _, _, w in sorted(found, key=lambda t: int(t[0]))]


def row(name: str, passes: bool = True) -> dict:
    stats = bm.arch_stats(CANDIDATES[name])
    return {"name": name, "config": bm.arch_config(CANDIDATES[name]), **stats, "passes": passes,
            "load_average_1m": None}


def run(at: str, rows: list) -> dict:
    return {"at": at, "candidates": [r["name"] for r in rows], "probe_settings": dict(bm.PROBE_SETTINGS),
            "host": {"cpu": None, "model": None, "macos": None}}


class ArchDescription(unittest.TestCase):
    def test_selected_description_matches_shipped_program(self):
        dilations = shipped_block_dilations()
        self.assertEqual(len(dilations), len(bm.ARCH["blocks"]), "one blocks.N.a conv per residual block")
        self.assertEqual(dilations, [b["dilation"] for b in bm.ARCH["blocks"]])
        text = bm.arch_description(bm.ARCH)
        if max(dilations) > 1:
            self.assertIn(f"dilations {dilations}", text)
        else:
            self.assertIn("undilated", text)
            self.assertNotRegex(text.replace("undilated", ""), "dilated")

    def test_dilated_candidate_names_its_dilations(self):
        arch = CANDIDATES["p128_b5"]
        dilations = [b["dilation"] for b in arch["blocks"]]
        self.assertGreater(max(dilations), 1)
        self.assertIn(f"dilations {dilations}", bm.arch_description(arch))
        self.assertNotIn("undilated", bm.arch_description(arch))

    def test_receptive_field_follows_kernels_and_dilations(self):
        for arch in bm.SWEEP_CANDIDATES:
            expected = 1 + sum(b["dilation"] * (b["k"] - 1) + b["k_b"] - 1 for b in arch["blocks"])
            self.assertEqual(bm.newest_receptive_field(arch), min(bm.WINDOW, expected), arch["name"])
            self.assertIn(f"receptive field {bm.newest_receptive_field(arch)} events", bm.arch_description(arch))
        undilated = bm.ARCH
        self.assertEqual(bm.newest_receptive_field(undilated), 1 + 2 * len(undilated["blocks"]))


class SweepProvenance(unittest.TestCase):
    def test_rows_keep_the_timestamp_of_their_own_run(self):
        legacy = {"measured_at": "2000-01-01T00:00:00Z", "probe_settings": dict(bm.PROBE_SETTINGS),
                  "rows": [row("p128_b5", passes=False), row("p64_b5", passes=False)]}
        first_rows = [row("mh128_b5_v8_nd")]
        first = bm.sweep_document(legacy, first_rows, run("2000-01-02T00:00:00Z", first_rows))
        self.assertNotIn("measured_at", first, "no single top-level timestamp for merged rows")
        self.assertEqual(first["last_run_at"], "2000-01-02T00:00:00Z")
        by_name = {r["name"]: r for r in first["rows"]}
        self.assertEqual(by_name["mh128_b5_v8_nd"]["measured_at"], "2000-01-02T00:00:00Z")
        self.assertEqual(by_name["mh128_b5_v8_nd"]["probe_settings"], bm.PROBE_SETTINGS)
        for name in ("p128_b5", "p64_b5"):
            self.assertIsNone(by_name[name]["measured_at"], "legacy rows do not inherit the new run's time")
            self.assertEqual(by_name[name]["measured_no_later_than"], "2000-01-01T00:00:00Z")
        self.assertEqual(sorted(first["rows_without_own_timestamp"]), ["p128_b5", "p64_b5"])
        self.assertEqual([r["at"] for r in first["runs"]], ["2000-01-02T00:00:00Z"])
        self.assertEqual(first["selected"], "mh128_b5_v8_nd")

        second_rows = [row("p128_b5", passes=False)]
        second = bm.sweep_document(first, second_rows, run("2000-01-03T00:00:00Z", second_rows))
        by_name = {r["name"]: r for r in second["rows"]}
        self.assertEqual(by_name["p128_b5"]["measured_at"], "2000-01-03T00:00:00Z")
        self.assertNotIn("measured_no_later_than", by_name["p128_b5"])
        self.assertEqual(by_name["mh128_b5_v8_nd"]["measured_at"], "2000-01-02T00:00:00Z")
        self.assertEqual(second["rows_without_own_timestamp"], ["p64_b5"])
        self.assertEqual([r["at"] for r in second["runs"]], ["2000-01-02T00:00:00Z", "2000-01-03T00:00:00Z"])
        self.assertEqual([r["name"] for r in second["rows"]],
                         [c["name"] for c in bm.SWEEP_CANDIDATES if c["name"] in by_name], "candidate order")

    def test_runs_without_remaining_rows_are_dropped(self):
        a = [row("mh128_b5_v8_nd")]
        first = bm.sweep_document({"rows": []}, a, run("2000-01-02T00:00:00Z", a))
        again = bm.sweep_document(first, a, run("2000-01-03T00:00:00Z", a))
        self.assertEqual([r["at"] for r in again["runs"]], ["2000-01-03T00:00:00Z"])
        self.assertEqual(again["rows_without_own_timestamp"], [])


class Validation(unittest.TestCase):
    def test_fp16_block_is_labelled_and_compared_with_fp32(self):
        rng = np.random.default_rng(7)
        target = rng.uniform(0, 100, size=40)
        fp32 = target + rng.normal(0, 1, size=40)
        fp16 = fp32 + rng.normal(0, 0.1, size=40)
        zero = np.arange(40) % 4 == 0
        block = bm.coreml_validation(fp16, fp32, target, zero, 35.0)
        self.assertIn(bm.MLPACKAGE.name, block["scored_by"])
        self.assertIn("FP16", block["scored_by"])
        self.assertEqual(block["all"], bm.metrics(fp16, target, 35.0))
        self.assertEqual(block["zero_history"]["windows"], int(zero.sum()))
        self.assertEqual(block["with_history"]["windows"], int((~zero).sum()))
        self.assertAlmostEqual(block["max_abs_diff_points_vs_fp32"], float(np.max(np.abs(fp16 - fp32))), places=4)


if __name__ == "__main__":
    unittest.main(verbosity=2)
