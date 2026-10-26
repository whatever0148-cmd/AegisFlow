---
tags:
  - agent
status: verified
latency_ms: 104358.1
last_verified: 2026-10-26T06:58:59Z
git_commit: c3ad550
---

# Agent-Core-Builder

**Status:** 🟢 verified · **p50:** 104358.1 ms · **commit:** `c3ad550`

> Implements the Rust + Swift crates under crates/ and the CoreML model builder, keeping `cargo test --release` green.

**Implemented in:** `.claude/agents/core-builder.md` · `crates/` · `models/build_model.py`

**Downstream:** [[Core-APFS-Clonefile]] (builds) · [[Core-Memory-Snapshot]] (builds) · [[Core-ANE-Classifier]] (builds) · [[Core-Cedar-Rules]] (builds)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| T02 · Train and export the context-window model AegisWorkflowFP16{,Batch}.mlpackage -> .mlmodelc | passed · 4583.5 ms |
| T03 · aegis-storage: clonefile branches, RENAME_SWAP commit, Drop rollback | passed · 1007.6 ms |
| T04 · aegis-governance: Cedar 4.x rules, redaction, ed25519 receipts | passed · 37807.0 ms |
| T05 · aegis-ane-scorer: Swift @_cdecl CoreML bridge + feature extraction | passed · 2605.4 ms |
| T06 · adapter-nebius-nemotron: 3-tier build recovery router | passed · 41720.6 ms |
| T07 · adapter-paypal: Sandbox Orders v2 middleware + dashboard rows | passed · 2642.7 ms |
| T08 · adapter-switch-mcp: MCP server + Mattermost/Slack approval cards | passed · 4325.5 ms |
| T14 · aegis-storage phase 2: WarmBranchPool, background reaper, persisted generation | passed · 608.8 ms |
| T15 · CoreML context-window model: artifacts, layout metadata, parity, placement; window scoring API | passed · 5526.3 ms |
| T16 · aegis-config + aegis-daemon: console API, localhost-only security, write-only credentials | passed · 3530.7 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
