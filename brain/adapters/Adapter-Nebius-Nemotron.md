---
tags:
  - adapter
status: pending
latency_ms: null
last_verified: null
git_commit: null
crate: adapter_nebius_nemotron
---

# Adapter-Nebius-Nemotron

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> 3-tier build-recovery router. Tier 0: local ANE score + Cedar. Tier 1: nvidia/Llama-3_3-Nemotron-Super-49B-v1 on Nebius. Tier 2: Tavily docs search + nvidia/Llama-3_1-Nemotron-Ultra-253B-v1. Every patch is verified inside a fresh APFS CoW branch.

**Implemented in:** `crates/adapter-nebius-nemotron/src/lib.rs`

**Upstream:** [[Core-APFS-Clonefile]] (verifies patches in CoW branch) · [[Core-ANE-Classifier]] (tier-0 risk) · [[Core-Cedar-Rules]] (commit gate)
**Downstream:** [[Bench-EndToEnd-Rollback]] (tier-0 routing)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | no `cargo test --release` run recorded yet |
| tier-0 route (ANE + Cedar) | — |
| live Nebius / Tavily calls | — |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
