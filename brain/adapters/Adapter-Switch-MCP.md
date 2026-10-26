---
tags:
  - adapter
status: verified
latency_ms: 0.11225
last_verified: 2026-10-26T03:52:53Z
git_commit: 4403ab4-dirty
crate: adapter_switch_mcp
---

# Adapter-Switch-MCP

**Status:** 🟢 verified · **p50:** 0.1123 ms · **commit:** `4403ab4-dirty` · **bench run: PRELIMINARY**

> MCP stdio server (branch / score / policy / commit / rollback / approval tools) and the Switch chat bridge that renders workspace-diff + team-approval cards for Mattermost and Slack webhooks.

**Implemented in:** `crates/adapter-switch-mcp/src/mcp.rs` · `crates/adapter-switch-mcp/src/cards.rs`

**Upstream:** [[Core-APFS-Clonefile]] (branch tools) · [[Core-Memory-Snapshot]] (commit / restore) · [[Core-Cedar-Rules]] (approval gate)
**Downstream:** [[Bench-EndToEnd-Rollback]] (MCP round-trip)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 8 passed, 0 failed (3 suites) |
| tools/call round-trip (in-process) | p50 112.2 µs · p95 192.0 µs · n=6000 |
| chat webhooks | not called · the benchmark never posts to chat; aegis_request_approval posts only to configured webhooks |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
