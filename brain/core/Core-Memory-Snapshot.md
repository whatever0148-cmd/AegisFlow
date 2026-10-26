---
tags:
  - core
status: verified
latency_ms: 0.130333
last_verified: 2026-10-26T03:52:53Z
git_commit: 4403ab4-dirty
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1303 ms · **commit:** `4403ab4-dirty` · **bench run: PRELIMINARY**

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1197 ms · p95 0.2925 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1303 ms · p95 0.3031 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1467 ms · p95 0.1968 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1330 ms · p95 0.3041 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3242 ms · p95 0.7050 ms · n=300 |
| commit caller-visible · tiny | p50 0.4522 ms · p95 0.9930 ms · n=400 |
| commit caller-visible · small | p50 0.4717 ms · p95 0.9522 ms · n=300 |
| commit caller-visible · medium | p50 0.5020 ms · p95 0.9726 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4704 ms · p95 0.7825 ms · n=200 |
| acquire right after commit · tiny | p50 0.4288 ms · p95 0.9321 ms · n=400 · pool 0 / waited for refill 298 / cold 102 |
| acquire right after commit · small | p50 1.589 ms · p95 2.240 ms · n=300 · pool 0 / waited for refill 189 / cold 111 |
| acquire right after commit · medium | p50 10.158 ms · p95 11.998 ms · n=60 · pool 0 / waited for refill 52 / cold 8 |
| acquire right after commit · aegisflow_src | p50 1.946 ms · p95 2.546 ms · n=200 · pool 0 / waited for refill 166 / cold 34 |
| commit right after acquire (refill clone in flight) · small | p50 1.463 ms · p95 2.311 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.072 ms · p95 1.268 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0932 ms · p95 0.2145 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 5.116 ms · p95 7.067 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
