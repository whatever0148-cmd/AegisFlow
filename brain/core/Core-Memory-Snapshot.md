---
tags:
  - core
status: verified
latency_ms: 0.12425
last_verified: 2026-10-25T18:15:32Z
git_commit: 3bfc312-dirty
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1242 ms · **commit:** `3bfc312-dirty`

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T18:15:35Z · bench 2026-10-25T18:15:32Z · tests 2026-10-25T08:10:13Z · commit `3bfc312-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1168 ms · p95 0.1271 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1242 ms · p95 0.1368 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1403 ms · p95 0.1521 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1270 ms · p95 0.1389 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3044 ms · p95 0.3407 ms · n=300 |
| commit caller-visible · tiny | p50 0.4346 ms · p95 0.4789 ms · n=400 |
| commit caller-visible · small | p50 0.4468 ms · p95 0.4961 ms · n=300 |
| commit caller-visible · medium | p50 0.4680 ms · p95 0.5361 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4370 ms · p95 0.5068 ms · n=200 |
| acquire right after commit · tiny | p50 0.4780 ms · p95 0.5463 ms · n=400 · pool 0 / waited for refill 391 / cold 9 |
| acquire right after commit · small | p50 1.428 ms · p95 1.733 ms · n=300 · pool 0 / waited for refill 123 / cold 177 |
| acquire right after commit · medium | p50 10.036 ms · p95 10.656 ms · n=60 · pool 0 / waited for refill 59 / cold 1 |
| acquire right after commit · aegisflow_src | p50 1.798 ms · p95 2.082 ms · n=200 · pool 0 / waited for refill 108 / cold 92 |
| commit right after acquire (refill clone in flight) · small | p50 1.397 ms · p95 1.568 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.085 ms · p95 1.240 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0879 ms · p95 0.0988 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 4.872 ms · p95 5.181 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
