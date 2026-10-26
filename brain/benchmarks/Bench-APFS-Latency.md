---
tags:
  - benchmark
status: verified
latency_ms: 0.105958
last_verified: 2026-10-26T06:51:38Z
git_commit: 7cb1a10-dirty
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1060 ms · **commit:** `7cb1a10-dirty`

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.0992 ms · p95 0.1167 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1746 ms · p95 0.2622 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 160.0 µs · p95 249.0 µs · n=400 · 13.333 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.1796 ms · p95 0.2137 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1163 ms · p95 0.1258 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4355 ms · p95 0.4823 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0823 ms · p95 0.0992 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6315 ms · p95 0.7010 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.4566 ms · p95 0.5231 ms · n=400 · cold 32 / waited 368 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1060 ms · p95 0.1202 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 1.003 ms · p95 1.222 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 989.0 µs · p95 1141.0 µs · n=300 · 7.167 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.027 ms · p95 1.240 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1281 ms · p95 0.1478 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4618 ms · p95 0.5830 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0883 ms · p95 0.0983 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 4.916 ms · p95 5.269 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.548 ms · p95 1.873 ms · n=300 · cold 131 / waited 169 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1357 ms · p95 0.1593 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 7.698 ms · p95 8.951 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 7120.0 µs · p95 7894.0 µs · n=60 · 6.730 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 7.182 ms · p95 8.769 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1434 ms · p95 0.1708 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.4798 ms · p95 0.5577 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.0988 ms · p95 0.1363 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 36.384 ms · p95 43.595 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 9.722 ms · p95 11.036 ms · n=60 · cold 3 / waited 57 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 118 files / 5.518 MiB · acquire (pool warm hit) | p50 0.1108 ms · p95 0.1707 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 118 files / 5.518 MiB · clonefile (cold) | p50 1.249 ms · p95 1.472 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · kernel sys per clonefile | p50 1270.0 µs · p95 1536.0 µs · n=200 · 7.471 µs/entry |
| aegisflow_src · 118 files / 5.518 MiB · standby replenish | p50 1.351 ms · p95 1.766 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · RENAME_SWAP | p50 0.1276 ms · p95 0.1432 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · commit caller-visible | p50 0.4505 ms · p95 0.5243 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · rollback caller-visible (background reap) | p50 0.0909 ms · p95 0.1131 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · background reap | p50 8.010 ms · p95 10.616 ms · n=200 |
| aegisflow_src · 118 files / 5.518 MiB · acquire right after commit | p50 1.921 ms · p95 2.433 ms · n=200 · cold 81 / waited 119 / pool 0 |
| aegisflow_src · 118 files / 5.518 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.0992 ms; hit rate 1.000; p95 0.1167 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1060 ms; hit rate 1.000; p95 0.1202 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1357 ms; hit rate 1.000; p95 0.1593 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (118 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1108 ms; hit rate 1.000; p95 0.1707 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1281 ms; p95 0.1478 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0883 ms; p95 0.0983 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1746 ms; p95 0.2622 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 1.003 ms; p95 1.222 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 7.698 ms; p95 8.951 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (118 files) | < 0.5 ms (same reference as the pool rows) | 1.249 ms; p95 1.472 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 13.333 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 7.167 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 6.730 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (118 files) | — (O(entries) inside the kernel) | 7.471 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.1796 ms; p95 0.2137 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.027 ms; p95 1.240 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 7.182 ms; p95 8.769 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (118 files) | — | 1.351 ms; p95 1.766 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4355 ms; p95 0.4823 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4618 ms; p95 0.5830 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.4798 ms; p95 0.5577 ms; n=60 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (118 files) | < 0.5 ms (reference) | 0.4505 ms; p95 0.5243 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.4566 ms; pool_ready 0 · pool_after_waiting_for_refill 368 · cold 32; p95 0.5231 ms; n=400 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.548 ms; pool_ready 0 · pool_after_waiting_for_refill 169 · cold 131; p95 1.873 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 9.722 ms; pool_ready 0 · pool_after_waiting_for_refill 57 · cold 3; p95 11.036 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (118 files) | < 0.5 ms (reference) | 1.921 ms; pool_ready 0 · pool_after_waiting_for_refill 119 · cold 81; p95 2.433 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6315 ms; p95 0.7010 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 4.916 ms; p95 5.269 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 36.384 ms; p95 43.595 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (118 files) | — | 8.010 ms; p95 10.616 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0853 ms; p95 0.0947 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 4.858 ms; p95 5.190 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.110 ms; p95 1.384 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.450 ms; p95 1.839 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0992-0.1357 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.175 ms at 9 files and 7.698 ms at 1025 files, with getrusage ru_stime of 6.73-13.33 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.027 ms small, 7.182 ms medium). An acquire right after a commit finds the standby stale: at medium 3/60 fell back to a cold clone and 57/60 waited for the refill in flight, p50 9.722 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.450 ms, rollback 1.110 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.128 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.462 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.194 ms, cpuOnly 0.223 ms). One call after a 3.0 ms gap takes 0.390 ms with the CPU busy in the gap and 0.846 ms with it idle (cpuOnly 0.219 / 1.544 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.56, and faster with an idle CPU, speedup 1.82). The end-to-end ane_score stage, one call per transaction, has p50 0.366 ms, 1.89x the back-to-back p50 and 0.94x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 1.06 cores (p95 1.85, max 2.40, 75 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 3.29 cores, 168.55 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
