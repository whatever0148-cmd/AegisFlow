---
tags:
  - benchmark
status: verified
latency_ms: 3.353208
last_verified: 2026-10-26T03:52:53Z
git_commit: 4403ab4-dirty
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 3.353 ms · **commit:** `4403ab4-dirty` · **bench run: PRELIMINARY**

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.170 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 3.353 ms · p95 5.753 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 3.833 ms · p95 5.922 ms · n=200 |
| rollback path stage · branch | p50 113.8 µs · p95 288.0 µs · n=200 |
| rollback path stage · agent_edit | p50 158.5 µs · p95 413.9 µs · n=200 |
| rollback path stage · diff | p50 2417.6 µs · p95 4104.6 µs · n=200 |
| rollback path stage · features | p50 8.583 µs · p95 19.416 µs · n=200 |
| rollback path stage · ane_score | p50 394.8 µs · p95 474.8 µs · n=200 |
| rollback path stage · cedar | p50 27.375 µs · p95 51.000 µs · n=200 |
| rollback path stage · rollback | p50 106.9 µs · p95 1070.8 µs · n=200 |
| commit path stage · branch | p50 114.5 µs · p95 144.7 µs · n=200 |
| commit path stage · agent_edit | p50 198.7 µs · p95 319.5 µs · n=200 |
| commit path stage · diff | p50 2464.4 µs · p95 3326.5 µs · n=200 |
| commit path stage · features | p50 4.583 µs · p95 12.083 µs · n=200 |
| commit path stage · ane_score | p50 377.3 µs · p95 482.6 µs · n=200 |
| commit path stage · cedar | p50 19.375 µs · p95 56.041 µs · n=200 |
| commit path stage · commit | p50 543.9 µs · p95 1618.1 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.875 |
| commit_case risk p50 · with session history vs without | 1.597 vs 1.340 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 261.2 µs · p95 304.5 µs · n=2000 |
| ed25519 verify | p50 40.250 µs · p95 47.250 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
