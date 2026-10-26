---
tags:
  - adapter
status: verified
latency_ms: 0.195292
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
crate: adapter_nebius_nemotron
---

# Adapter-Nebius-Nemotron

**Status:** 🟢 verified · **p50:** 0.1953 ms · **commit:** `c3ad550`

> 3-tier build-recovery router. Tier 0: local ANE score + Cedar. Tier 1: nvidia/Llama-3_3-Nemotron-Super-49B-v1 on Nebius. Tier 2: Tavily docs search + nvidia/Llama-3_1-Nemotron-Ultra-253B-v1. Every patch is verified inside a fresh APFS CoW branch.

**Implemented in:** `crates/adapter-nebius-nemotron/src/lib.rs`

**Upstream:** [[Core-APFS-Clonefile]] (verifies patches in CoW branch) · [[Core-ANE-Classifier]] (tier-0 risk) · [[Core-Cedar-Rules]] (commit gate)
**Downstream:** [[Bench-EndToEnd-Rollback]] (tier-0 routing)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| cargo test --release | 7 passed, 0 failed (2 suites) |
| tier-0 route (ANE + Cedar) | p50 195.3 µs · p95 235.2 µs · n=5000 |
| live Nebius / Tavily calls | not called · NEBIUS_API_KEY present; set AEGIS_BENCH_LIVE=1 for a live tier-1 probe |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
