---
tags:
  - agent
status: verified
latency_ms: 118405.7
last_verified: 2026-10-26T06:46:27Z
git_commit: 7cb1a10-dirty
---

# Agent-Core-Builder

**Status:** 🟢 verified · **p50:** 118405.7 ms · **commit:** `7cb1a10-dirty`

> Implements the Rust + Swift crates under crates/ and the CoreML model builder, keeping `cargo test --release` green.

**Implemented in:** `.claude/agents/core-builder.md` · `crates/` · `models/build_model.py`

**Downstream:** [[Core-APFS-Clonefile]] (builds) · [[Core-Memory-Snapshot]] (builds) · [[Core-ANE-Classifier]] (builds) · [[Core-Cedar-Rules]] (builds)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| T02 · Train and export the context-window model AegisWorkflowFP16{,Batch}.mlpackage -> .mlmodelc | passed · 5030.5 ms |
| T03 · aegis-storage: clonefile branches, RENAME_SWAP commit, Drop rollback | passed · 1069.9 ms |
| T04 · aegis-governance: Cedar 4.x rules, redaction, ed25519 receipts | passed · 44775.2 ms |
| T05 · aegis-ane-scorer: Swift @_cdecl CoreML bridge + feature extraction | passed · 2856.3 ms |
| T06 · adapter-nebius-nemotron: 3-tier build recovery router | passed · 47432.4 ms |
| T07 · adapter-paypal: Sandbox Orders v2 middleware + dashboard rows | passed · 2546.5 ms |
| T08 · adapter-switch-mcp: MCP server + Mattermost/Slack approval cards | passed · 5414.8 ms |
| T14 · aegis-storage phase 2: WarmBranchPool, background reaper, persisted generation | passed · 594.2 ms |
| T15 · CoreML context-window model: artifacts, layout metadata, parity, placement; window scoring API | passed · 5387.4 ms |
| T16 · aegis-config + aegis-daemon: console API, localhost-only security, write-only credentials | passed · 3298.5 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
