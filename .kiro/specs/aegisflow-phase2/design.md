# Design Document — AegisFlow Phase 2

## Overview

```
                     ┌──────────── aegis-daemon (Axum, 127.0.0.1 only) ◄── apps/aegis-console (static)
                     │                 token + Host/Origin guard        contract: apps/aegis-console/API.md
                     ▼
agent edit ──► WarmBranchPool::acquire ──────────────► Branch (pool hit: one renameatx_np RENAME_EXCL)
                 ▲  refill thread: clonefile(live → standby/)   (miss: cold clonefile on the caller's thread)
                 │  invalidated by commit / bump_generation / FSEvents / age
                 │
             Branch::changes() ──► 64 features ──► ContextWindow (last 64 events of the session)
                                                     │ score_window
                                                     ▼
                                   aegis-ane-scorer: CoreML FP16 window model (.cpuAndNeuralEngine)
                                                     │ risk 0..100 of the newest event
                                                     ▼
                                   aegis-governance: Cedar 4 + redaction + ed25519 receipts
                         ┌───────────────────────────┴───────────────────────────┐
                         ▼                                                       ▼
        Branch::commit(): RENAME_SWAP, generation + 1,            Branch::rollback(): detach into trash
        old tree → versions/, pools invalidated                   (caller returns), reaper deletes it
```

The pipeline and adapters of phase 1 are unchanged; phase 2 changes how branches are produced
and discarded, what the model sees, and adds the console, the daemon and credential handling.

## Components

| Component | Responsibility | Where |
|---|---|---|
| `Workspace` | live tree + store (`<parent>/.aegis-<name>/`: branches, versions, trash, standby, generation, lock); cold branch, commit, restore | `crates/aegis-storage/src/lib.rs` |
| `WarmBranchPool` | standby clones, validation, acquire, refill thread, FSEvents watcher | `crates/aegis-storage/src/pool.rs`, `fsevents.rs` |
| Reaper | background deletion of detached branches, pruned versions and stale standbys | `crates/aegis-storage/src/reaper.rs` |
| `AneScorer` / `ContextWindow` | CoreML bridge, window packing, `MLComputePlan` report | `crates/aegis-ane-scorer/` |
| Model build | simulator, training, MIL export, compile, parity, placement | `models/build_model.py`, `harness/build_model.sh` |
| `aegis-config` | settings resolution (env, then `.env`), 0600 atomic `.env` writes | `crates/aegis-config/` |
| `aegis-daemon` + console | HTTP + WebSocket API, live stats, approvals, PayPal timeline | `crates/aegis-daemon/`, `apps/aegis-console/` |
| `aegis-bench` | hardware benchmark, gated / informational rows, `targets_note` | `crates/aegis-bench/` |
| Harness + brain | TASKS.json runner, gate, Obsidian sync, Kiro task lists | `harness/` |

## Warm branch pool: staleness model

A standby is a snapshot of live taken earlier. It carries three labels, and acquire hands it out
only if all of them are still current:

* **Generation `g`** — persisted in `<store>/generation`, incremented under the store `flock` by
  every commit and every `bump_generation()`, in any process. Read lock-free with `pread`.
* **Live root inode `i`** — changes only through a swap, and every swap installs an inode that
  has never been live before, so an unchanged `i` means the clone copied the tree with inode `i`.
* **Pool epoch `e`** — in-memory, bumped by in-process commits (through any clone of the same
  `Workspace`), explicit `invalidate()`, and every FSEvents batch that contains a change under live.

Refill reads `g` then `i`, clones, then reads `i` then `g` again and keeps the clone only if both
are unchanged and the epoch did not move (otherwise it counts a race and discards it). Commits
bump `g` after the swap, so a label can only be older than its content (discarded
conservatively), never newer. Acquire checks epoch and age, then `g` and `i` against the store,
renames, and re-checks the epoch after the rename. The returned `Branch` carries base `i` and
`g`, so `Branch::commit` applies the same optimistic-concurrency check as for a cold branch.

FSEvents filtering: events on paths below the live root always count as changes. Events on the
live root path itself cannot be classified by their flags: every `clonefile(live, …)` (the
refill, a cold `branch()`, another process) is reported there with `ItemCloned`, FSEvents carries
earlier flags of a path over into later events on it (after a commit's `RENAME_SWAP`, later clones
arrive as `ItemRenamed | ItemIsDir | ItemCloned`; a `touch`/`chmod` of the root adds
`ItemInodeMetaMod` and can carry a leftover `ItemCloned`), so a flag-based rule made each refill
invalidate the standby it had just produced. A root-path event therefore counts as a change only
if (a) it carries a must-scan-subdirs, user/kernel dropped, mount or unmount flag, or (b) live's
root inode, read with one `lstat` per batch, differs from the inode of a valid standby (live was
replaced by a commit through another `Workspace` or process, or by some tool); otherwise it is
ignored and counted in `fs_events_ignored`. The swap events of an in-process commit are ignored
as well, since that commit already bumped the epoch. Trade-off: a change to only the root
directory's own metadata (mode, owner, times, extended attributes, Finder info) no longer
invalidates standbys; a branch handed out later, and a commit of it, carries the root metadata
from its clone time, and callers that care call `bump_generation()` after such a change.
Residual window: a direct write to live is known to the pool only after FSEvents delivers it,
and writes through a
descriptor that stays open may be reported only at `close`. `max_standby_age` bounds the age of
any handed-out snapshot; `bump_generation()` is the hard guarantee for callers that edit live
directly.

