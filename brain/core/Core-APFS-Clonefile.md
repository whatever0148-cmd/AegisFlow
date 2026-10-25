---
tags:
  - core
status: verified
latency_ms: 0.105042
last_verified: 2026-10-25T17:28:23Z
git_commit: 9f1f777-dirty
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1050 ms · **commit:** `9f1f777-dirty`

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T17:28:26Z · bench 2026-10-25T17:28:23Z · tests 2026-10-25T08:10:13Z · commit `9f1f777-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.0946 ms vs 0.1706 ms p50 (1.8x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1050 ms vs 1.001 ms p50 (9.5x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1204 ms vs 7.459 ms p50 (61.9x) · hits 60 / misses 0 |
| branch · aegisflow_src (111 files) · pool warm hit vs cold clonefile | 0.1087 ms vs 1.264 ms p50 (11.6x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 160.0 µs = 13.333 µs/entry · sys share of wall 0.9992 |
| kernel time per clonefile · small (138 entries) | sys p50 999.0 µs = 7.239 µs/entry · sys share of wall 1.001 |
| kernel time per clonefile · medium (1058 entries) | sys p50 6368.0 µs = 6.019 µs/entry · sys share of wall 0.9984 |
| kernel time per clonefile · aegisflow_src (162 entries) | sys p50 1231.0 µs = 7.599 µs/entry · sys share of wall 0.9992 |
| cold clonefile wall cost per tree entry (fit) | 6.970 µs/entry + 86.620 µs |
| kernel sys time per tree entry (fit) | 5.861 µs/entry + 182.0 µs |
| standby replenish (refill thread) · tiny | p50 0.1812 ms · p95 0.2138 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.023 ms · p95 1.167 ms · n=300 |
| standby replenish (refill thread) · medium | p50 6.817 ms · p95 7.675 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.324 ms · p95 1.554 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6156 ms · p95 0.6955 ms · n=400 |
| background reap per detached tree · small | p50 4.916 ms · p95 5.121 ms · n=300 |
| background reap per detached tree · medium | p50 36.306 ms · p95 37.740 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 7.832 ms · p95 9.034 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0946-0.1204 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.171 ms at 9 files and 7.459 ms at 1025 files, with getrusage ru_stime of 6.02-13.33 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.023 ms small, 6.817 ms medium). An acquire right after a commit finds the standby stale: at medium 0/60 fell back to a cold clone and 60/60 waited for the refill in flight, p50 10.058 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.405 ms, rollback 1.089 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.126 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.444 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.182 ms, cpuOnly 0.223 ms). One call after a 3.0 ms gap takes 0.362 ms with the CPU busy in the gap and 0.730 ms with it idle (cpuOnly 0.221 / 1.210 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.61, and faster with an idle CPU, speedup 1.66). The end-to-end ane_score stage, one call per transaction, has p50 0.369 ms, 2.03x the back-to-back p50 and 1.02x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.78 cores (p95 1.66, max 1.83, 73 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 2.73 cores, 142.15 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
