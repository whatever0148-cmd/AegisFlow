---
tags:
  - benchmark-log
---

# Telemetry log

One row per `aegis-bench` run (p50 values; `small` APFS fixture). Source: `artifacts/aegis_phase1_telemetry.json`. `clonefile` is the cold clone; `rollback` is caller-visible (detach only for phase-1 rows, which have no pool, reaper, speedup or gate columns); `gated` counts passed rows of `targets_summary`; `scale` < 1 marks a reduced run; `run` is `preliminary` for a reduced-scale run, a busy machine at the start or external CPU contention during the run (`run_conditions`), `quiet` otherwise, `—` when not recorded.

Back to [[00-AegisFlow-Index]]

<!-- aegis:log:start -->
| bench generated_at | commit | tests pass/fail | clonefile ms | RENAME_SWAP ms | rollback ms | ANE ms | Cedar µs | e2e rollback ms | pool acquire ms | reap ms | ANE speedup | gated | scale | run |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-25T08:06:37Z | `a29112b` | 66/0 | 1.116 | 0.1253 | 0.0894 | 0.0395 | 4.291 | 9.035 |
| 2026-10-25T15:28:03Z | `60fc826-dirty` | 66/0 | 1.006 | 0.1288 | 0.0869 | 0.1785 | 4.542 | 4.178 | 0.1053 | 5.096 | 1.232 | 11/11 | 1.000 | preliminary |
| 2026-10-25T15:38:36Z | `b3f6810` | 66/0 | 1.014 | 0.1296 | 0.0883 | 0.1751 | 4.541 | 3.366 | 0.1074 | 5.053 | 1.255 | 11/11 | 1.000 | preliminary |
| 2026-10-25T17:07:14Z | `9f1f777-dirty` | 66/0 | 0.9906 | 0.1273 | 0.0879 | 0.1743 | 4.209 | 2.574 | 0.1050 | 4.878 | 1.267 | 11/11 | 1.000 | preliminary |
| 2026-10-25T17:28:23Z | `9f1f777-dirty` | 66/0 | 1.001 | 0.1261 | 0.0892 | 0.1820 | 4.208 | 2.791 | 0.1050 | 4.916 | 1.224 | 11/11 | 1.000 | quiet |
| 2026-10-25T18:15:32Z | `3bfc312-dirty` | 66/0 | 1.036 | 0.1242 | 0.0879 | 0.1816 | 4.250 | 3.165 | 0.1069 | 4.929 | 1.226 | 11/11 | 1.000 | quiet |
<!-- aegis:log:end -->
