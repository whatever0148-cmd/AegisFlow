---
tags:
  - benchmark
status: verified
latency_ms: 0.19375
last_verified: 2026-10-26T06:51:38Z
git_commit: 7cb1a10-dirty
---

# Bench-ANE-FP16

**Status:** 🟢 verified · **p50:** 0.1938 ms · **commit:** `7cb1a10-dirty`

> Online context-window model latency per compute unit (units interleaved in rounds), cpuOnly / cpuAndNeuralEngine speedup, MLComputePlan per-operation placement, parity against the FP32 reference in models/parity_vectors.json, and the batch model.

**Implemented in:** `crates/aegis-bench/src/ane.rs` · `models/parity_vectors.json`

**Upstream:** [[Core-ANE-Classifier]] (measured by) · [[Agent-Telemetry-UI]] (publishes)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| online window model · cpuAndNeuralEngine · back-to-back | p50 0.1938 ms · p95 0.2288 ms · n=2400 |
| online window model · cpuOnly · back-to-back | p50 0.2228 ms · p95 0.3093 ms · n=2400 |
| online window model · all · back-to-back | p50 0.2005 ms · p95 0.2278 ms · n=2400 |
| method | AneScorer::score_window on one full window per call (FFI + FP16 fill + CoreML prediction); back-to-back: per_round consecutive calls per unit, no gap between calls; units interleaved: yes · 40 rounds x 60 per unit |
| speedup cpuOnly p50 / cpuAndNeuralEngine p50 · back-to-back | 1.150 |
| method · isolated calls | one AneScorer::score_window per gap, units cpuAndNeuralEngine and cpuOnly interleaved in rounds; the gap is outside the timed region; gap 3.000 ms; busy: the calling thread spins for the gap (CPU busy, as during diff and feature extraction); idle: the calling thread sleeps for the gap (CPU idle, as while a transaction waits on I/O or the pool); 20 rounds x 20 per unit |
| online window model · cpuAndNeuralEngine · isolated calls (busy CPU gap) | p50 0.3895 ms · p95 0.5647 ms · n=400 |
| online window model · cpuOnly · isolated calls (busy CPU gap) | p50 0.2195 ms · p95 0.4922 ms · n=400 |
| speedup cpuOnly / cpuAndNeuralEngine · isolated calls (busy CPU gap) | 0.5634 |
| online window model · cpuAndNeuralEngine · isolated calls (idle CPU gap) | p50 0.8460 ms · p95 1.417 ms · n=400 |
| online window model · cpuOnly · isolated calls (idle CPU gap) | p50 1.544 ms · p95 2.778 ms · n=400 |
| speedup cpuOnly / cpuAndNeuralEngine · isolated calls (idle CPU gap) | 1.825 |
| one call per transaction (end-to-end ane_score stage) | p50 365.6 µs · p95 411.0 µs · n=200 |
| MLComputePlan preferred device | ANE 36 · CPU 0 · GPU 0 of 36 dispatched ops (135 incl. constants) |
| Neural Engine cores (MLNeuralEngineComputeDevice) | 16 |
| parity · single events (max \|diff\| points) | 0.3012 |
| parity · sessions, cpuAndNeuralEngine | max 0.2746 · mean 0.1024 points |
| parity · sessions, cpuOnly | max 0.1386 · mean 0.0276 points |
| parity · scenario benign_edit | max 0.2088 · mean 0.1273 points (n=13) |
| parity · scenario build_break_loop | max 0.2266 · mean 0.1214 points (n=6) |
| parity · scenario mass_delete | max 0.2705 · mean 0.1202 points (n=8) |
| parity · scenario mixed | max 0.2339 · mean 0.0870 points (n=22) |
| parity · scenario procurement | max 0.2746 · mean 0.0732 points (n=30) |
| parity · scenario refactor | max 0.2440 · mean 0.1301 points (n=10) |
| parity · scenario secret_leak | max 0.2527 · mean 0.1527 points (n=7) |
| batch model (32 windows/prediction) MLComputePlan | ANE 36 · CPU 0 · GPU 0 of 36 ops |
| batch prediction · cpuAndNeuralEngine | p50 1.556 ms · p95 2.794 ms · n=200 · 48.611 µs/window |
| batch prediction · cpuOnly | p50 5.530 ms · p95 9.280 ms · n=200 · 172.8 µs/window |
| batch speedup cpuOnly / cpuAndNeuralEngine | 3.555 |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| CoreML FP16 online window model · cpuAndNeuralEngine · back-to-back calls | < 0.6 ms | 0.1938 ms; p95 0.2288 ms; n=2400 | ✅ PASS |
| online model executes on the Neural Engine (MLComputePlan) | all ops on ANE | ANE 36 · CPU 0 · GPU 0 | ✅ PASS |
| online model, back-to-back calls: cpuAndNeuralEngine faster than cpuOnly | speedup > 1 (cpuOnly p50 / cpuAndNeuralEngine p50) | cpuAndNeuralEngine p50 ms 0.1938 · cpuOnly p50 ms 0.2228 · speedup 1.150; n=2400 | ✅ PASS |
| batch model executes on the Neural Engine (MLComputePlan) | all ops on ANE | ANE 36 · CPU 0 · GPU 0 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| CoreML FP16 online window model · all (compute units) · back-to-back calls | < 0.6 ms (reference) | 0.2005 ms; p95 0.2278 ms; n=2400 | not gated (would pass) |
| CoreML FP16 online window model · cpuAndNeuralEngine · isolated calls (3.0 ms busy CPU gap before each) | < 0.6 ms (reference) | 0.3895 ms; p95 0.5647 ms; n=400 | not gated (would pass) |
| CoreML FP16 online window model · cpuAndNeuralEngine · isolated calls (3.0 ms idle CPU gap before each) | < 0.6 ms (reference) | 0.8460 ms; p95 1.417 ms; n=400 | not gated (would fail) |
| online model, isolated calls (3.0 ms busy CPU gap): cpuAndNeuralEngine faster than cpuOnly | speedup > 1 (reference) | 0.5634; n=400 | not gated (would fail) |
| online model, isolated calls (3.0 ms idle CPU gap): cpuAndNeuralEngine faster than cpuOnly | speedup > 1 (reference) | 1.825; n=400 | not gated (would pass) |
| CoreML online window model · one call per transaction (end-to-end ane_score stage, rollback path) | < 0.6 ms (reference) | 0.3656 ms; p95 0.4110 ms; n=200 | not gated (would pass) |
| CoreML batch window model · per window, amortized (cpuAndNeuralEngine) | — | 48.611 µs | not gated |
| batch model: cpuAndNeuralEngine faster than cpuOnly | speedup > 1 (reference) | 3.555 | not gated (would pass) |
| online model parity · sessions (score_window vs FP32 reference, max \|diff\| points) | <= 1 points (parity file tolerance) | 0.2746 | not gated (would pass) |

