//! Preliminary caller-visible latencies of cold vs pooled branching, background vs synchronous
//! rollback, and FSEvents invalidation delay.
//!
//! For trees of about 9, 61, 129 and 1025 entries:
//! * cold `Workspace::branch()` (caller-visible) and its clonefile time;
//! * pooled `WarmBranchPool::acquire()` on a warm hit (after `wait_ready`), its rename time, and
//!   the replenisher's per-standby duration;
//! * caller-visible `Branch::rollback()` in background mode (detach only) and in synchronous mode
//!   (detach + delete), and the reaper's deletion time per tree;
//! * the APFS collision cases: rollback right after a pooled acquire (the replenish clone is in
//!   flight, see `apfs_contention.rs`) with the default replenish delay and with
//!   `set_replenish_delay`, and commit followed immediately by the next acquire.
//!
//! The reaper and replenisher are idle at the start of each timed call (`wait_reaped` /
//! `wait_ready` between iterations), so each number is the operation alone; in real use the
//! background work overlaps the agent's work. Then, on the small tree, the delay from a write in
//! live to the pool's FSEvents invalidation, and whether a write through a descriptor that stays
//! open is reported before `close`.
//! Run: `cargo run --release -p aegis-storage --example pool_latency [-- --iterations N]`.

mod support;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aegis_storage::{AcquireSource, PoolConfig, ReapKind, WarmBranchPool, Workspace};
use support::{build_tree, iterations_override, micros, obj, percentile, print_json, scratch_root, Json, TREE_ENTRIES};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const ITERATIONS: [usize; 4] = [300, 200, 200, 60];
const WARMUP: usize = 5;
const WAIT: Duration = Duration::from_secs(30);
const FS_TRIALS: usize = 20;
const OPEN_FD_TRIALS: usize = 5;
/// How long to wait for an event while the written descriptor is still open.
const OPEN_FD_WAIT: Duration = Duration::from_secs(2);

fn ensure(ok: bool, what: &str) -> Res<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("timed out waiting for {what}").into())
    }
}

