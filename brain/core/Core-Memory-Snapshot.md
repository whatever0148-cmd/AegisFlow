---
tags:
  - core
status: verified
latency_ms: 0.128084
last_verified: 2026-10-26T06:51:38Z
git_commit: 7cb1a10-dirty
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1281 ms · **commit:** `7cb1a10-dirty`

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1163 ms · p95 0.1258 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1281 ms · p95 0.1478 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1434 ms · p95 0.1708 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1276 ms · p95 0.1432 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3135 ms · p95 0.4206 ms · n=300 |
| commit caller-visible · tiny | p50 0.4355 ms · p95 0.4823 ms · n=400 |
| commit caller-visible · small | p50 0.4618 ms · p95 0.5830 ms · n=300 |
| commit caller-visible · medium | p50 0.4798 ms · p95 0.5577 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4505 ms · p95 0.5243 ms · n=200 |
| acquire right after commit · tiny | p50 0.4566 ms · p95 0.5231 ms · n=400 · pool 0 / waited for refill 368 / cold 32 |
| acquire right after commit · small | p50 1.548 ms · p95 1.873 ms · n=300 · pool 0 / waited for refill 169 / cold 131 |
| acquire right after commit · medium | p50 9.722 ms · p95 11.036 ms · n=60 · pool 0 / waited for refill 57 / cold 3 |
| acquire right after commit · aegisflow_src | p50 1.921 ms · p95 2.433 ms · n=200 · pool 0 / waited for refill 119 / cold 81 |
| commit right after acquire (refill clone in flight) · small | p50 1.450 ms · p95 1.839 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.110 ms · p95 1.384 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0883 ms · p95 0.0983 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 4.858 ms · p95 5.190 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
