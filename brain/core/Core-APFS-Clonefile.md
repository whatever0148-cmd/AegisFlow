---
tags:
  - core
status: verified
latency_ms: 0.105958
last_verified: 2026-10-26T06:51:38Z
git_commit: 7cb1a10-dirty
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1060 ms · **commit:** `7cb1a10-dirty`

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.0992 ms vs 0.1746 ms p50 (1.8x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1060 ms vs 1.003 ms p50 (9.5x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1357 ms vs 7.698 ms p50 (56.7x) · hits 60 / misses 0 |
| branch · aegisflow_src (118 files) · pool warm hit vs cold clonefile | 0.1108 ms vs 1.249 ms p50 (11.3x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 160.0 µs = 13.333 µs/entry · sys share of wall 1.003 |
| kernel time per clonefile · small (138 entries) | sys p50 989.0 µs = 7.167 µs/entry · sys share of wall 1.000 |
| kernel time per clonefile · medium (1058 entries) | sys p50 7120.0 µs = 6.730 µs/entry · sys share of wall 1.006 |
| kernel time per clonefile · aegisflow_src (170 entries) | sys p50 1270.0 µs = 7.471 µs/entry · sys share of wall 0.9989 |
| cold clonefile wall cost per tree entry (fit) | 7.232 µs/entry + 39.910 µs |
| kernel sys time per tree entry (fit) | 6.642 µs/entry + 96.602 µs |
| standby replenish (refill thread) · tiny | p50 0.1796 ms · p95 0.2137 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.027 ms · p95 1.240 ms · n=300 |
| standby replenish (refill thread) · medium | p50 7.182 ms · p95 8.769 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.351 ms · p95 1.766 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6315 ms · p95 0.7010 ms · n=400 |
| background reap per detached tree · small | p50 4.916 ms · p95 5.269 ms · n=300 |
| background reap per detached tree · medium | p50 36.384 ms · p95 43.595 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 8.010 ms · p95 10.616 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0992-0.1357 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.175 ms at 9 files and 7.698 ms at 1025 files, with getrusage ru_stime of 6.73-13.33 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.027 ms small, 7.182 ms medium). An acquire right after a commit finds the standby stale: at medium 3/60 fell back to a cold clone and 57/60 waited for the refill in flight, p50 9.722 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.450 ms, rollback 1.110 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.128 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.462 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.194 ms, cpuOnly 0.223 ms). One call after a 3.0 ms gap takes 0.390 ms with the CPU busy in the gap and 0.846 ms with it idle (cpuOnly 0.219 / 1.544 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.56, and faster with an idle CPU, speedup 1.82). The end-to-end ane_score stage, one call per transaction, has p50 0.366 ms, 1.89x the back-to-back p50 and 0.94x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 1.06 cores (p95 1.85, max 2.40, 75 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 3.29 cores, 168.55 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
