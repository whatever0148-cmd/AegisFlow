---
tags:
  - benchmark
status: pending
latency_ms: null
last_verified: null
git_commit: null
---

# Bench-EndToEnd-Rollback

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> Full transaction: branch -> agent edit -> diff -> features -> ANE -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/main.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

No measurements recorded yet.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
