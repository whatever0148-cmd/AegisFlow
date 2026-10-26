---
tags:
  - benchmark
status: verified
latency_ms: 3.253292
last_verified: 2026-10-26T06:51:38Z
git_commit: 7cb1a10-dirty
---

# Bench-EndToEnd-Rollback

**Status:** 🟢 verified · **p50:** 3.253 ms · **commit:** `7cb1a10-dirty`

> Full transaction: pool acquire -> agent edit -> diff -> features -> context-window score -> Cedar -> rollback or commit, with live-tree integrity hashing; plus PayPal order preparation and receipt signing.

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Adapter-Nebius-Nemotron]] (tier-0 routing) · [[Adapter-PayPal-Commerce]] (receipt signing) · [[Adapter-Switch-MCP]] (MCP round-trip) · [[Agent-QA-Reviewer]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| branching | WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions (replenish delay 2.022 ms) |
| scoring | AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent) |
| rollback path (acquire→edit→diff→score→Cedar deny→rollback) | p50 3.253 ms · p95 4.186 ms · n=200 |
| commit path (acquire→edit→diff→score→Cedar allow→swap) | p50 3.697 ms · p95 4.922 ms · n=200 |
| rollback path stage · branch | p50 108.2 µs · p95 121.6 µs · n=200 |
| rollback path stage · agent_edit | p50 136.7 µs · p95 168.1 µs · n=200 |
| rollback path stage · diff | p50 2540.9 µs · p95 3132.2 µs · n=200 |
| rollback path stage · features | p50 4.625 µs · p95 10.250 µs · n=200 |
| rollback path stage · ane_score | p50 365.6 µs · p95 411.0 µs · n=200 |
| rollback path stage · cedar | p50 12.542 µs · p95 28.416 µs · n=200 |
| rollback path stage · rollback | p50 91.167 µs · p95 824.8 µs · n=200 |
| commit path stage · branch | p50 108.5 µs · p95 122.0 µs · n=200 |
| commit path stage · agent_edit | p50 135.7 µs · p95 272.0 µs · n=200 |
| commit path stage · diff | p50 2476.9 µs · p95 3085.8 µs · n=200 |
| commit path stage · features | p50 2.500 µs · p95 4.792 µs · n=200 |
| commit path stage · ane_score | p50 366.6 µs · p95 421.1 µs · n=200 |
| commit path stage · cedar | p50 12.916 µs · p95 22.500 µs · n=200 |
| commit path stage · commit | p50 470.2 µs · p95 1378.5 µs · n=200 |
| acquire sources · rollback_path | pool 200 / waited 0 / cold 0 |
| acquire sources · commit_path | pool 200 / waited 0 / cold 0 |
| rollback_case risk p50 · with session history vs without | 100.0 vs 99.875 |
| commit_case risk p50 · with session history vs without | 1.597 vs 1.340 |
| live tree byte-identical after every rollback | yes |
| live tree equals the last allowed commit | yes |
| PayPal prepare + ed25519 receipt | p50 273.5 µs · p95 297.0 µs · n=2000 |
| ed25519 verify | p50 39.500 µs · p95 44.667 µs · n=2000 |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