### APFS serialisation and foreground priority

A directory `clonefile` in flight blocks every directory rename and clone on the same volume.
Acquire, cold branch, commit and rollback detach therefore mark themselves as foreground
operations; the refill thread starts no clone and no disposal rename while one runs, and an
acquire that finds no standby while a refill of the current epoch is in flight waits for it
instead of queueing a cold clone behind it. A refill that has already started cannot be
preempted, so a commit or rollback issued within about one clone duration of an acquire waits
behind it; `set_replenish_delay(d)` postpones refills until no foreground operation ran for `d`.
An acquire right after a commit is not a warm hit: the commit made the standby stale and the new
tree needs a fresh O(entries) clone. The benchmark reports these collision cases separately.
The gate is in-process only; other processes on the volume are not coordinated with.

## Reaper

Rollback renames the branch into `trash/` (detach, O(1)) and, in background mode (default,
`set_background_reclaim(true)`), queues the deletion to one reaper thread per workspace. The
receipt carries `queued = true`, `reclaim = 0` and a ticket; the reaper records
`ReapRecord { ticket, kind: Branch | Version | Standby, waited, duration, ok }` (bounded buffer,
`drain_reap_records`). Pruned versions after a commit and stale standbys take the same path.
`wait_reaped(timeout)` waits until nothing is queued or in progress; `reclaim_trash()` skips
entries the reaper is about to delete. Synchronous mode deletes on the caller's thread and
reports the time in `reclaim`. The reaper is detached when the workspace is dropped; anything
left in `trash/` by a process that exits is removed by the next `reclaim_trash()`.

## Context-window risk model

* Input `features` FP16 `[1, 64, 1, 64]`: channels are the 64 features of
  `models/feature_schema.json`, width holds the last 64 events of one session, oldest → newest,
  zero-padded on the left (zero = no event). Output `risk_score` `[1,1,1,1]` in 0..100.
* Metadata `aegis_input_layout = window`, `window_length = 64`; the batch model uses
  `window_batch` with B sessions along the batch axis. The bridge refuses 4-D models without the
  layout key.
* Architecture and size are recorded by the build in `models/model_card.json` (convolutional
  tower over several recency views of the window, sigmoid × 100). Placement is not assumed:
  `compute_plan()` reads `MLComputePlan` for `.cpuAndNeuralEngine` and reports every dispatched
  op's preferred device.
* `models/parity_vectors.json` (format 2) holds single events (`vectors`, `reference_fp32`) and
  multi-event sessions (`windows`, sparse `[index, value]` events with `reference_fp32`); the
  reference is the FP32 forward pass of the trained weights.
* Daemon usage: one `ContextWindow` per agent session (`scorer.new_window()`), `push` every
  event's features, `score_window`, `clear` when the session ends. Predictions on one
  `AneScorer` handle are serialised; open more handles for parallel predictions.

## Console, daemon and credentials

The console ↔ daemon contract (endpoints, WebSocket messages, shapes, security rules) is
specified in **`apps/aegis-console/API.md`**; that file is the source of truth for both sides.
Design points that matter for the requirements:

* The daemon binds `127.0.0.1` only, checks `Host` / `Origin`, and requires the per-process
  token (substituted into `index.html`) on every `POST` and on the WebSocket.
* It creates one `Arc<Workspace>` per live tree and routes every branch, commit and pool
  through it, so commits invalidate pools immediately and the foreground gate covers all
  callers; if it edits live directly it calls `bump_generation()`.
* `aegis-config::Settings` resolves env first, then `.env`; `export_to_process_env` runs once
  at the top of `main` (daemon and bench) before any thread or CoreML init. `.env` is written
  0600 through a temporary file and an atomic rename. Secrets are write-only on every surface
  (CLI reads values from stdin; API never echoes them).
* Remote services (Nebius, Tavily, PayPal Sandbox, Mattermost / Slack) are called only when
  configured; otherwise the daemon reports `enabled: false` with a reason.

## Benchmark design (`aegis-bench`)

* `.env` is loaded through `aegis-config` before anything else; live services need
  `AEGIS_BENCH_LIVE=1` plus credentials.
