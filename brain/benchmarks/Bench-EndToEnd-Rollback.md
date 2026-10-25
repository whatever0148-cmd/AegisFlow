---
tags:
  - benchmark
status: verified
latency_ms: 2.790916
last_verified: 2026-10-25T17:28:23Z
git_commit: 9f1f777-dirty
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 2.791 ms · **commit:** `9f1f777-dirty`

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T17:28:26Z · bench 2026-10-25T17:28:23Z · tests 2026-10-25T08:10:13Z · commit `9f1f777-dirty`_

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.188 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 2.791 ms · p95 3.809 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 3.168 ms · p95 4.004 ms · n=200 |
| rollback path stage · branch | p50 108.3 µs · p95 120.3 µs · n=200 |
| rollback path stage · agent_edit | p50 138.9 µs · p95 160.8 µs · n=200 |
| rollback path stage · diff | p50 2046.6 µs · p95 2949.5 µs · n=200 |
| rollback path stage · features | p50 5.500 µs · p95 7.834 µs · n=200 |
| rollback path stage · ane_score | p50 369.1 µs · p95 401.3 µs · n=200 |
| rollback path stage · cedar | p50 13.084 µs · p95 22.709 µs · n=200 |
| rollback path stage · rollback | p50 84.291 µs · p95 119.3 µs · n=200 |
| commit path stage · branch | p50 108.5 µs · p95 118.6 µs · n=200 |
| commit path stage · agent_edit | p50 138.0 µs · p95 167.0 µs · n=200 |
| commit path stage · diff | p50 2039.7 µs · p95 2892.0 µs · n=200 |
| commit path stage · features | p50 2.542 µs · p95 3.792 µs · n=200 |
| commit path stage · ane_score | p50 366.5 µs · p95 420.3 µs · n=200 |
| commit path stage · cedar | p50 12.584 µs · p95 17.125 µs · n=200 |
| commit path stage · commit | p50 449.8 µs · p95 517.7 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.875 |
| commit_case risk p50 · with session history vs without | 1.597 vs 1.340 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 257.9 µs · p95 299.6 µs · n=2000 |
| ed25519 verify | p50 39.167 µs · p95 46.417 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
