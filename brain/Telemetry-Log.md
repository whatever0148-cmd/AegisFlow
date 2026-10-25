---
tags:
  - benchmark-log
---

# Telemetry log

One row per `aegis-bench` run (p50 values; `small` APFS fixture). Source: `artifacts/aegis_phase1_telemetry.json`.

Back to [[00-AegisFlow-Index]]

<!-- aegis:log:start -->
| bench generated_at | commit | tests pass/fail | clonefile ms | RENAME_SWAP ms | rollback ms | ANE ms | Cedar µs | e2e rollback ms |
|---|---|---|---|---|---|---|---|---|
| 2026-10-25T08:06:37Z | `a29112b` | 66/0 | 1.116 | 0.1253 | 0.0894 | 0.0395 | 4.291 | 9.035 |
<!-- aegis:log:end -->
