---
tags:
  - benchmark
status: verified
latency_ms: 0.114
last_verified: 2026-10-26T03:52:53Z
git_commit: 4403ab4-dirty
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1140 ms · **commit:** `4403ab4-dirty` · **bench run: PRELIMINARY**

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.1067 ms · p95 0.2690 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1759 ms · p95 0.4012 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 182.0 µs · p95 414.0 µs · n=400 · 15.167 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.2157 ms · p95 0.4907 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1197 ms · p95 0.2925 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4522 ms · p95 0.9930 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0912 ms · p95 0.2336 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6879 ms · p95 1.688 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.4288 ms · p95 0.9321 ms · n=400 · cold 102 / waited 298 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1140 ms · p95 0.2541 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 1.034 ms · p95 1.243 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 1041.0 µs · p95 1224.0 µs · n=300 · 7.543 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.097 ms · p95 2.285 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1303 ms · p95 0.3031 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4717 ms · p95 0.9522 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0932 ms · p95 0.2145 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 5.097 ms · p95 7.247 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.589 ms · p95 2.240 ms · n=300 · cold 111 / waited 189 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1497 ms · p95 0.3179 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 7.321 ms · p95 8.411 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 7354.0 µs · p95 8414.0 µs · n=60 · 6.951 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 7.247 ms · p95 9.271 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1467 ms · p95 0.1968 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.5020 ms · p95 0.9726 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.1070 ms · p95 0.2744 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 37.545 ms · p95 44.267 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 10.158 ms · p95 11.998 ms · n=60 · cold 8 / waited 52 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 113 files / 5.482 MiB · acquire (pool warm hit) | p50 0.1109 ms · p95 0.2770 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 113 files / 5.482 MiB · clonefile (cold) | p50 1.322 ms · p95 1.981 ms · n=200 |
| aegisflow_src · 113 files / 5.482 MiB · kernel sys per clonefile | p50 1285.0 µs · p95 1605.0 µs · n=200 · 7.835 µs/entry |
| aegisflow_src · 113 files / 5.482 MiB · standby replenish | p50 1.310 ms · p95 2.199 ms · n=200 |
| aegisflow_src · 113 files / 5.482 MiB · RENAME_SWAP | p50 0.1330 ms · p95 0.3041 ms · n=200 |
| aegisflow_src · 113 files / 5.482 MiB · commit caller-visible | p50 0.4704 ms · p95 0.7825 ms · n=200 |
| aegisflow_src · 113 files / 5.482 MiB · rollback caller-visible (background reap) | p50 0.0922 ms · p95 0.1756 ms · n=200 |
| aegisflow_src · 113 files / 5.482 MiB · background reap | p50 7.857 ms · p95 9.424 ms · n=200 |
| aegisflow_src · 113 files / 5.482 MiB · acquire right after commit | p50 1.946 ms · p95 2.546 ms · n=200 · cold 34 / waited 166 / pool 0 |
| aegisflow_src · 113 files / 5.482 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1067 ms; hit rate 1.000; p95 0.2690 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1140 ms; hit rate 1.000; p95 0.2541 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1497 ms; hit rate 1.000; p95 0.3179 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (113 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1109 ms; hit rate 1.000; p95 0.2770 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1303 ms; p95 0.3031 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0932 ms; p95 0.2145 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1759 ms; p95 0.4012 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 1.034 ms; p95 1.243 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 7.321 ms; p95 8.411 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (113 files) | < 0.5 ms (same reference as the pool rows) | 1.322 ms; p95 1.981 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 15.167 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 7.543 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 6.951 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (113 files) | — (O(entries) inside the kernel) | 7.835 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.2157 ms; p95 0.4907 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.097 ms; p95 2.285 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 7.247 ms; p95 9.271 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (113 files) | — | 1.310 ms; p95 2.199 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4522 ms; p95 0.9930 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4717 ms; p95 0.9522 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.5020 ms; p95 0.9726 ms; n=60 | not gated (would fail) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (113 files) | < 0.5 ms (reference) | 0.4704 ms; p95 0.7825 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.4288 ms; pool_ready 0 · pool_after_waiting_for_refill 298 · cold 102; p95 0.9321 ms; n=400 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.589 ms; pool_ready 0 · pool_after_waiting_for_refill 189 · cold 111; p95 2.240 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 10.158 ms; pool_ready 0 · pool_after_waiting_for_refill 52 · cold 8; p95 11.998 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (113 files) | < 0.5 ms (reference) | 1.946 ms; pool_ready 0 · pool_after_waiting_for_refill 166 · cold 34; p95 2.546 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6879 ms; p95 1.688 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 5.097 ms; p95 7.247 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 37.545 ms; p95 44.267 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (113 files) | — | 7.857 ms; p95 9.424 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0897 ms; p95 0.2050 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 5.116 ms; p95 7.067 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.072 ms; p95 1.268 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.463 ms; p95 2.311 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.1067-0.1497 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.176 ms at 9 files and 7.321 ms at 1025 files, with getrusage ru_stime of 6.95-15.17 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.097 ms small, 7.247 ms medium). An acquire right after a commit finds the standby stale: at medium 8/60 fell back to a cold clone and 52/60 waited for the refill in flight, p50 10.158 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.463 ms, rollback 1.072 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.130 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.472 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.181 ms, cpuOnly 0.229 ms). One call after a 3.0 ms gap takes 0.360 ms with the CPU busy in the gap and 0.674 ms with it idle (cpuOnly 0.238 / 1.529 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.66, and faster with an idle CPU, speedup 2.27). The end-to-end ane_score stage, one call per transaction, has p50 0.395 ms, 2.18x the back-to-back p50 and 1.10x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. This run is PRELIMINARY (mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs)); only a quiet full-scale run decides the phase result; the end-of-run load average and the total external CPU (system minus own: mean 4.11 cores, 146.58 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