* APFS, per fixture (tiny, small, medium, aegisflow_src): kernel time of one `clonefile`
  (`getrusage` deltas, no other thread active), cold branch + background rollback, synchronous
  rollback, pool warm-hit acquire (after `wait_ready`; rollback outside the timed region and
  after the refill finished), standby replenish, commit after the refill finished, acquire
  right after a commit (source recorded), collision cases, reaper durations by ticket, store
  cleanliness after the pool is dropped, and a live-tree manifest check after every phase.
* CoreML: placement and per-op list, latency per compute unit with units interleaved in rounds
  (one full window per call), speedup, parity for single events and sessions, batch model.
  Two call patterns: back-to-back calls (gated) and isolated calls, one prediction after a fixed
  gap in which the calling thread spins (CPU busy) or sleeps (CPU idle), for cpuAndNeuralEngine
  and cpuOnly (informational, with their own speedup). The end-to-end `ane_score` stage is
  published next to them because a transaction makes one prediction.
* End-to-end: pool acquire (replenish delay sized from the measured cold clone) → edit → diff →
  features → `score_window` over a per-agent `ContextWindow` → Cedar → rollback / commit.
* Output: the full measurement tree, `targets_summary` (exactly 11 gated rows, `pass` computed
  from measurements, each latency row with p50, p95 and `n`), `informational` (not gated, each row
  tagged with its `section`), and a generated `targets_note`.
* Gating preconditions: a branch-acquire row passes only when every acquire made with the pool
  settled was a warm hit (`measured_hit_rate` = 1); the APFS gated rows require live-tree
  integrity in every phase and a clean store after the pool is dropped.
* Run conditions: `run_conditions.preliminary` is true for a reduced-scale run, when the 1-minute
  load average at start or end is above the performance-core count, or when the source
  fingerprint (`sources`: SHA-256 over the tracked and untracked-but-not-ignored files under
  `crates`, `policies`, `models`, `Cargo.toml`, `Cargo.lock`) changed during the run.
  `--quiet-wait S` waits up to S seconds for the load to drop before measuring. A reduced-scale
  run refuses to write the canonical artifact.
* Harness: the benchmark runs once per pass (T17). `aegis-bench --verify phase1` (T12) and
  `--verify gate` (T18) check the written artifact: schema, scale, and a fingerprint equal to
  the working tree; the gate additionally needs 11/11 gated rows, the integrity row and a run that
  is not preliminary.

## Error handling

* Storage errors carry the failing syscall (`StorageError::Io { op, path, source }`); a commit
  on a stale base is `Conflict` and rolls the branch back.
* A refill failure backs off exponentially and is reported in `PoolStats.last_error`; acquire
  then falls back to a cold clone.
* The bench aborts on any storage or CoreML error rather than writing partial numbers. A failed
  live-tree integrity check is written into the artifact (APFS gated rows `pass: false`, integrity
  row false) and then makes the bench exit non-zero. Missed targets are not errors and are written
  as `pass: false`.

## Testing strategy

| Area | Verified by |
|---|---|
| Pool staleness, concurrency, FSEvents, orphans, reaper | `cargo test --release -p aegis-storage` (T14) |
| Model artifacts, layout metadata, parity, placement | `bash harness/build_model.sh --verify-only` (T15) |
| Window API (`ContextWindow`, one-event equals `score()`) | `cargo test --release -p aegis-ane-scorer` (T15) |
| Daemon security, API contract, credentials | `cargo test --release -p aegis-config`, `-p aegis-daemon` (T16) |
| Hardware numbers and integrity | `aegis-bench` run + structure check (T17), `--verify phase1` (T12), `--verify gate` (T18) |
| Summary rows, gate preconditions, run conditions, artifact checks | `cargo test --release -p aegis-bench` |
| Brain rendering, preserved layout and user blocks | `python3 harness/sync_obsidian_brain.py --no-open --require-phase2` (T20) |

## Known limits

Numbers for each item are in the benchmark artifact, not here.

* The pool hides the clone cost from acquire but does not remove it: every refill is one cold
  clone of CPU and APFS time, O(entries) in the kernel.
* Commit or rollback within about one clone duration of an acquire waits behind the refill
  clone; an acquire right after a commit is usually cold or waits for the refill.
* FSEvents detection of direct edits to live is best effort (delivery delay, coalescing,
  descriptors kept open); `bump_generation()` is the guarantee.
* The Neural Engine has a fixed cost per prediction, so the window model is slower in absolute
  terms than the phase-1 per-event CPU model; its speedup over `.cpuOnly` is modest and must be
  re-measured on a quiet machine. The gated latency and speedup are back-to-back figures; the
  isolated-call rows show what one prediction per transaction costs on each unit.
* The risk model is trained on synthetic sessions labelled by a hand-written teacher; it is not
  validated on real incidents.
