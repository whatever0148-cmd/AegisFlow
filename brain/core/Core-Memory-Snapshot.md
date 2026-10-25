---
tags:
  - core
status: pending
latency_ms: null
last_verified: null
git_commit: null
crate: aegis_storage
---

# Core-Memory-Snapshot

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> Atomic commit + version memory: `renameatx_np(..., RENAME_SWAP)` exchanges branch and live trees in one syscall; the previous live tree is kept as a CoW version snapshot that `restore()` can bring back. Phase 1 snapshots workspace state on disk only (no process-memory snapshots).

**Implemented in:** `crates/aegis-storage/src/lib.rs`

**Upstream:** [[Agent-Systems-Architect]] (designs versioning) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-Switch-MCP]] (commit / restore) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | no `cargo test --release` run recorded yet |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
