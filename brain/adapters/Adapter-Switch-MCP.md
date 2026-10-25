---
tags:
  - adapter
status: pending
latency_ms: null
last_verified: null
git_commit: null
crate: adapter_switch_mcp
---

# Adapter-Switch-MCP

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> MCP stdio server (branch / score / policy / commit / rollback / approval tools) and the Switch chat bridge that renders workspace-diff + team-approval cards for Mattermost and Slack webhooks.

**Implemented in:** `crates/adapter-switch-mcp/src/mcp.rs` · `crates/adapter-switch-mcp/src/cards.rs`

**Upstream:** [[Core-APFS-Clonefile]] (branch tools) · [[Core-Memory-Snapshot]] (commit / restore) · [[Core-Cedar-Rules]] (approval gate)
**Downstream:** [[Bench-EndToEnd-Rollback]] (MCP round-trip)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | no `cargo test --release` run recorded yet |
| tools/call round-trip (in-process) | — |
| chat webhooks | — |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
