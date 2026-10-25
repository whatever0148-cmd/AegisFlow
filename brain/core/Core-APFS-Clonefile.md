---
tags:
  - core
status: verified
latency_ms: 0.106875
last_verified: 2026-10-25T18:15:32Z
git_commit: 3bfc312-dirty
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1069 ms · **commit:** `3bfc312-dirty`

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T18:15:35Z · bench 2026-10-25T18:15:32Z · tests 2026-10-25T08:10:13Z · commit `3bfc312-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.0988 ms vs 0.1746 ms p50 (1.8x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1069 ms vs 1.036 ms p50 (9.7x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1209 ms vs 6.506 ms p50 (53.8x) · hits 60 / misses 0 |
| branch · aegisflow_src (113 files) · pool warm hit vs cold clonefile | 0.1025 ms vs 1.236 ms p50 (12.1x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 170.0 µs = 14.167 µs/entry · sys share of wall 1.000 |
| kernel time per clonefile · small (138 entries) | sys p50 1118.0 µs = 8.101 µs/entry · sys share of wall 0.9953 |
| kernel time per clonefile · medium (1058 entries) | sys p50 6364.0 µs = 6.015 µs/entry · sys share of wall 0.9991 |
| kernel time per clonefile · aegisflow_src (164 entries) | sys p50 1226.0 µs = 7.476 µs/entry · sys share of wall 1.000 |
| cold clonefile wall cost per tree entry (fit) | 5.987 µs/entry + 184.4 µs |
| kernel sys time per tree entry (fit) | 5.822 µs/entry + 222.6 µs |
| standby replenish (refill thread) · tiny | p50 0.1838 ms · p95 0.2155 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.040 ms · p95 1.201 ms · n=300 |
| standby replenish (refill thread) · medium | p50 6.476 ms · p95 6.903 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.222 ms · p95 1.444 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6295 ms · p95 0.6908 ms · n=400 |
| background reap per detached tree · small | p50 4.929 ms · p95 5.220 ms · n=300 |
| background reap per detached tree · medium | p50 35.373 ms · p95 36.852 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 7.535 ms · p95 7.809 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0988-0.1209 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.175 ms at 9 files and 6.506 ms at 1025 files, with getrusage ru_stime of 6.02-14.17 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.040 ms small, 6.476 ms medium). An acquire right after a commit finds the standby stale: at medium 1/60 fell back to a cold clone and 59/60 waited for the refill in flight, p50 10.036 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.397 ms, rollback 1.085 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.124 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.447 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.182 ms, cpuOnly 0.223 ms). One call after a 3.0 ms gap takes 0.361 ms with the CPU busy in the gap and 0.742 ms with it idle (cpuOnly 0.224 / 1.194 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.62, and faster with an idle CPU, speedup 1.61). The end-to-end ane_score stage, one call per transaction, has p50 0.354 ms, 1.95x the back-to-back p50 and 0.98x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.96 cores (p95 1.99, max 2.48, 75 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 2.91 cores, 145.74 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
