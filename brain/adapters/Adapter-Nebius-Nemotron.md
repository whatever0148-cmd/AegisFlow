---
tags:
  - adapter
status: verified
latency_ms: 0.171917
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
crate: adapter_nebius_nemotron
---

# Adapter-Nebius-Nemotron

**Status:** 🟢 verified · **p50:** 0.1719 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> 3-tier build-recovery router. Tier 0: local ANE score + Cedar. Tier 1: nvidia/Llama-3_3-Nemotron-Super-49B-v1 on Nebius. Tier 2: Tavily docs search + nvidia/Llama-3_1-Nemotron-Ultra-253B-v1. Every patch is verified inside a fresh APFS CoW branch.

**Implemented in:** `crates/adapter-nebius-nemotron/src/lib.rs`

**Upstream:** [[Core-APFS-Clonefile]] (verifies patches in CoW branch) · [[Core-ANE-Classifier]] (tier-0 risk) · [[Core-Cedar-Rules]] (commit gate)
**Downstream:** [[Bench-EndToEnd-Rollback]] (tier-0 routing)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 7 passed, 0 failed (2 suites) |
| tier-0 route (ANE + Cedar) | p50 171.9 µs · p95 215.7 µs · n=5000 |
| live Nebius / Tavily calls | not called · NEBIUS_API_KEY not set |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
