---
tags:
  - benchmark
status: verified
latency_ms: 3.164708
last_verified: 2026-10-25T18:15:32Z
git_commit: 3bfc312-dirty
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 3.165 ms · **commit:** `3bfc312-dirty`

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T18:15:35Z · bench 2026-10-25T18:15:32Z · tests 2026-10-25T08:10:13Z · commit `3bfc312-dirty`_

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.175 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 3.165 ms · p95 4.199 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 3.560 ms · p95 4.534 ms · n=200 |
| rollback path stage · branch | p50 107.5 µs · p95 118.8 µs · n=200 |
| rollback path stage · agent_edit | p50 141.8 µs · p95 170.5 µs · n=200 |
| rollback path stage · diff | p50 2372.5 µs · p95 3020.4 µs · n=200 |
| rollback path stage · features | p50 5.125 µs · p95 10.791 µs · n=200 |
| rollback path stage · ane_score | p50 354.2 µs · p95 423.8 µs · n=200 |
| rollback path stage · cedar | p50 13.375 µs · p95 24.542 µs · n=200 |
| rollback path stage · rollback | p50 86.584 µs · p95 533.9 µs · n=200 |
| commit path stage · branch | p50 110.1 µs · p95 123.9 µs · n=200 |
| commit path stage · agent_edit | p50 152.7 µs · p95 203.8 µs · n=200 |
| commit path stage · diff | p50 2380.8 µs · p95 3206.1 µs · n=200 |
| commit path stage · features | p50 2.750 µs · p95 4.584 µs · n=200 |
| commit path stage · ane_score | p50 364.6 µs · p95 412.9 µs · n=200 |
| commit path stage · cedar | p50 14.000 µs · p95 25.791 µs · n=200 |
| commit path stage · commit | p50 477.2 µs · p95 1035.2 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.875 |
| commit_case risk p50 · with session history vs without | 1.597 vs 1.340 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 264.6 µs · p95 287.9 µs · n=2000 |
| ed25519 verify | p50 38.958 µs · p95 44.833 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
