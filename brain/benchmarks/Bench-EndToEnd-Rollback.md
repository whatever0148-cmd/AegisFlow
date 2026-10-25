---
tags:
  - benchmark
status: verified
latency_ms: 9.035375
last_verified: 2026-10-25T08:06:37Z
git_commit: a29112b
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 9.035 ms · **commit:** `a29112b`

> Full transaction: branch -> agent edit -> diff -> features -> ANE -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/main.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T08:11:35Z · bench 2026-10-25T08:06:37Z · tests 2026-10-25T08:10:13Z · commit `a29112b`_

| Metric | Measured |
|---|---|
| rollback path (branch→edit→diff→ANE→Cedar deny→rollback) | p50 9.035 ms · p95 10.716 ms · n=200 |
| commit path (branch→edit→diff→ANE→Cedar allow→swap) | p50 9.274 ms · p95 10.119 ms · n=200 |
| stage p50 · branch | 1202.8 µs |
| stage p50 · agent_edit | 144.7 µs |
| stage p50 · diff | 2401.2 µs |
| stage p50 · features | 4.708 µs |
| stage p50 · ane_score | 40.250 µs |
| stage p50 · cedar | 17.625 µs |
| stage p50 · rollback | 4998.5 µs |
| live tree byte-identical after every rollback | yes |
| PayPal prepare + ed25519 receipt | p50 116.5 µs · p95 122.0 µs · n=2000 |
| ed25519 verify | p50 39.458 µs · p95 45.792 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
