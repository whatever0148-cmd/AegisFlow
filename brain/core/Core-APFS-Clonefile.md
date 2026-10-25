---
tags:
  - core
status: verified
latency_ms: 1.116458
last_verified: 2026-10-25T08:06:37Z
git_commit: a29112b
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 1.116 ms · **commit:** `a29112b`

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree before an agent touches it. Branches that are dropped without a commit are detached and reclaimed automatically (Rust `Drop`).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T08:11:35Z · bench 2026-10-25T08:06:37Z · tests 2026-10-25T08:10:13Z · commit `a29112b`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| clonefile · tiny (9 files) | p50 0.1653 ms · p95 0.2348 ms · n=400 |
| clonefile · small (129 files) | p50 1.116 ms · p95 1.512 ms · n=300 |
| clonefile · medium (1025 files) | p50 10.319 ms · p95 12.125 ms · n=60 |
| clonefile · aegisflow_src (61 files) | p50 0.8968 ms · p95 1.754 ms · n=200 |
| clonefile cost per tree entry (fit) | 9.820 µs |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
