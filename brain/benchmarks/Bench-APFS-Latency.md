---
tags:
  - benchmark
status: target-missed
latency_ms: 1.116458
last_verified: 2026-10-25T08:06:37Z
git_commit: a29112b
---

# Bench-APFS-Latency

**Status:** 🟡 target missed · **p50:** 1.116 ms · **commit:** `a29112b`

> clonefile branch, renameatx_np RENAME_SWAP commit and rollback latency on generated fixtures of increasing size (APFS volume of this Mac).

**Implemented in:** `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T08:11:35Z · bench 2026-10-25T08:06:37Z · tests 2026-10-25T08:10:13Z · commit `a29112b`_

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · clonefile | p50 0.1653 ms · p95 0.2348 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1223 ms · p95 0.1497 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback (detach) | p50 0.0884 ms · p95 0.1118 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback (reclaim) | p50 0.6013 ms · p95 0.7481 ms · n=400 |
| small · 129 files / 1.076 MiB · clonefile | p50 1.116 ms · p95 1.512 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1253 ms · p95 0.1447 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback (detach) | p50 0.0894 ms · p95 0.1014 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback (reclaim) | p50 5.062 ms · p95 5.560 ms · n=300 |
| medium · 1025 files / 8.560 MiB · clonefile | p50 10.319 ms · p95 12.125 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1319 ms · p95 0.1687 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback (detach) | p50 0.1128 ms · p95 0.2234 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback (reclaim) | p50 39.805 ms · p95 51.706 ms · n=60 |
| aegisflow_src · 61 files / 0.5103 MiB · clonefile | p50 0.8968 ms · p95 1.754 ms · n=200 |
| aegisflow_src · 61 files / 0.5103 MiB · RENAME_SWAP | p50 0.1268 ms · p95 0.1504 ms · n=200 |
| aegisflow_src · 61 files / 0.5103 MiB · rollback (detach) | p50 0.0913 ms · p95 0.1090 ms · n=200 |
| aegisflow_src · 61 files / 0.5103 MiB · rollback (reclaim) | p50 5.341 ms · p95 9.331 ms · n=200 |

> Targets are checked on the `small` fixture (129 files): clonefile < 0.5000 ms, swap < 0.5000 ms, rollback < 0.5000 ms.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