fn bench_tree(entries: usize, iterations: usize, root: &std::path::Path) -> Res<Json> {
    let base = root.join(format!("tree-{entries}"));
    let live = base.join("live");
    let info = build_tree(&live, entries, 1024, 16 * 1024)?;
    let ws = Arc::new(Workspace::open(&live)?);
    eprintln!("pool_latency: {entries} entries x {iterations}");

    // Cold branch + background rollback.
    for _ in 0..WARMUP {
        ws.branch("warmup")?.rollback()?;
    }
    ensure(ws.wait_reaped(WAIT), "reaper")?;
    ws.drain_reap_records();
    let (mut cold, mut cold_clone, mut rb_bg, mut detach) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..iterations {
        let started = Instant::now();
        let branch = ws.branch("cold")?;
        cold.push(micros(started.elapsed()));
        cold_clone.push(micros(branch.clone_duration()));
        fs::write(branch.path().join("agent_edit.rs"), format!("// edit {i}\n"))?;
        let started = Instant::now();
        let receipt = branch.rollback()?;
        rb_bg.push(micros(started.elapsed()));
        detach.push(micros(receipt.detach));
        if !receipt.queued {
            return Err("background reclaim expected".into());
        }
        ensure(ws.wait_reaped(WAIT), "reaper")?;
    }
    let records = ws.drain_reap_records();
    let reap: Vec<f64> = records.iter().filter(|r| r.kind == ReapKind::Branch && r.ok).map(|r| micros(r.duration)).collect();
    let reap_wait: Vec<f64> = records.iter().filter(|r| r.kind == ReapKind::Branch).map(|r| micros(r.waited)).collect();

    // Synchronous rollback for comparison.
    ws.set_background_reclaim(false);
    let mut rb_sync = Vec::new();
    for i in 0..iterations {
        let branch = ws.branch("sync")?;
        fs::write(branch.path().join("agent_edit.rs"), format!("// edit {i}\n"))?;
        let started = Instant::now();
        branch.rollback()?;
        rb_sync.push(micros(started.elapsed()));
    }
    ws.set_background_reclaim(true);

    // Pooled acquire (warm hits) + background rollback right after it (replenish delay 0).
    let pool = WarmBranchPool::new(Arc::clone(&ws), PoolConfig { capacity: 1, max_standby_age: None, watch_fs_events: true })?;
    for _ in 0..WARMUP {
        ensure(pool.wait_ready(WAIT), "standby")?;
        pool.acquire("warmup")?.0.rollback()?;
    }
    ensure(ws.wait_reaped(WAIT), "reaper")?;
    ensure(pool.wait_ready(WAIT), "standby")?;
    pool.drain_replenish_durations();
    let warm_stats = pool.stats();
    let (mut acquire, mut rename, mut acquire_cold, mut rb_pool) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..iterations {
        ensure(pool.wait_ready(WAIT), "standby")?;
        let started = Instant::now();
        let (branch, receipt) = pool.acquire("pooled")?;
        let took = micros(started.elapsed());
        match receipt.source {
            AcquireSource::Pool => {
                acquire.push(took);
                rename.extend(receipt.rename.map(micros));
            }
            AcquireSource::Cold => acquire_cold.push(took),
        }
        fs::write(branch.path().join("agent_edit.rs"), format!("// edit {i}\n"))?;
        let started = Instant::now();
        branch.rollback()?;
        rb_pool.push(micros(started.elapsed()));
        ensure(ws.wait_reaped(WAIT), "reaper")?;
    }
    ensure(pool.wait_ready(WAIT), "standby")?;
    let replenish: Vec<f64> = pool.drain_replenish_durations().into_iter().map(micros).collect();
    let stats = pool.stats();

    // Same loop with a replenish delay of twice the measured cold-branch p50 (at least 1 ms), so
    // the rollback right after acquire does not meet the replenish clone.
    let delay_us = percentile(&cold, 0.5).unwrap_or(0.0).mul_add(2.0, 0.0).max(1000.0);
    let delay = Duration::from_secs_f64(delay_us / 1e6);
    pool.set_replenish_delay(delay);
    let (mut acquire_delayed, mut rb_delayed) = (Vec::new(), Vec::new());
    for i in 0..iterations {
        ensure(pool.wait_ready(WAIT), "standby")?;
        let started = Instant::now();
        let (branch, receipt) = pool.acquire("pooled-delay")?;
        if receipt.source == AcquireSource::Pool {
            acquire_delayed.push(micros(started.elapsed()));
        }
        fs::write(branch.path().join("agent_edit.rs"), format!("// edit {i}\n"))?;
        let started = Instant::now();
        branch.rollback()?;
        rb_delayed.push(micros(started.elapsed()));
        ensure(ws.wait_reaped(WAIT), "reaper")?;
    }
    pool.set_replenish_delay(Duration::ZERO);

    // Acquire, commit immediately, acquire immediately (default delay 0). The commit meets the
    // replenish clone started after the first acquire; the second acquire finds the standby
    // invalidated by the commit and either waits for the replenish clone in flight or clones cold.
    let (mut commit_after_acquire, mut acquire_after_commit) = (Vec::new(), Vec::new());
    let (mut after_commit_pool, mut after_commit_waited, mut after_commit_cold) = (0u64, 0u64, 0u64);
    for i in 0..iterations {
        ensure(pool.wait_ready(WAIT), "standby")?;
        let (branch, _) = pool.acquire("commit")?;
        fs::write(branch.path().join("agent_commit.rs"), format!("// commit {i}\n"))?;
        let started = Instant::now();
        branch.commit()?;
        commit_after_acquire.push(micros(started.elapsed()));
        let started = Instant::now();
        let (next, receipt) = pool.acquire("after-commit")?;
        acquire_after_commit.push(micros(started.elapsed()));
        match (receipt.source, receipt.waited_for_replenish.is_some()) {
            (AcquireSource::Pool, true) => after_commit_waited += 1,
            (AcquireSource::Pool, false) => after_commit_pool += 1,
            (AcquireSource::Cold, _) => after_commit_cold += 1,
        }
        next.rollback()?;
        ensure(ws.wait_reaped(WAIT), "reaper")?;
    }
    drop(pool);
    ensure(ws.wait_reaped(WAIT), "reaper")?;
    drop(ws);
    fs::remove_dir_all(&base)?;

    let ratio = percentile(&cold, 0.5).zip(percentile(&acquire, 0.5)).filter(|(_, p)| *p > 0.0).map(|(c, p)| c / p);
    Ok(obj(vec![
        ("entries", Json::Int(info.entries as u64)),
        ("files", Json::Int(info.files as u64)),
        ("dirs", Json::Int(info.dirs as u64)),
        ("iterations", Json::Int(iterations as u64)),
        ("cold_branch_us", Json::summary(&cold)),
        ("cold_clonefile_us", Json::summary(&cold_clone)),
        ("pooled_acquire_hit_us", Json::summary(&acquire)),
        ("pooled_acquire_miss_us", Json::summary(&acquire_cold)),
        ("pooled_rename_us", Json::summary(&rename)),
        ("cold_over_pooled_p50", Json::opt(ratio)),
        ("pool_hits", Json::Int(stats.hits - warm_stats.hits)),
        ("pool_misses", Json::Int(stats.misses - warm_stats.misses)),
        ("pool_stale_discards", Json::Int(stats.stale_discards - warm_stats.stale_discards)),
        ("pool_fs_events", Json::Int(stats.fs_events)),
        ("pool_fs_events_ignored", Json::Int(stats.fs_events_ignored)),
        ("replenish_us", Json::summary(&replenish)),
        ("rollback_background_caller_us", Json::summary(&rb_bg)),
        ("rollback_background_detach_us", Json::summary(&detach)),
        ("rollback_right_after_pooled_acquire_us", Json::summary(&rb_pool)),
        ("replenish_delay_us", Json::Num(delay_us)),
        ("pooled_acquire_hit_with_delay_us", Json::summary(&acquire_delayed)),
        ("rollback_right_after_pooled_acquire_with_delay_us", Json::summary(&rb_delayed)),
        ("commit_right_after_pooled_acquire_us", Json::summary(&commit_after_acquire)),
        ("acquire_right_after_commit_us", Json::summary(&acquire_after_commit)),
        ("acquire_right_after_commit_sources", obj(vec![
            ("pool_ready", Json::Int(after_commit_pool)),
            ("pool_after_waiting_for_replenish", Json::Int(after_commit_waited)),
            ("cold", Json::Int(after_commit_cold)),
        ])),
        ("rollback_sync_caller_us", Json::summary(&rb_sync)),
        ("reap_us", Json::summary(&reap)),
        ("reap_queue_wait_us", Json::summary(&reap_wait)),
    ]))
}