**MLComputePlan per-op placement (.cpuAndNeuralEngine, dispatched ops in program order)**

| # | operator | device | supported |
|---|---|---|---|
| 0 | `ios17.mul` | ANE | CPU, ANE |
| 1 | `ios17.conv` | ANE | CPU, ANE |
| 2 | `ios16.relu` | ANE | CPU, ANE |
| 3 | `ios17.conv` | ANE | CPU, ANE |
| 4 | `ios16.relu` | ANE | CPU, ANE |
| 5 | `ios17.conv` | ANE | CPU, ANE |
| 6 | `ios17.add` | ANE | CPU, ANE |
| 7 | `ios16.relu` | ANE | CPU, ANE |
| 8 | `ios17.conv` | ANE | CPU, ANE |
| 9 | `ios16.relu` | ANE | CPU, ANE |
| 10 | `ios17.conv` | ANE | CPU, ANE |
| 11 | `ios17.add` | ANE | CPU, ANE |
| 12 | `ios16.relu` | ANE | CPU, ANE |
| 13 | `ios17.conv` | ANE | CPU, ANE |
| 14 | `ios16.relu` | ANE | CPU, ANE |
| 15 | `ios17.conv` | ANE | CPU, ANE |
| 16 | `ios17.add` | ANE | CPU, ANE |
| 17 | `ios16.relu` | ANE | CPU, ANE |
| 18 | `ios17.conv` | ANE | CPU, ANE |
| 19 | `ios16.relu` | ANE | CPU, ANE |
| 20 | `ios17.conv` | ANE | CPU, ANE |
| 21 | `ios17.add` | ANE | CPU, ANE |
| 22 | `ios16.relu` | ANE | CPU, ANE |
| 23 | `ios17.conv` | ANE | CPU, ANE |
| 24 | `ios16.relu` | ANE | CPU, ANE |
| 25 | `ios17.conv` | ANE | CPU, ANE |
| 26 | `ios17.add` | ANE | CPU, ANE |
| 27 | `ios16.relu` | ANE | CPU, ANE |
| 28 | `ios17.slice_by_index` | ANE | CPU, ANE |
| 29 | `ios16.reduce_mean` | ANE | CPU, ANE |
| 30 | `ios17.concat` | ANE | CPU, ANE |
| 31 | `ios17.conv` | ANE | CPU, ANE |
| 32 | `ios16.relu` | ANE | CPU, ANE |
| 33 | `ios17.conv` | ANE | CPU, ANE |
| 34 | `ios16.sigmoid` | ANE | CPU, ANE |
| 35 | `ios17.mul` | ANE | CPU, ANE |

> MLComputePlan (.cpuAndNeuralEngine) placed the online window model (window layout, 64 events x 64 features) as ANE 36 / CPU 0 / GPU 0 of 36 dispatched ops (135 including constants); the batch model (32 windows per prediction) as ANE 36 / CPU 0 / GPU 0 of 36.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
