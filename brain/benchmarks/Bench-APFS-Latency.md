---
tags:
  - benchmark
status: verified
latency_ms: 0.105959
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1060 ms · **commit:** `c3ad550`

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.1032 ms · p95 0.1300 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1694 ms · p95 0.2179 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 167.0 µs · p95 208.0 µs · n=400 · 13.917 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.2001 ms · p95 0.2875 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1181 ms · p95 0.1365 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4426 ms · p95 0.5405 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0866 ms · p95 0.1071 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6588 ms · p95 0.8083 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.4779 ms · p95 0.5800 ms · n=400 · cold 61 / waited 339 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1060 ms · p95 0.1223 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 0.9979 ms · p95 1.172 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 1038.0 µs · p95 1191.0 µs · n=300 · 7.522 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.064 ms · p95 1.305 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1261 ms · p95 0.1402 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4506 ms · p95 0.5128 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0889 ms · p95 0.1031 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 4.993 ms · p95 5.494 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.692 ms · p95 2.006 ms · n=300 · cold 175 / waited 125 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1190 ms · p95 0.1502 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 6.500 ms · p95 7.661 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 6686.0 µs · p95 7289.0 µs · n=60 · 6.319 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 6.524 ms · p95 7.740 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1385 ms · p95 0.1537 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.4650 ms · p95 0.5293 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.0912 ms · p95 0.1075 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 35.961 ms · p95 37.707 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 9.940 ms · p95 10.737 ms · n=60 · cold 2 / waited 58 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 118 files / 5.518 MiB · acquire (pool warm hit) | p50 0.1064 ms · p95 0.1373 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 118 files / 5.518 MiB · clonefile (cold) | p50 1.376 ms · p95 1.851 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · kernel sys per clonefile | p50 1269.0 µs · p95 1495.0 µs · n=200 · 7.465 µs/entry |
| aegisflow_src · 118 files / 5.518 MiB · standby replenish | p50 1.339 ms · p95 1.667 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · RENAME_SWAP | p50 0.1319 ms · p95 0.1674 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · commit caller-visible | p50 0.4670 ms · p95 0.5877 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · rollback caller-visible (background reap) | p50 0.0887 ms · p95 0.1138 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · background reap | p50 7.949 ms · p95 9.479 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · acquire right after commit | p50 2.050 ms · p95 2.764 ms · n=200 · cold 55 / waited 145 / pool 0 |
| aegisflow_src · 118 files / 5.518 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1032 ms; hit rate 1.000; p95 0.1300 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1060 ms; hit rate 1.000; p95 0.1223 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1190 ms; hit rate 1.000; p95 0.1502 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (118 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1064 ms; hit rate 1.000; p95 0.1373 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1261 ms; p95 0.1402 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0889 ms; p95 0.1031 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1694 ms; p95 0.2179 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 0.9979 ms; p95 1.172 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 6.500 ms; p95 7.661 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (118 files) | < 0.5 ms (same reference as the pool rows) | 1.376 ms; p95 1.851 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 13.917 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 7.522 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 6.319 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (118 files) | — (O(entries) inside the kernel) | 7.465 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.2001 ms; p95 0.2875 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.064 ms; p95 1.305 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 6.524 ms; p95 7.740 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (118 files) | — | 1.339 ms; p95 1.667 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4426 ms; p95 0.5405 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4506 ms; p95 0.5128 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.4650 ms; p95 0.5293 ms; n=60 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (118 files) | < 0.5 ms (reference) | 0.4670 ms; p95 0.5877 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.4779 ms; pool_ready 0 · pool_after_waiting_for_refill 339 · cold 61; p95 0.5800 ms; n=400 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.692 ms; pool_ready 0 · pool_after_waiting_for_refill 125 · cold 175; p95 2.006 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 9.940 ms; pool_ready 0 · pool_after_waiting_for_refill 58 · cold 2; p95 10.737 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (118 files) | < 0.5 ms (reference) | 2.050 ms; pool_ready 0 · pool_after_waiting_for_refill 145 · cold 55; p95 2.764 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6588 ms; p95 0.8083 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 4.993 ms; p95 5.494 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 35.961 ms; p95 37.707 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (118 files) | — | 7.949 ms; p95 9.479 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0860 ms; p95 0.0998 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 4.870 ms; p95 5.210 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.139 ms; p95 1.305 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.443 ms; p95 1.673 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.1032-0.1190 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.169 ms at 9 files and 6.500 ms at 1025 files, with getrusage ru_stime of 6.32-13.92 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.064 ms small, 6.524 ms medium). An acquire right after a commit finds the standby stale: at medium 2/60 fell back to a cold clone and 58/60 waited for the refill in flight, p50 9.940 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.443 ms, rollback 1.139 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.126 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.451 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.194 ms, cpuOnly 0.222 ms). One call after a 3.0 ms gap takes 0.377 ms with the CPU busy in the gap and 0.913 ms with it idle (cpuOnly 0.223 / 1.712 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.59, and faster with an idle CPU, speedup 1.88). The end-to-end ane_score stage, one call per transaction, has p50 0.371 ms, 1.91x the back-to-back p50 and 0.98x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.92 cores (p95 1.48, max 1.85, 76 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 3.03 cores, 160.61 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
