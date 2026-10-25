---
tags:
  - benchmark-log
---

# Telemetry log

One row per `aegis-bench` run (p50 values; `small` APFS fixture). Source: `artifacts/aegis_phase1_telemetry.json`. `clonefile` is the cold clone; `rollback` is caller-visible (detach only for phase-1 rows, which have no pool, reaper, speedup or gate columns); `gated` counts passed rows of `targets_summary`; `scale` < 1 marks a reduced run; `run` is `preliminary` for a reduced-scale or busy-machine run (`run_conditions`), `quiet` otherwise, `—` when not recorded.

Back to [[00-AegisFlow-Index]]

<!-- aegis:log:start -->
| bench generated_at | commit | tests pass/fail | clonefile ms | RENAME_SWAP ms | rollback ms | ANE ms | Cedar µs | e2e rollback ms | pool acquire ms | reap ms | ANE speedup | gated | scale | run |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-25T08:06:37Z | `a29112b` | 66/0 | 1.116 | 0.1253 | 0.0894 | 0.0395 | 4.291 | 9.035 |
| 2026-10-25T15:28:03Z | `60fc826-dirty` | 66/0 | 1.006 | 0.1288 | 0.0869 | 0.1785 | 4.542 | 4.178 | 0.1053 | 5.096 | 1.232 | 11/11 | 1.000 | preliminary |
| 2026-10-25T15:38:36Z | `b3f6810` | 66/0 | 1.014 | 0.1296 | 0.0883 | 0.1751 | 4.541 | 3.366 | 0.1074 | 5.053 | 1.255 | 11/11 | 1.000 | preliminary |
<!-- aegis:log:end -->
