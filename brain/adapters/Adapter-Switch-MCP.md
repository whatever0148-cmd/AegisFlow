---
tags:
  - adapter
status: verified
latency_ms: 0.142834
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
crate: adapter_switch_mcp
---

# Adapter-Switch-MCP

**Status:** 🟢 verified · **p50:** 0.1428 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> MCP stdio server (branch / score / policy / commit / rollback / approval tools) and the Switch chat bridge that renders workspace-diff + team-approval cards for Mattermost and Slack webhooks.

**Implemented in:** `crates/adapter-switch-mcp/src/mcp.rs` · `crates/adapter-switch-mcp/src/cards.rs`

**Upstream:** [[Core-APFS-Clonefile]] (branch tools) · [[Core-Memory-Snapshot]] (commit / restore) · [[Core-Cedar-Rules]] (approval gate)
**Downstream:** [[Bench-EndToEnd-Rollback]] (MCP round-trip)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 8 passed, 0 failed (3 suites) |
| tools/call round-trip (in-process) | p50 142.8 µs · p95 180.2 µs · n=6000 |
| chat webhooks | not called · the benchmark never posts to chat; aegis_request_approval posts only to configured webhooks |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
