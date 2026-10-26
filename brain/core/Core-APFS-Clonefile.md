---
tags:
  - core
status: verified
latency_ms: 0.1055
last_verified: 2026-10-26T04:52:47Z
git_commit: b5edd4f
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1055 ms · **commit:** `b5edd4f`

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T04:52:49Z · bench 2026-10-26T04:52:47Z · tests 2026-10-25T08:10:13Z · commit `b5edd4f`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.0951 ms vs 0.1630 ms p50 (1.7x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1055 ms vs 1.036 ms p50 (9.8x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1125 ms vs 6.635 ms p50 (59.0x) · hits 60 / misses 0 |
| branch · aegisflow_src (113 files) · pool warm hit vs cold clonefile | 0.0991 ms vs 1.261 ms p50 (12.7x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 152.0 µs = 12.667 µs/entry · sys share of wall 1.003 |
| kernel time per clonefile · small (138 entries) | sys p50 1007.0 µs = 7.297 µs/entry · sys share of wall 1.000 |
| kernel time per clonefile · medium (1058 entries) | sys p50 6299.0 µs = 5.954 µs/entry · sys share of wall 1.002 |
| kernel time per clonefile · aegisflow_src (164 entries) | sys p50 1238.0 µs = 7.549 µs/entry · sys share of wall 1.003 |
| cold clonefile wall cost per tree entry (fit) | 6.119 µs/entry + 174.7 µs |
| kernel sys time per tree entry (fit) | 5.793 µs/entry + 186.9 µs |
| standby replenish (refill thread) · tiny | p50 0.1825 ms · p95 0.2182 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.026 ms · p95 1.143 ms · n=300 |
| standby replenish (refill thread) · medium | p50 6.087 ms · p95 6.714 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.179 ms · p95 1.441 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6093 ms · p95 0.6953 ms · n=400 |
| background reap per detached tree · small | p50 4.870 ms · p95 5.146 ms · n=300 |
| background reap per detached tree · medium | p50 35.080 ms · p95 35.931 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 7.549 ms · p95 7.814 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0951-0.1125 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.163 ms at 9 files and 6.635 ms at 1025 files, with getrusage ru_stime of 5.95-12.67 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.026 ms small, 6.087 ms medium). An acquire right after a commit finds the standby stale: at medium 14/60 fell back to a cold clone and 46/60 waited for the refill in flight, p50 9.717 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.410 ms, rollback 1.115 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.123 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.438 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.177 ms, cpuOnly 0.219 ms). One call after a 3.0 ms gap takes 0.359 ms with the CPU busy in the gap and 0.730 ms with it idle (cpuOnly 0.225 / 1.512 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.63, and faster with an idle CPU, speedup 2.07). The end-to-end ane_score stage, one call per transaction, has p50 0.355 ms, 2.01x the back-to-back p50 and 0.99x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.67 cores (p95 1.26, max 1.57, 72 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 2.38 cores, 123.65 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
