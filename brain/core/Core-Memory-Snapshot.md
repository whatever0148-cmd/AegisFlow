---
tags:
  - core
status: verified
latency_ms: 0.126084
last_verified: 2026-10-25T17:28:23Z
git_commit: 9f1f777-dirty
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1261 ms · **commit:** `9f1f777-dirty`

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T17:28:26Z · bench 2026-10-25T17:28:23Z · tests 2026-10-25T08:10:13Z · commit `9f1f777-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1162 ms · p95 0.1261 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1261 ms · p95 0.1335 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1333 ms · p95 0.1452 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1278 ms · p95 0.1456 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3038 ms · p95 0.3299 ms · n=300 |
| commit caller-visible · tiny | p50 0.4138 ms · p95 0.4757 ms · n=400 |
| commit caller-visible · small | p50 0.4443 ms · p95 0.4809 ms · n=300 |
| commit caller-visible · medium | p50 0.4582 ms · p95 0.5100 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4498 ms · p95 0.5319 ms · n=200 |
| acquire right after commit · tiny | p50 0.4420 ms · p95 0.5282 ms · n=400 · pool 0 / waited for refill 382 / cold 18 |
| acquire right after commit · small | p50 1.418 ms · p95 1.691 ms · n=300 · pool 0 / waited for refill 92 / cold 208 |
| acquire right after commit · medium | p50 10.058 ms · p95 10.887 ms · n=60 · pool 0 / waited for refill 60 / cold 0 |
| acquire right after commit · aegisflow_src | p50 1.789 ms · p95 2.047 ms · n=200 · pool 0 / waited for refill 91 / cold 109 |
| commit right after acquire (refill clone in flight) · small | p50 1.405 ms · p95 1.606 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.089 ms · p95 1.254 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0892 ms · p95 0.0978 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 4.920 ms · p95 5.183 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
