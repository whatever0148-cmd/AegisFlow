---
tags:
  - core
status: verified
latency_ms: 0.105959
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1060 ms · **commit:** `c3ad550`

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.1032 ms vs 0.1694 ms p50 (1.6x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1060 ms vs 0.9979 ms p50 (9.4x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1190 ms vs 6.500 ms p50 (54.6x) · hits 60 / misses 0 |
| branch · aegisflow_src (118 files) · pool warm hit vs cold clonefile | 0.1064 ms vs 1.376 ms p50 (12.9x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 167.0 µs = 13.917 µs/entry · sys share of wall 1.002 |
| kernel time per clonefile · small (138 entries) | sys p50 1038.0 µs = 7.522 µs/entry · sys share of wall 0.9992 |
| kernel time per clonefile · medium (1058 entries) | sys p50 6686.0 µs = 6.319 µs/entry · sys share of wall 0.9990 |
| kernel time per clonefile · aegisflow_src (170 entries) | sys p50 1269.0 µs = 7.465 µs/entry · sys share of wall 0.9988 |
| cold clonefile wall cost per tree entry (fit) | 5.969 µs/entry + 204.2 µs |
| kernel sys time per tree entry (fit) | 6.177 µs/entry + 162.0 µs |
| standby replenish (refill thread) · tiny | p50 0.2001 ms · p95 0.2875 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.064 ms · p95 1.305 ms · n=300 |
| standby replenish (refill thread) · medium | p50 6.524 ms · p95 7.740 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.339 ms · p95 1.667 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6588 ms · p95 0.8083 ms · n=400 |
| background reap per detached tree · small | p50 4.993 ms · p95 5.494 ms · n=300 |
| background reap per detached tree · medium | p50 35.961 ms · p95 37.707 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 7.949 ms · p95 9.479 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.1032-0.1190 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.169 ms at 9 files and 6.500 ms at 1025 files, with getrusage ru_stime of 6.32-13.92 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.064 ms small, 6.524 ms medium). An acquire right after a commit finds the standby stale: at medium 2/60 fell back to a cold clone and 58/60 waited for the refill in flight, p50 9.940 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.443 ms, rollback 1.139 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.126 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.451 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.194 ms, cpuOnly 0.222 ms). One call after a 3.0 ms gap takes 0.377 ms with the CPU busy in the gap and 0.913 ms with it idle (cpuOnly 0.223 / 1.712 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.59, and faster with an idle CPU, speedup 1.88). The end-to-end ane_score stage, one call per transaction, has p50 0.371 ms, 1.91x the back-to-back p50 and 0.98x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.92 cores (p95 1.48, max 1.85, 76 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 3.03 cores, 160.61 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
