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
| 2026-10-26T03:37:03Z | `4403ab4-dirty` | 66/0 | 0.9870 | 0.1247 | 0.0868 | 0.1789 | 4.208 | 3.227 | 0.1054 | 4.880 | 1.233 | 11/11 | 1.000 | quiet |
| 2026-10-26T03:49:57Z | `4403ab4-dirty` | 66/0 | 1.091 | 0.1301 | 0.0947 | 0.1903 | 4.458 | 3.726 | 0.1191 | 5.209 | 1.450 | 11/11 | 1.000 | preliminary |
| 2026-10-26T03:52:53Z | `4403ab4-dirty` | 66/0 | 1.034 | 0.1303 | 0.0932 | 0.1815 | 4.333 | 3.353 | 0.1140 | 5.097 | 1.262 | 11/11 | 1.000 | preliminary |
| 2026-10-26T04:52:47Z | `b5edd4f` | 66/0 | 1.036 | 0.1234 | 0.0880 | 0.1765 | 4.208 | 2.876 | 0.1055 | 4.870 | 1.242 | 11/11 | 1.000 | quiet |
<!-- aegis:log:end -->
