---
tags:
  - benchmark
status: verified
latency_ms: 3.164375
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 3.164 ms · **commit:** `c3ad550`

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.334 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 3.164 ms · p95 4.235 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 3.799 ms · p95 5.426 ms · n=200 |
| rollback path stage · branch | p50 105.1 µs · p95 121.2 µs · n=200 |
| rollback path stage · agent_edit | p50 140.4 µs · p95 169.4 µs · n=200 |
| rollback path stage · diff | p50 2382.7 µs · p95 3227.7 µs · n=200 |
| rollback path stage · features | p50 5.042 µs · p95 11.416 µs · n=200 |
| rollback path stage · ane_score | p50 370.8 µs · p95 443.5 µs · n=200 |
| rollback path stage · cedar | p50 13.500 µs · p95 35.125 µs · n=200 |
| rollback path stage · rollback | p50 88.709 µs · p95 653.2 µs · n=200 |
| commit path stage · branch | p50 110.9 µs · p95 132.9 µs · n=200 |
| commit path stage · agent_edit | p50 144.9 µs · p95 435.5 µs · n=200 |
| commit path stage · diff | p50 2594.2 µs · p95 3174.1 µs · n=200 |
| commit path stage · features | p50 3.250 µs · p95 9.500 µs · n=200 |
| commit path stage · ane_score | p50 377.2 µs · p95 439.1 µs · n=200 |
| commit path stage · cedar | p50 16.000 µs · p95 40.375 µs · n=200 |
| commit path stage · commit | p50 517.3 µs · p95 1432.4 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.875 |
| commit_case risk p50 · with session history vs without | 1.597 vs 1.340 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 275.0 µs · p95 314.2 µs · n=2000 |
| ed25519 verify | p50 39.667 µs · p95 48.209 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
