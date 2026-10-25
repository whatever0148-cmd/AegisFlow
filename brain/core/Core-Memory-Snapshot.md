---
tags:
  - core
status: verified
latency_ms: 0.128833
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1288 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1178 ms · p95 0.1338 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1288 ms · p95 0.1514 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1433 ms · p95 0.1693 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1333 ms · p95 0.1604 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3185 ms · p95 0.3792 ms · n=300 |
| commit caller-visible · tiny | p50 0.4407 ms · p95 0.5121 ms · n=400 |
| commit caller-visible · small | p50 0.4668 ms · p95 0.5595 ms · n=300 |
| commit caller-visible · medium | p50 0.4788 ms · p95 0.5690 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4633 ms · p95 0.6627 ms · n=200 |
| acquire right after commit · tiny | p50 0.4860 ms · p95 0.6595 ms · n=400 · pool 0 / waited for refill 356 / cold 44 |
| acquire right after commit · small | p50 1.765 ms · p95 2.529 ms · n=300 · pool 0 / waited for refill 151 / cold 149 |
| acquire right after commit · medium | p50 12.428 ms · p95 14.436 ms · n=60 · pool 0 / waited for refill 59 / cold 1 |
| acquire right after commit · aegisflow_src | p50 2.217 ms · p95 3.275 ms · n=200 · pool 0 / waited for refill 136 / cold 64 |
| commit right after acquire (refill clone in flight) · small | p50 1.605 ms · p95 2.626 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.286 ms · p95 2.522 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0869 ms · p95 0.1005 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 5.066 ms · p95 5.553 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
