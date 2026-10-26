---
tags:
  - core
status: verified
latency_ms: 0.194416
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
crate: aegis_ane_scorer
---

# Core-ANE-Classifier

**Status:** 🟢 verified · **p50:** 0.1944 ms · **commit:** `c3ad550`

> CoreML FP16 context-window risk model: one prediction scores the newest event of an agent session given the events before it (`ContextWindow` -> `score_window`), loaded with `.cpuAndNeuralEngine` through the Swift `@_cdecl` bridge. Device placement is read back from `MLComputePlan`. Weights are trained on synthetic sessions labelled by the documented teacher in models/build_model.py; this is not a validated production risk model.

**Implemented in:** `crates/aegis-ane-scorer/swift/AegisANEBridge.swift` · `crates/aegis-ane-scorer/src/lib.rs` · `models/build_model.py` · `models/feature_schema.json`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (tier-0 risk) · [[Adapter-PayPal-Commerce]] (order risk) · [[Bench-ANE-FP16]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| cargo test --release | 14 passed, 0 failed (2 suites) |
| architecture (models/model_card.json) | mh128_b5_v8_nd · 13 convs · 128 hidden · 8 horizon views · 468481 parameters · 172097600 MACs/prediction |
| input | window layout · last 64 events x 64 features |
| inference · cpuAndNeuralEngine · back-to-back | p50 0.1944 ms · p95 0.2212 ms · n=2400 |
| inference · cpuOnly · back-to-back | p50 0.2223 ms · p95 0.2935 ms · n=2400 |
| speedup (cpuOnly p50 / cpuAndNeuralEngine p50) · back-to-back | 1.144 |
| inference · isolated calls (busy CPU gap) · cpuAndNeuralEngine vs cpuOnly | p50 0.3769 ms · p95 0.4241 ms · n=400 vs p50 0.2225 ms · p95 0.3011 ms · n=400 · speedup 0.5903 |
| inference · isolated calls (idle CPU gap) · cpuAndNeuralEngine vs cpuOnly | p50 0.9127 ms · p95 1.446 ms · n=400 vs p50 1.712 ms · p95 2.694 ms · n=400 · speedup 1.875 |
| one call per transaction (end-to-end ane_score stage) | p50 370.8 µs · p95 443.5 µs · n=200 |
| MLComputePlan per op (.cpuAndNeuralEngine) | mul x2 -> ANE, conv x13 -> ANE, relu x12 -> ANE, add x5 -> ANE, slice_by_index x1 -> ANE, reduce_mean x1 -> ANE, concat x1 -> ANE, sigmoid x1 -> ANE |
| parity · single events vs FP32 reference (max \|diff\| points) | 0.3012 |
| parity · sessions (score_window) vs FP32 reference | max 0.2746 · mean 0.1024 points · 96/96 within 1.000 |
| device placement (MLComputePlan) | MLComputePlan (.cpuAndNeuralEngine) placed the online window model (window layout, 64 events x 64 features) as ANE 36 / CPU 0 / GPU 0 of 36 dispatched ops (135 including constants); the batch model (32 windows per prediction) as ANE 36 / CPU 0 / GPU 0 of 36. |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
