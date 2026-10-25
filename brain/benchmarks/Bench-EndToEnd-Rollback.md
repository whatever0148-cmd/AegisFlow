---
tags:
  - benchmark
status: verified
latency_ms: 4.178375
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 4.178 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.105 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 4.178 ms · p95 11.574 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 4.340 ms · p95 6.300 ms · n=200 |
| rollback path stage · branch | p50 126.4 µs · p95 343.2 µs · n=200 |
| rollback path stage · agent_edit | p50 197.6 µs · p95 815.7 µs · n=200 |
| rollback path stage · diff | p50 3007.7 µs · p95 8980.5 µs · n=200 |
| rollback path stage · features | p50 21.500 µs · p95 45.875 µs · n=200 |
| rollback path stage · ane_score | p50 482.7 µs · p95 903.5 µs · n=200 |
| rollback path stage · cedar | p50 51.708 µs · p95 129.4 µs · n=200 |
| rollback path stage · rollback | p50 154.4 µs · p95 1701.8 µs · n=200 |
| commit path stage · branch | p50 120.8 µs · p95 168.1 µs · n=200 |
| commit path stage · agent_edit | p50 205.9 µs · p95 297.8 µs · n=200 |
| commit path stage · diff | p50 2817.3 µs · p95 3605.7 µs · n=200 |
| commit path stage · features | p50 10.417 µs · p95 16.375 µs · n=200 |
| commit path stage · ane_score | p50 446.8 µs · p95 595.3 µs · n=200 |
| commit path stage · cedar | p50 45.708 µs · p95 68.125 µs · n=200 |
| commit path stage · commit | p50 649.0 µs · p95 1759.4 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.938 |
| commit_case risk p50 · with session history vs without | 1.591 vs 1.525 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 279.4 µs · p95 409.3 µs · n=2000 |
| ed25519 verify | p50 42.041 µs · p95 81.833 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
