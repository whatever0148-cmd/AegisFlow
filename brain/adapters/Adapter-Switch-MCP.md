---
tags:
  - adapter
status: verified
latency_ms: 0.144667
last_verified: 2026-10-25T15:38:36Z
git_commit: b3f6810
crate: adapter_switch_mcp
---

# Adapter-Switch-MCP

**Status:** 🟢 verified · **p50:** 0.1447 ms · **commit:** `b3f6810` · **bench run: PRELIMINARY**

> MCP stdio server (branch / score / policy / commit / rollback / approval tools) and the Switch chat bridge that renders workspace-diff + team-approval cards for Mattermost and Slack webhooks.

**Implemented in:** `crates/adapter-switch-mcp/src/mcp.rs` · `crates/adapter-switch-mcp/src/cards.rs`

**Upstream:** [[Core-APFS-Clonefile]] (branch tools) · [[Core-Memory-Snapshot]] (commit / restore) · [[Core-Cedar-Rules]] (approval gate)
**Downstream:** [[Bench-EndToEnd-Rollback]] (MCP round-trip)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:38:39Z · bench 2026-10-25T15:38:36Z · tests 2026-10-25T08:10:13Z · commit `b3f6810`_

> **PRELIMINARY: 1-min load average 13.13 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 8 passed, 0 failed (3 suites) |
| tools/call round-trip (in-process) | p50 144.7 µs · p95 204.4 µs · n=6000 |
| chat webhooks | not called · the benchmark never posts to chat; aegis_request_approval posts only to configured webhooks |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
