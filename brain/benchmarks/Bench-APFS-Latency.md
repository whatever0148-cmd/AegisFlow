---
tags:
  - benchmark
status: verified
latency_ms: 0.106875
last_verified: 2026-10-25T18:15:32Z
git_commit: 3bfc312-dirty
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1069 ms · **commit:** `3bfc312-dirty`

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T18:15:35Z · bench 2026-10-25T18:15:32Z · tests 2026-10-25T08:10:13Z · commit `3bfc312-dirty`_

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.0988 ms · p95 0.1140 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1746 ms · p95 0.1995 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 170.0 µs · p95 242.0 µs · n=400 · 14.167 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.1838 ms · p95 0.2155 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1168 ms · p95 0.1271 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4346 ms · p95 0.4789 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0823 ms · p95 0.0970 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6295 ms · p95 0.6908 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.4780 ms · p95 0.5463 ms · n=400 · cold 9 / waited 391 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1069 ms · p95 0.1213 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 1.036 ms · p95 1.196 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 1118.0 µs · p95 1404.0 µs · n=300 · 8.101 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.040 ms · p95 1.201 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1242 ms · p95 0.1368 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4468 ms · p95 0.4961 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0879 ms · p95 0.0988 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 4.929 ms · p95 5.220 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.428 ms · p95 1.733 ms · n=300 · cold 177 / waited 123 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1209 ms · p95 0.1401 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 6.506 ms · p95 7.643 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 6364.0 µs · p95 7201.0 µs · n=60 · 6.015 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 6.476 ms · p95 6.903 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1403 ms · p95 0.1521 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.4680 ms · p95 0.5361 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.0878 ms · p95 0.0966 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 35.373 ms · p95 36.852 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 10.036 ms · p95 10.656 ms · n=60 · cold 1 / waited 59 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 113 files / 5.469 MiB · acquire (pool warm hit) | p50 0.1025 ms · p95 0.1198 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 113 files / 5.469 MiB · clonefile (cold) | p50 1.236 ms · p95 1.395 ms · n=200 |
| aegisflow_src · 113 files / 5.469 MiB · kernel sys per clonefile | p50 1226.0 µs · p95 1366.0 µs · n=200 · 7.476 µs/entry |
| aegisflow_src · 113 files / 5.469 MiB · standby replenish | p50 1.222 ms · p95 1.444 ms · n=200 |
| aegisflow_src · 113 files / 5.469 MiB · RENAME_SWAP | p50 0.1270 ms · p95 0.1389 ms · n=200 |
| aegisflow_src · 113 files / 5.469 MiB · commit caller-visible | p50 0.4370 ms · p95 0.5068 ms · n=200 |
| aegisflow_src · 113 files / 5.469 MiB · rollback caller-visible (background reap) | p50 0.0846 ms · p95 0.0999 ms · n=200 |
| aegisflow_src · 113 files / 5.469 MiB · background reap | p50 7.535 ms · p95 7.809 ms · n=200 |
| aegisflow_src · 113 files / 5.469 MiB · acquire right after commit | p50 1.798 ms · p95 2.082 ms · n=200 · cold 92 / waited 108 / pool 0 |
| aegisflow_src · 113 files / 5.469 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.0988 ms; hit rate 1.000; p95 0.1140 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1069 ms; hit rate 1.000; p95 0.1213 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1209 ms; hit rate 1.000; p95 0.1401 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (113 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1025 ms; hit rate 1.000; p95 0.1198 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1242 ms; p95 0.1368 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0879 ms; p95 0.0988 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1746 ms; p95 0.1995 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 1.036 ms; p95 1.196 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 6.506 ms; p95 7.643 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (113 files) | < 0.5 ms (same reference as the pool rows) | 1.236 ms; p95 1.395 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 14.167 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 8.101 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 6.015 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (113 files) | — (O(entries) inside the kernel) | 7.476 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.1838 ms; p95 0.2155 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.040 ms; p95 1.201 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 6.476 ms; p95 6.903 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (113 files) | — | 1.222 ms; p95 1.444 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4346 ms; p95 0.4789 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4468 ms; p95 0.4961 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.4680 ms; p95 0.5361 ms; n=60 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (113 files) | < 0.5 ms (reference) | 0.4370 ms; p95 0.5068 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.4780 ms; pool_ready 0 · pool_after_waiting_for_refill 391 · cold 9; p95 0.5463 ms; n=400 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.428 ms; pool_ready 0 · pool_after_waiting_for_refill 123 · cold 177; p95 1.733 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 10.036 ms; pool_ready 0 · pool_after_waiting_for_refill 59 · cold 1; p95 10.656 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (113 files) | < 0.5 ms (reference) | 1.798 ms; pool_ready 0 · pool_after_waiting_for_refill 108 · cold 92; p95 2.082 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6295 ms; p95 0.6908 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 4.929 ms; p95 5.220 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 35.373 ms; p95 36.852 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (113 files) | — | 7.535 ms; p95 7.809 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0849 ms; p95 0.0959 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 4.872 ms; p95 5.181 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.085 ms; p95 1.240 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.397 ms; p95 1.568 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0988-0.1209 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.175 ms at 9 files and 6.506 ms at 1025 files, with getrusage ru_stime of 6.02-14.17 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.040 ms small, 6.476 ms medium). An acquire right after a commit finds the standby stale: at medium 1/60 fell back to a cold clone and 59/60 waited for the refill in flight, p50 10.036 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.397 ms, rollback 1.085 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.124 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.447 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.182 ms, cpuOnly 0.223 ms). One call after a 3.0 ms gap takes 0.361 ms with the CPU busy in the gap and 0.742 ms with it idle (cpuOnly 0.224 / 1.194 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.62, and faster with an idle CPU, speedup 1.61). The end-to-end ane_score stage, one call per transaction, has p50 0.354 ms, 1.95x the back-to-back p50 and 0.98x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.96 cores (p95 1.99, max 2.48, 75 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 2.91 cores, 145.74 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
