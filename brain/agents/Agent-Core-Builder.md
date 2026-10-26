---
tags:
  - agent
status: verified
latency_ms: 17936.6
last_verified: 2026-10-26T03:51:33Z
git_commit: 4403ab4-dirty
---

# Agent-Core-Builder

**Status:** 🟢 verified · **p50:** 17936.6 ms · **commit:** `4403ab4-dirty`

> Implements the Rust + Swift crates under crates/ and the CoreML model builder, keeping `cargo test --release` green.

**Implemented in:** `.claude/agents/core-builder.md` · `crates/` · `models/build_model.py`

**Downstream:** [[Core-APFS-Clonefile]] (builds) · [[Core-Memory-Snapshot]] (builds) · [[Core-ANE-Classifier]] (builds) · [[Core-Cedar-Rules]] (builds)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| T02 · Train and export the context-window model AegisWorkflowFP16{,Batch}.mlpackage -> .mlmodelc | passed · 4693.6 ms |
| T03 · aegis-storage: clonefile branches, RENAME_SWAP commit, Drop rollback | passed · 1022.9 ms |
| T04 · aegis-governance: Cedar 4.x rules, redaction, ed25519 receipts | passed · 422.0 ms |
| T05 · aegis-ane-scorer: Swift @_cdecl CoreML bridge + feature extraction | passed · 668.2 ms |
| T06 · adapter-nebius-nemotron: 3-tier build recovery router | passed · 438.3 ms |
| T07 · adapter-paypal: Sandbox Orders v2 middleware + dashboard rows | passed · 304.4 ms |
| T08 · adapter-switch-mcp: MCP server + Mattermost/Slack approval cards | passed · 1207.3 ms |
| T14 · aegis-storage phase 2: WarmBranchPool, background reaper, persisted generation | passed · 519.8 ms |
| T15 · CoreML context-window model: artifacts, layout metadata, parity, placement; window scoring API | passed · 5182.0 ms |
| T16 · aegis-config + aegis-daemon: console API, localhost-only security, write-only credentials | passed · 3478.1 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
