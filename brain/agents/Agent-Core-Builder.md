---
tags:
  - agent
status: pending
latency_ms: null
last_verified: null
git_commit: null
---

# Agent-Core-Builder

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> Implements the Rust + Swift crates under crates/ and the CoreML model builder, keeping `cargo test --release` green.

**Implemented in:** `.claude/agents/core-builder.md` · `crates/` · `models/build_model.py`

**Downstream:** [[Core-APFS-Clonefile]] (builds) · [[Core-Memory-Snapshot]] (builds) · [[Core-ANE-Classifier]] (builds) · [[Core-Cedar-Rules]] (builds)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

| Metric | Measured |
|---|---|
| T02 · Train and export AegisWorkflowFP16.mlpackage -> .mlmodelc | pending |
| T03 · aegis-storage: clonefile branches, RENAME_SWAP commit, Drop rollback | pending |
| T04 · aegis-governance: Cedar 4.x rules, redaction, ed25519 receipts | pending |
| T05 · aegis-ane-scorer: Swift @_cdecl CoreML bridge + feature extraction | pending |
| T06 · adapter-nebius-nemotron: 3-tier build recovery router | pending |
| T07 · adapter-paypal: Sandbox Orders v2 middleware + dashboard rows | pending |
| T08 · adapter-switch-mcp: MCP server + Mattermost/Slack approval cards | pending |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
