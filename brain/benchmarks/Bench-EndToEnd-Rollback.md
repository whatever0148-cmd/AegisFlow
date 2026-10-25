---
tags:
  - benchmark
status: verified
latency_ms: 3.365833
last_verified: 2026-10-25T15:38:36Z
git_commit: b3f6810
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 3.366 ms · **commit:** `b3f6810` · **bench run: PRELIMINARY**

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:38:39Z · bench 2026-10-25T15:38:36Z · tests 2026-10-25T08:10:13Z · commit `b3f6810`_

> **PRELIMINARY: 1-min load average 13.13 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.155 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 3.366 ms · p95 5.681 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 3.809 ms · p95 5.778 ms · n=200 |
| rollback path stage · branch | p50 107.9 µs · p95 141.9 µs · n=200 |
| rollback path stage · agent_edit | p50 154.0 µs · p95 280.7 µs · n=200 |
| rollback path stage · diff | p50 2523.9 µs · p95 4114.8 µs · n=200 |
| rollback path stage · features | p50 7.667 µs · p95 22.333 µs · n=200 |
| rollback path stage · ane_score | p50 373.9 µs · p95 486.5 µs · n=200 |
| rollback path stage · cedar | p50 20.375 µs · p95 60.708 µs · n=200 |
| rollback path stage · rollback | p50 97.125 µs · p95 1022.8 µs · n=200 |
| commit path stage · branch | p50 109.7 µs · p95 128.8 µs · n=200 |
| commit path stage · agent_edit | p50 167.2 µs · p95 292.0 µs · n=200 |
| commit path stage · diff | p50 2573.7 µs · p95 3368.8 µs · n=200 |
| commit path stage · features | p50 3.625 µs · p95 11.250 µs · n=200 |
| commit path stage · ane_score | p50 372.4 µs · p95 451.0 µs · n=200 |
| commit path stage · cedar | p50 20.500 µs · p95 53.625 µs · n=200 |
| commit path stage · commit | p50 517.8 µs · p95 1436.1 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.938 |
| commit_case risk p50 · with session history vs without | 1.591 vs 1.525 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 244.8 µs · p95 323.8 µs · n=2000 |
| ed25519 verify | p50 39.625 µs · p95 49.042 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
