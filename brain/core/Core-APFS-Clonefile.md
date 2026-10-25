---
tags:
  - core
status: pending
latency_ms: null
last_verified: null
git_commit: null
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree before an agent touches it. Branches that are dropped without a commit are detached and reclaimed automatically (Rust `Drop`).

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

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
