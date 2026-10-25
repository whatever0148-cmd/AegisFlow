---
tags:
  - core
status: verified
latency_ms: 0.039458
last_verified: 2026-10-25T08:06:37Z
git_commit: a29112b
crate: aegis_ane_scorer
---

# Core-ANE-Classifier

**Status:** 🟢 verified · **p50:** 0.0395 ms · **commit:** `a29112b`

> CoreML FP16 MLP (64 -> 32 -> 16 -> 1, sigmoid x 100) loaded with `.cpuAndNeuralEngine` through the Swift `@_cdecl("aegis_ane_score_workflow")` bridge. Device placement is read back from `MLComputePlan`. Weights are trained on synthetic data labelled by the documented heuristic in models/build_model.py; this is not a validated production risk model.

**Implemented in:** `crates/aegis-ane-scorer/swift/AegisANEBridge.swift` · `crates/aegis-ane-scorer/src/lib.rs` · `models/build_model.py` · `models/feature_schema.json`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (tier-0 risk) · [[Adapter-PayPal-Commerce]] (order risk) · [[Bench-ANE-FP16]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T08:11:35Z · bench 2026-10-25T08:06:37Z · tests 2026-10-25T08:10:13Z · commit `a29112b`_

| Metric | Measured |
|---|---|
| cargo test --release | 14 passed, 0 failed (2 suites) |
| inference · cpuAndNeuralEngine | p50 0.0395 ms · p95 0.0482 ms · n=3000 |
| parity max abs diff vs NumPy FP32 (points) | 0.1155 |
| device placement (MLComputePlan) | MLComputePlan (.cpuAndNeuralEngine) placed online-model ops as ANE 0 / CPU 7 / GPU 0 of 7; batch model (16384 operations per prediction) as ANE 7 / CPU 0 / GPU 0 of 7. |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
