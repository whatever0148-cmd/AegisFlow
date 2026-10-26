---
tags:
  - core
status: verified
latency_ms: 0.126083
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1261 ms · **commit:** `c3ad550`

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1181 ms · p95 0.1365 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1261 ms · p95 0.1402 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1385 ms · p95 0.1537 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1319 ms · p95 0.1674 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3074 ms · p95 0.3549 ms · n=300 |
| commit caller-visible · tiny | p50 0.4426 ms · p95 0.5405 ms · n=400 |
| commit caller-visible · small | p50 0.4506 ms · p95 0.5128 ms · n=300 |
| commit caller-visible · medium | p50 0.4650 ms · p95 0.5293 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4670 ms · p95 0.5877 ms · n=200 |
| acquire right after commit · tiny | p50 0.4779 ms · p95 0.5800 ms · n=400 · pool 0 / waited for refill 339 / cold 61 |
| acquire right after commit · small | p50 1.692 ms · p95 2.006 ms · n=300 · pool 0 / waited for refill 125 / cold 175 |
| acquire right after commit · medium | p50 9.940 ms · p95 10.737 ms · n=60 · pool 0 / waited for refill 58 / cold 2 |
| acquire right after commit · aegisflow_src | p50 2.050 ms · p95 2.764 ms · n=200 · pool 0 / waited for refill 145 / cold 55 |
| commit right after acquire (refill clone in flight) · small | p50 1.443 ms · p95 1.673 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.139 ms · p95 1.305 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0889 ms · p95 0.1031 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 4.870 ms · p95 5.210 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
