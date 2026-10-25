---
tags:
  - core
status: verified
latency_ms: 0.125333
last_verified: 2026-10-25T08:06:37Z
git_commit: a29112b
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** 🟢 verified · **p50:** 0.1253 ms · **commit:** `a29112b`

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. Phase 1 snapshots workspace state on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T08:11:35Z · bench 2026-10-25T08:06:37Z · tests 2026-10-25T08:10:13Z · commit `a29112b`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| renameatx_np swap · tiny | p50 0.1223 ms · p95 0.1497 ms · n=400 |
| renameatx_np swap · small | p50 0.1253 ms · p95 0.1447 ms · n=300 |
| renameatx_np swap · medium | p50 0.1319 ms · p95 0.1687 ms · n=60 |
| renameatx_np swap · aegisflow_src | p50 0.1268 ms · p95 0.1504 ms · n=200 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
