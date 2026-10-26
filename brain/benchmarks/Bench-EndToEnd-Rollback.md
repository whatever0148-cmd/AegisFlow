---
tags:
  - benchmark
status: verified
latency_ms: 2.875708
last_verified: 2026-10-26T04:52:47Z
git_commit: b5edd4f
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 2.876 ms · **commit:** `b5edd4f`

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T04:52:49Z · bench 2026-10-26T04:52:47Z · tests 2026-10-25T08:10:13Z · commit `b5edd4f`_

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.190 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 2.876 ms · p95 3.819 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 3.486 ms · p95 4.122 ms · n=200 |
| rollback path stage · branch | p50 105.9 µs · p95 118.8 µs · n=200 |
| rollback path stage · agent_edit | p50 134.1 µs · p95 153.3 µs · n=200 |
| rollback path stage · diff | p50 2148.0 µs · p95 3043.1 µs · n=200 |
| rollback path stage · features | p50 4.416 µs · p95 6.958 µs · n=200 |
| rollback path stage · ane_score | p50 355.4 µs · p95 398.5 µs · n=200 |
| rollback path stage · cedar | p50 11.792 µs · p95 18.083 µs · n=200 |
| rollback path stage · rollback | p50 82.291 µs · p95 495.8 µs · n=200 |
| commit path stage · branch | p50 107.7 µs · p95 119.0 µs · n=200 |
| commit path stage · agent_edit | p50 150.3 µs · p95 193.9 µs · n=200 |
| commit path stage · diff | p50 2334.0 µs · p95 2995.0 µs · n=200 |
| commit path stage · features | p50 2.458 µs · p95 5.833 µs · n=200 |
| commit path stage · ane_score | p50 352.2 µs · p95 407.6 µs · n=200 |
| commit path stage · cedar | p50 11.584 µs · p95 17.375 µs · n=200 |
| commit path stage · commit | p50 460.3 µs · p95 917.0 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.875 |
| commit_case risk p50 · with session history vs without | 1.597 vs 1.340 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 267.2 µs · p95 284.2 µs · n=2000 |
| ed25519 verify | p50 39.333 µs · p95 41.833 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