/// Writes a sentinel into live, renames it, and waits until the rename is reported: every event
/// older than the rename has then been delivered (FSEvents delivers in event-id order).
fn settle(pool: &WarmBranchPool, live: &std::path::Path, n: usize) -> Res<()> {
    let written = live.join(format!(".settle-{n}"));
    let renamed = live.join(format!(".settled-{n}"));
    fs::write(&written, "settle\n")?;
    fs::rename(&written, &renamed)?;
    ensure(pool.wait_until(WAIT, |s| s.recent_fs_changes.iter().any(|c| c.path == renamed)), "sentinel event")?;
    ensure(pool.wait_ready(WAIT), "standby")
}

fn fs_event_delivery(root: &std::path::Path) -> Res<Json> {
    let base = root.join("fsevents");
    let live = base.join("live");
    build_tree(&live, 129, 1024, 16 * 1024)?;
    fs::write(live.join("held.log"), "")?;
    let ws = Arc::new(Workspace::open(&live)?);
    let pool = WarmBranchPool::new(Arc::clone(&ws), PoolConfig { capacity: 1, max_standby_age: None, watch_fs_events: true })?;
    ensure(pool.wait_ready(WAIT), "standby")?;
    let mut sentinel = 0;
    settle(&pool, &live, sentinel)?;
    eprintln!("pool_latency: FSEvents delivery x {FS_TRIALS}");

    // In-place write (open, truncate, write, close) of an existing file.
    let mut delays = Vec::new();
    let mut missed = 0u64;
    for i in 0..FS_TRIALS {
        let epoch = pool.invalidation_epoch();
        fs::write(live.join("README.md"), format!("# edit {i}\n"))?;
        let written = Instant::now();
        if pool.wait_invalidation(epoch, WAIT) {
            delays.push(micros(written.elapsed()));
        } else {
            missed += 1;
        }
        sentinel += 1;
        settle(&pool, &live, sentinel)?;
    }

    // Append through a descriptor that stays open, then close it.
    let mut open_fd = Vec::new();
    for i in 0..OPEN_FD_TRIALS {
        let epoch = pool.invalidation_epoch();
        let mut file = OpenOptions::new().append(true).open(live.join("held.log"))?;
        file.write_all(format!("line {i}\n").as_bytes())?;
        let written = Instant::now();
        let before_close = pool.wait_invalidation(epoch, OPEN_FD_WAIT);
        let before_close_us = before_close.then(|| micros(written.elapsed()));
        let epoch_at_close = pool.invalidation_epoch();
        drop(file);
        let closed = Instant::now();
        let after_close = before_close || pool.wait_invalidation(epoch_at_close, WAIT);
        let after_close_us = (!before_close && after_close).then(|| micros(closed.elapsed()));
        open_fd.push(obj(vec![
            ("reported_while_open", Json::Bool(before_close)),
            ("write_to_invalidation_us", Json::opt(before_close_us)),
            ("reported_after_close", Json::Bool(after_close)),
            ("close_to_invalidation_us", Json::opt(after_close_us)),
        ]));
        sentinel += 1;
        settle(&pool, &live, sentinel)?;
    }
    let stats = pool.stats();
    drop(pool);
    ensure(ws.wait_reaped(WAIT), "reaper")?;
    drop(ws);
    fs::remove_dir_all(&base)?;
    Ok(obj(vec![
        ("in_place_write_trials", Json::Int(FS_TRIALS as u64)),
        ("in_place_write_to_invalidation_us", Json::summary(&delays)),
        ("in_place_write_not_reported", Json::Int(missed)),
        ("open_fd_wait_limit_us", Json::Num(micros(OPEN_FD_WAIT))),
        ("open_fd_append", Json::Arr(open_fd)),
        ("fs_events", Json::Int(stats.fs_events)),
        ("fs_events_ignored", Json::Int(stats.fs_events_ignored)),
    ]))
}

fn main() -> Res<()> {
    let root = scratch_root("pool-latency")?;
    let mut trees = Vec::new();
    for (idx, &entries) in TREE_ENTRIES.iter().enumerate() {
        trees.push(bench_tree(entries, iterations_override().unwrap_or(ITERATIONS[idx]), &root)?);
    }
    let fsevents = fs_event_delivery(&root)?;
    fs::remove_dir_all(&root)?;
    print_json(&obj(vec![
        ("example", Json::Str("pool_latency".into())),
        (
            "note",
            Json::Str(
                "preliminary: measured on a shared machine with concurrent compiles; the lead runs the quiet benchmark"
                    .into(),
            ),
        ),
        ("trees", Json::Arr(trees)),
        ("fsevents", fsevents),
    ]));
    Ok(())
}
