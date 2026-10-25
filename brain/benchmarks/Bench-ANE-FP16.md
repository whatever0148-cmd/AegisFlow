---
tags:
  - benchmark
status: target-missed
latency_ms: 0.039458
last_verified: 2026-10-25T08:06:37Z
git_commit: a29112b
---

# Bench-ANE-FP16

**Status:** 🟡 target missed · **p50:** 0.0395 ms · **commit:** `a29112b`

> CoreML FP16 inference latency per compute-unit setting, MLComputePlan per-operation device placement, and parity against a NumPy FP32 reference.

**Implemented in:** `crates/aegis-bench/src/main.rs` · `models/parity_vectors.json`

**Upstream:** [[Core-ANE-Classifier]] (measured by) · [[Agent-Telemetry-UI]] (publishes)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T08:11:35Z · bench 2026-10-25T08:06:37Z · tests 2026-10-25T08:10:13Z · commit `a29112b`_

| Metric | Measured |
|---|---|
| inference · cpuAndNeuralEngine | p50 0.0395 ms · p95 0.0482 ms · n=3000 |
| inference · cpuOnly | p50 0.0389 ms · p95 0.0482 ms · n=3000 |
| inference · all | p50 0.0393 ms · p95 0.0483 ms · n=3000 |
| MLComputePlan preferred device | ANE 0 · CPU 7 · GPU 0 of 7 ops |
| Neural Engine cores (MLNeuralEngineComputeDevice) | 16 |
| parity max abs diff vs NumPy FP32 (points) | 0.1155 |
| batch model (16384 ops/prediction) MLComputePlan | ANE 7 · CPU 0 · GPU 0 of 7 ops |
| batch prediction · cpuAndNeuralEngine | p50 4.871 ms · p95 4.924 ms · n=40 · 0.2973 µs/op |
| batch prediction · cpuOnly | p50 4.515 ms · p95 4.572 ms · n=40 · 0.2756 µs/op |

> MLComputePlan (.cpuAndNeuralEngine) placed online-model ops as ANE 0 / CPU 7 / GPU 0 of 7; batch model (16384 operations per prediction) as ANE 7 / CPU 0 / GPU 0 of 7.

> The online (one operation per prediction) model is not on the Neural Engine, so the 'inference on the ANE' target is marked missed even though its latency target is met.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
