---
tags:
  - core
status: verified
latency_ms: 0.123375
last_verified: 2026-10-26T04:52:47Z
git_commit: b5edd4f
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1234 ms · **commit:** `b5edd4f`

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. A persisted generation number makes stale branches and standbys conflict instead of overwriting newer trees. Workspace state is snapshotted on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T04:52:49Z · bench 2026-10-26T04:52:47Z · tests 2026-10-25T08:10:13Z · commit `b5edd4f`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np RENAME_SWAP · tiny | p50 0.1150 ms · p95 0.1267 ms · n=400 |
| renameatx_np RENAME_SWAP · small | p50 0.1234 ms · p95 0.1297 ms · n=300 |
| renameatx_np RENAME_SWAP · medium | p50 0.1335 ms · p95 0.1490 ms · n=60 |
| renameatx_np RENAME_SWAP · aegisflow_src | p50 0.1231 ms · p95 0.1363 ms · n=200 |
| commit total (lock + checks + swap + version) · small | p50 0.3015 ms · p95 0.3239 ms · n=300 |
| commit caller-visible · tiny | p50 0.4164 ms · p95 0.4781 ms · n=400 |
| commit caller-visible · small | p50 0.4376 ms · p95 0.4766 ms · n=300 |
| commit caller-visible · medium | p50 0.4425 ms · p95 0.4843 ms · n=60 |
| commit caller-visible · aegisflow_src | p50 0.4294 ms · p95 0.4940 ms · n=200 |
| acquire right after commit · tiny | p50 0.4409 ms · p95 0.5227 ms · n=400 · pool 0 / waited for refill 389 / cold 11 |
| acquire right after commit · small | p50 1.470 ms · p95 1.726 ms · n=300 · pool 0 / waited for refill 105 / cold 195 |
| acquire right after commit · medium | p50 9.717 ms · p95 10.462 ms · n=60 · pool 0 / waited for refill 46 / cold 14 |
| acquire right after commit · aegisflow_src | p50 1.725 ms · p95 2.058 ms · n=200 · pool 0 / waited for refill 88 / cold 112 |
| commit right after acquire (refill clone in flight) · small | p50 1.410 ms · p95 1.593 ms · n=150 |
| rollback right after acquire (refill clone in flight) · small | p50 1.115 ms · p95 1.270 ms · n=150 |
| rollback caller-visible (background reap) · small | p50 0.0880 ms · p95 0.0963 ms · n=300 |
| rollback synchronous (detach + delete) · small | p50 4.889 ms · p95 5.270 ms · n=300 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
