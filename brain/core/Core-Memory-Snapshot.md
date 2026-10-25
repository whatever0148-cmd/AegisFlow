---
tags:
  - core
status: verified
latency_ms: 0.129625
last_verified: 2026-10-25T15:38:36Z
git_commit: b3f6810
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1296 ms · **commit:** `b3f6810` · **bench run: PRELIMINARY**

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:38:39Z · bench 2026-10-25T15:38:36Z · tests 2026-10-25T08:10:13Z · commit `b3f6810`_

> **PRELIMINARY: 1-min load average 13.13 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1192 ms · p95 0.1381 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1296 ms · p95 0.1510 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1423 ms · p95 0.1732 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1272 ms · p95 0.1452 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3175 ms · p95 0.3699 ms · n=300 |
| commit caller-visible · tiny | p50 0.4483 ms · p95 0.5025 ms · n=400 |
| commit caller-visible · small | p50 0.4650 ms · p95 0.5368 ms · n=300 |
| commit caller-visible · medium | p50 0.4987 ms · p95 0.6287 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4596 ms · p95 0.5216 ms · n=200 |
| acquire right after commit · tiny | p50 0.5075 ms · p95 0.6823 ms · n=400 · pool 0 / waited for refill 395 / cold 5 |
| acquire right after commit · small | p50 1.700 ms · p95 2.335 ms · n=300 · pool 0 / waited for refill 155 / cold 145 |
| acquire right after commit · medium | p50 10.820 ms · p95 15.168 ms · n=60 · pool 0 / waited for refill 59 / cold 1 |
| acquire right after commit · aegisflow_src | p50 2.153 ms · p95 2.867 ms · n=200 · pool 0 / waited for refill 107 / cold 93 |
| commit right after acquire (refill clone in flight) · small | p50 1.643 ms · p95 2.466 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.289 ms · p95 2.064 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0883 ms · p95 0.1081 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 5.082 ms · p95 6.599 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
