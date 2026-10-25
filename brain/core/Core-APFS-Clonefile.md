---
tags:
  - core
status: verified
latency_ms: 0.107375
last_verified: 2026-10-25T15:38:36Z
git_commit: b3f6810
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1074 ms · **commit:** `b3f6810` · **bench run: PRELIMINARY**

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:38:39Z · bench 2026-10-25T15:38:36Z · tests 2026-10-25T08:10:13Z · commit `b3f6810`_

> **PRELIMINARY: 1-min load average 13.13 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.1002 ms vs 0.1723 ms p50 (1.7x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1074 ms vs 1.014 ms p50 (9.4x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1465 ms vs 7.729 ms p50 (52.7x) · hits 60 / misses 0 |
| branch · aegisflow_src (108 files) · pool warm hit vs cold clonefile | 0.1052 ms vs 1.338 ms p50 (12.7x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 161.0 µs = 13.417 µs/entry · sys share of wall 1.000 |
| kernel time per clonefile · small (138 entries) | sys p50 1091.0 µs = 7.906 µs/entry · sys share of wall 0.9980 |
| kernel time per clonefile · medium (1058 entries) | sys p50 8856.0 µs = 8.371 µs/entry · sys share of wall 0.9931 |
| kernel time per clonefile · aegisflow_src (159 entries) | sys p50 1458.0 µs = 9.170 µs/entry · sys share of wall 0.9818 |
| cold clonefile wall cost per tree entry (fit) | 7.217 µs/entry + 96.670 µs |
| kernel sys time per tree entry (fit) | 8.327 µs/entry + 45.657 µs |
| standby replenish (refill thread) · tiny | p50 0.1878 ms · p95 0.2103 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.117 ms · p95 1.366 ms · n=300 |
| standby replenish (refill thread) · medium | p50 7.943 ms · p95 9.945 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.468 ms · p95 2.598 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6474 ms · p95 0.6974 ms · n=400 |
| background reap per detached tree · small | p50 5.053 ms · p95 6.016 ms · n=300 |
| background reap per detached tree · medium | p50 38.498 ms · p95 69.912 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 7.893 ms · p95 9.033 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.1002-0.1465 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.172 ms at 9 files and 7.729 ms at 1025 files, with getrusage ru_stime of 7.91-13.42 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.117 ms small, 7.943 ms medium). An acquire right after a commit finds the standby stale: at medium 1/60 fell back to a cold clone and 59/60 waited for the refill in flight, p50 10.820 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.643 ms, rollback 1.289 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.130 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.465 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.175 ms, cpuOnly 0.220 ms). One call after a 3.0 ms gap takes 0.368 ms with the CPU busy in the gap and 0.851 ms with it idle (cpuOnly 0.212 / 1.461 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.58, and faster with an idle CPU, speedup 1.72). The end-to-end ane_score stage, one call per transaction, has p50 0.374 ms, 2.14x the back-to-back p50 and 1.02x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. This run is PRELIMINARY (1-min load average 13.13 at end above the busy threshold 4); only a quiet full-scale run decides the phase result.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
