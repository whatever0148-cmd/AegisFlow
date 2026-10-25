//! Warm branch pool: pre-cloned standby copies of the live tree.
//!
//! `clonefile(2)` of a directory costs time proportional to the number of entries (APFS creates a
//! clone record per inode inside the one syscall). [`WarmBranchPool`] moves that cost off the
//! caller's path: a replenisher thread keeps up to `capacity` standby clones of live in
//! `<store>/standby/`, and [`WarmBranchPool::acquire`] hands one out by renaming it into
//! `<store>/branches/<id>` with `renameatx_np(RENAME_EXCL)` (one O(1) syscall), then wakes the
//! replenisher. When no valid standby is available, acquire falls back to a cold
//! [`Workspace::branch`].
//!
//! # Never handing out a stale standby
//!
//! A standby is a snapshot of live taken earlier, so it carries the state it was cloned from:
//! the live **generation** `g` (persisted in `<store>/generation`, bumped under the store lock by
//! every commit and by [`Workspace::bump_generation`], in any process), the live root **inode** `i`,
//! and the pool's in-memory invalidation **epoch** `e`.
//!
//! * Replenish reads `g` then `i`, clones, then reads `i` then `g` again, and keeps the clone only
//!   if both are unchanged and the epoch did not move. Live's inode changes only through a swap,
//!   and every swap installs an inode that has never been live before (a fresh clone; old trees go
//!   to `versions/` and `restore` clones them again), so an unchanged `i` means the clone copied
//!   the tree with inode `i`. Commits bump `g` *after* the swap, so a label can only be older than
//!   its content (then it is discarded conservatively), never newer.
//! * Acquire hands out a standby only if its epoch is current, its age is within
//!   `max_standby_age`, the generation file still says `g` and live's inode is still `i`. Otherwise
//!   the standby is discarded (counted in `stale_discards`) and the next one, or a cold clone
//!   (counted in `misses`), is used. The epoch is checked once more after the rename.
//! * The returned [`Branch`] has base inode `i` and base generation `g`, so the regular
//!   optimistic-concurrency check in [`Branch::commit`] applies unchanged. A commit that lands
//!   after acquire's check behaves exactly as for a cold branch created just before that commit:
//!   the later commit is rejected as a conflict.
//! * In-process commits through any clone of the same [`Workspace`] also bump the epoch of its
//!   pools immediately, so they re-clone right away instead of discovering staleness at acquire.
//!
//! # Edits that bypass this crate
//!
//! Direct writes to the live tree change neither `g` nor `i`. Two mechanisms cover them:
//!
//! * `watch_fs_events: true` watches live with FSEvents (see `fsevents.rs`); every delivered batch
//!   that contains a change bumps the epoch, which invalidates all standbys. Events reported on
//!   the live root path itself get a different rule. `clonefile(live, ..)` (our replenisher, a
//!   cold `branch()`, another process cloning live) is reported there with `ItemCloned`, and
//!   FSEvents carries earlier flags of a path over into later events on it. On macOS 27, after a
//!   commit's `RENAME_SWAP`, every later clone of live arrives as `ItemRenamed | ItemIsDir |
//!   ItemCloned`. After a `touch` or `chmod` of the root it arrives with `ItemInodeMetaMod` added,
//!   and the touch itself can carry a leftover `ItemCloned`. The flags therefore cannot separate a
//!   clone of live from a change to the root directory. Counting them made each refill invalidate
//!   the standby it had just produced, in a loop after every commit. A root-path event therefore
//!   counts as a change only if (a) it carries a dropped-events, must-scan or (un)mount flag, or
//!   (b) live's root inode (one `lstat` per batch) differs from the inode of a valid standby,
//!   because a commit through another [`Workspace`] or process, or some tool, replaced live.
//!   Edits below the root arrive on their own paths and always count. The swap events of an
//!   in-process commit (`RootChanged`, `ItemRenamed`) are ignored as well, because that commit has
//!   already invalidated the pool and the refill clones the new tree. **Trade-off:** a change to
//!   only the root directory's own metadata (mode, owner, times, extended attributes, Finder info)
//!   does not invalidate standbys. A branch handed out later, and a commit of it, carries the root
//!   metadata from its clone time; call [`Workspace::bump_generation`] after such a change if that
//!   matters. **Residual window:** a write that completes
//!   at time T is known to the pool only when its event is delivered at T + d, where d is the
//!   FSEvents delivery latency (kernel, then `fseventsd`, then our dispatch queue; after the first
//!   event of a burst, further events are coalesced for 10 ms). An acquire in [T, T + d) can return
//!   a standby that lacks that write. `examples/pool_latency.rs` measures d and also checks
//!   whether a write through a descriptor that is still open is reported before `close`. FSEvents
//!   can also coalesce or drop events under load; dropped-event notifications still arrive as
//!   events and invalidate, but this is best effort, not a guarantee.
//! * `max_standby_age` bounds how old a handed-out snapshot can be, whatever the cause.
//!
//! Callers that edit live directly and need a hard guarantee should call
//! [`Workspace::bump_generation`] after the edit (all pools in all processes then discard their
//! standbys), or use cold branches.
//!
//! # APFS serialisation and foreground priority
//!
//! Measured with `examples/apfs_contention.rs`: while a directory `clonefile` runs, every
//! directory rename and every other `clonefile` on the same volume waits for it to finish (file
//! creation, writes and `stat` do not). A replenish clone in flight would therefore delay an
//! acquire's rename, a cold clone, a commit's `RENAME_SWAP` or a rollback's detach by up to one
//! clone duration. In this process the pool avoids that where it can:
//!
//! * acquire, cold `branch`/`restore`, commit and rollback-detach mark themselves as foreground
//!   operations on the workspace; while any is running the replenisher starts no clone and no
//!   disposal rename, and acquire wakes the replenisher only after its own rename or cold clone;
//! * an acquire that finds no standby while a clone for the current epoch is in flight waits for
//!   that clone instead of starting a cold clone that would queue behind it
//!   ([`AcquireReceipt::waited_for_replenish`]);
//! * a clone that has already started cannot be preempted. A commit or rollback issued right
//!   after an acquire (before the replenish clone of about one cold-clone duration finishes)
//!   waits for it. [`WarmBranchPool::set_replenish_delay`] postpones replenishing until no
//!   foreground operation has run for the given time, for callers whose branches are short-lived.
//!
//! Other processes on the same volume are not coordinated with.
//!
//! # Threads and cleanup
//!
//! The pool owns one replenisher thread and, with `watch_fs_events`, one FSEvents stream on a
//! private dispatch queue. Dropping the pool stops both and disposes of every standby through the
//! workspace's reaper (or inline in synchronous reclaim mode). Standbys left behind by a crashed
//! process (their name starts with its pid) are disposed of when the next pool on the same store
//! starts.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::fsevents::{self, Watcher};
use crate::reaper::ReapKind;
use crate::{live_inode, new_branch_id, remove_any, sys, unique_suffix, Branch, BranchOrigin, StorageError, Workspace};

/// Default upper bound on a standby's age before it is re-cloned.
pub const DEFAULT_MAX_STANDBY_AGE: Duration = Duration::from_secs(30);
/// FSEvents latency parameter: coalescing interval after the first event of a burst.
const FS_EVENT_COALESCE: Duration = Duration::from_millis(10);
/// First retry delay after a failed replenish; doubles up to `RETRY_CAP`.
const RETRY_BASE: Duration = Duration::from_millis(50);
const RETRY_CAP: Duration = Duration::from_secs(5);
/// Bound on replenish samples kept for [`WarmBranchPool::drain_replenish_durations`].
const MAX_SAMPLES: usize = 1 << 16;
/// Safety re-check interval while foreground operations hold the replenisher back.
const FOREGROUND_RECHECK: Duration = Duration::from_millis(100);
/// Upper bound on how long acquire waits for an in-flight replenish clone.
const REPLENISH_WAIT_LIMIT: Duration = Duration::from_secs(10);
/// Change events kept in [`PoolStats::recent_fs_changes`].
const RECENT_FS_CHANGES: usize = 16;

/// Pool configuration. Construct with `PoolConfig { capacity: 2, ..Default::default() }`.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Standbys kept ready (default 1). 0 disables pre-cloning: every acquire is cold.
    pub capacity: usize,
    /// Standbys older than this are discarded and re-cloned (default [`DEFAULT_MAX_STANDBY_AGE`]).
    pub max_standby_age: Option<Duration>,
    /// Invalidate standbys on any FSEvents event under live (default true).
    pub watch_fs_events: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self { capacity: 1, max_standby_age: Some(DEFAULT_MAX_STANDBY_AGE), watch_fs_events: true }
    }
}

/// Where an acquired branch came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AcquireSource {
    /// A valid standby was renamed into place.
    Pool,
    /// No valid standby: a cold `clonefile` on the caller's thread.
    Cold,
}

/// Timing and provenance of one [`WarmBranchPool::acquire`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct AcquireReceipt {
    pub source: AcquireSource,
    /// Caller-visible time of the whole acquire (validation, discards, rename or cold clone).
    pub total: Duration,
    /// Time inside `renameatx_np(RENAME_EXCL)` for a pool hit.
    pub rename: Option<Duration>,
    /// Standbys discarded as stale during this acquire.
    pub stale_discarded: usize,
    /// Age of the handed-out standby (time since its clone started).
    pub standby_age: Option<Duration>,
    /// Time spent waiting for an in-flight replenish clone instead of cloning cold (included in
    /// `total`).
    pub waited_for_replenish: Option<Duration>,
}

/// One FSEvents event that counted as a change under live.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FsChange {
    pub path: PathBuf,
    /// Raw `kFSEventStreamEventFlag*` bits.
    pub flags: u32,
}

/// Snapshot of pool counters.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PoolStats {
    /// Standbys currently valid by epoch and age (generation is checked at acquire).
    pub ready: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
    /// Standbys discarded because of a commit, generation bump, FSEvents event, or age.
    pub stale_discards: u64,
    /// Duration of the most recent successful replenish (reads + clonefile + checks).
    pub last_replenish: Option<Duration>,
    /// A replenish clone is in progress.
    pub replenishing: bool,
    /// Quiet period after foreground operations before a replenish clone starts.
    pub replenish_delay: Duration,
    pub replenished: u64,
    /// Clones thrown away because live changed while they were being made.
    pub replenish_races: u64,
    pub replenish_failures: u64,
    /// FSEvents events that counted as changes under live.
    pub fs_events: u64,
    /// FSEvents events on the live root path that were ignored: clones of live, the swaps of
    /// in-process commits and the root's own metadata changes, while live's inode still matched
    /// every valid standby (see module docs).
    pub fs_events_ignored: u64,
    /// Most recent FSEvents change events, oldest first (diagnostics: what invalidated standbys).
    pub recent_fs_changes: Vec<FsChange>,
    /// Epoch bumps (FSEvents batches, in-process commits, explicit invalidations).
    pub invalidations: u64,
    pub epoch: u64,
    pub watching_fs_events: bool,
    pub paused: bool,
    pub last_error: Option<String>,
}

struct Standby {
    name: String,
    path: PathBuf,
    generation: u64,
    inode: u64,
    epoch: u64,
    created: Instant,
    clone: Duration,
}

#[derive(Default)]
struct PoolState {
    /// Oldest first; acquire takes the newest.
    standbys: Vec<Standby>,
    /// Stale standbys found by acquire, disposed of by the replenisher off the caller's path.
    pending_dispose: Vec<Standby>,
    replenishing: bool,
    /// Epoch the in-flight replenish clone was started under.
    replenish_epoch: u64,
    replenish_delay: Duration,
    epoch: u64,
    shutdown: bool,
    paused: bool,
    seq: u64,
    hits: u64,
    misses: u64,
    stale_discards: u64,
    replenished: u64,
    races: u64,
    failures: u64,
    fs_events: u64,
    fs_events_ignored: u64,
    recent_fs_changes: std::collections::VecDeque<FsChange>,
    invalidations: u64,
    last_replenish: Option<Duration>,
    last_error: Option<String>,
    replenish_samples: Vec<Duration>,
}

pub(crate) struct PoolInner {
    ws: Arc<Workspace>,
    cfg: PoolConfig,
    id: u64,
    dir: PathBuf,
    state: Mutex<PoolState>,
    cond: Condvar,
    /// Tests hold the replenisher between "replenishing" and the clone to observe that state.
    #[cfg(test)]
    hold_clone: (Mutex<bool>, Condvar),
}

enum Replenish {
    Ready(Standby, Duration),
    Raced(Standby),
    Failed(StorageError, Option<Standby>),
}

impl PoolInner {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_valid(&self, epoch: u64, sb: &Standby, now: Instant) -> bool {
        sb.epoch == epoch && self.cfg.max_standby_age.map_or(true, |max| now.saturating_duration_since(sb.created) <= max)
    }

    fn ready_count(&self, st: &PoolState) -> usize {
        let now = Instant::now();
        st.standbys.iter().filter(|sb| self.is_valid(st.epoch, sb, now)).count()
    }

    /// Removes standbys invalidated by an epoch bump or by age (counted as stale); the caller
    /// disposes of them after releasing the lock.
    fn take_invalid(&self, st: &mut PoolState) -> Vec<Standby> {
        let now = Instant::now();
        let epoch = st.epoch;
        let (keep, stale): (Vec<Standby>, Vec<Standby>) =
            std::mem::take(&mut st.standbys).into_iter().partition(|sb| self.is_valid(epoch, sb, now));
        st.standbys = keep;
        st.stale_discards += stale.len() as u64;
        stale
    }

    /// Wakes the replenisher and waiters (taking the lock first so a waiter that has just checked
    /// its condition cannot miss the notification).
    pub(crate) fn wake(&self) {
        drop(self.lock());
        self.cond.notify_all();
    }

    /// Blocks while a replenish clone is in flight (bounded by `limit`).
    fn wait_while_replenishing(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        let mut st = self.lock();
        while st.replenishing && !st.shutdown {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            st = self.cond.wait_timeout(st, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Invalidates every standby (in-process commit, generation bump, explicit request).
    pub(crate) fn invalidate_all(&self) {
        let mut st = self.lock();
        st.epoch += 1;
        st.invalidations += 1;
        drop(st);
        self.cond.notify_all();
    }

    /// True if `event` is reported on the live root path itself and carries no flag that counts
    /// as a change there regardless of inodes (see module docs, "Edits that bypass this crate").
    fn is_soft_root_event(&self, event: &fsevents::Event) -> bool {
        /// Events below live were coalesced or dropped (anything may have changed), or a volume
        /// was mounted or unmounted there.
        const ROOT_HARD_CHANGE: u32 = fsevents::EVENT_MUST_SCAN_SUB_DIRS
            | fsevents::EVENT_USER_DROPPED
            | fsevents::EVENT_KERNEL_DROPPED
            | fsevents::EVENT_MOUNT
            | fsevents::EVENT_UNMOUNT;
        event.flags & ROOT_HARD_CHANGE == 0 && event.path == self.ws.live()
    }

    fn on_fs_events(&self, events: &[fsevents::Event]) {
        let mut changes = 0u64;
        let mut ignored = 0u64;
        let is_soft_root = |event: &fsevents::Event| event.flags != fsevents::EVENT_HISTORY_DONE && self.is_soft_root_event(event);
        // One lstat per batch, outside the lock, only if the batch has a soft root event. Inner
        // None: live is gone or unreadable, which counts as a change.
        let live_now = events.iter().any(is_soft_root).then(|| live_inode(self.ws.live()).ok());
        let mut st = self.lock();
        // A soft root event matters only if live was replaced under a standby that would still be
        // handed out: then its inode differs. Root replacement during a clone is caught by the
        // replenisher's inode re-check, and after that by acquire's.
        let root_replaced = match live_now {
            Some(Some(inode)) => st.standbys.iter().any(|sb| sb.epoch == st.epoch && sb.inode != inode),
            Some(None) | None => true,
        };
        for event in events {
            if event.flags == fsevents::EVENT_HISTORY_DONE {
                continue;
            }
            if is_soft_root(event) && !root_replaced {
                ignored += 1;
                continue;
            }
            changes += 1;
            if st.recent_fs_changes.len() == RECENT_FS_CHANGES {
                st.recent_fs_changes.pop_front();
            }
            st.recent_fs_changes.push_back(FsChange { path: event.path.clone(), flags: event.flags });
        }
        st.fs_events_ignored += ignored;
        if changes > 0 {
            st.fs_events += changes;
            st.epoch += 1;
            st.invalidations += 1;
        }
        drop(st);
        self.cond.notify_all();
    }

    fn matches_live(&self, sb: &Standby) -> Result<bool, StorageError> {
        Ok(self.ws.generation()? == sb.generation && live_inode(self.ws.live())? == sb.inode)
    }

    /// Detaches a standby into trash and hands it to the reaper (or deletes it inline).
    fn dispose(&self, sb: Standby) {
        let trash = self.ws.store().join("trash").join(format!("standby-{}-{}", sb.name, unique_suffix()));
        let result = match sys::rename_excl(&sb.path, &trash) {
            Ok(()) => self.ws.shared.reclaim(ReapKind::Standby, trash).map(drop),
            // Could not detach (for example the standby directory vanished): delete in place.
            Err(_) => remove_any(&sb.path),
        };
        if let Err(e) = result {
            self.lock().last_error = Some(e.to_string());
        }
    }

    fn clone_standby(&self, seq: u64, epoch: u64) -> Replenish {
        #[cfg(test)]
        {
            let (held, cond) = &self.hold_clone;
            let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
            while *held {
                held = cond.wait(held).unwrap_or_else(PoisonError::into_inner);
            }
        }
        let name = format!("{}-{}-{}", std::process::id(), self.id, seq);
        let path = self.dir.join(&name);
        let started = Instant::now();
        let before = self.ws.generation().and_then(|g| Ok((g, live_inode(self.ws.live())?)));
        let (generation, inode) = match before {
            Ok(v) => v,
            Err(e) => return Replenish::Failed(e, None),
        };
        let clone_started = Instant::now();
        if let Err(e) = sys::clone_tree(self.ws.live(), &path) {
            let err = StorageError::io("clonefile", &path, e);
            // clonefile is all-or-nothing; anything under our name would be a foreign leftover.
            let leftover = fs::symlink_metadata(&path).is_ok().then(|| Standby {
                name: name.clone(),
                path: path.clone(),
                generation,
                inode,
                epoch,
                created: clone_started,
                clone: Duration::ZERO,
            });
            return Replenish::Failed(err, leftover);
        }
        let clone = clone_started.elapsed();
        let sb = Standby { name, path, generation, inode, epoch, created: clone_started, clone };
        let after = live_inode(self.ws.live()).and_then(|i| Ok((self.ws.generation()?, i)));
        match after {
            Ok((g2, i2)) if g2 == generation && i2 == inode => Replenish::Ready(sb, started.elapsed()),
            Ok(_) => Replenish::Raced(sb),
            Err(e) => Replenish::Failed(e, Some(sb)),
        }
    }
}

fn retry_delay(failures: u32) -> Duration {
    let factor = 1u32.checked_shl(failures.saturating_sub(1).min(16)).unwrap_or(u32::MAX);
    RETRY_BASE.saturating_mul(factor).min(RETRY_CAP)
}

fn run_replenisher(inner: Arc<PoolInner>) {
    let mut failures: u32 = 0;
    let mut retry_at: Option<Instant> = None;
    loop {
        let mut st = inner.lock();
        let (evicted, job) = loop {
            if st.shutdown {
                return;
            }
            // No directory operation of ours may delay a caller's (see module docs). The guard
            // that ends the last foreground operation wakes us under this lock.
            if inner.ws.shared.foreground_active() {
                st = inner.cond.wait_timeout(st, FOREGROUND_RECHECK).unwrap_or_else(PoisonError::into_inner).0;
                continue;
            }
            let mut evicted = inner.take_invalid(&mut st);
            evicted.append(&mut st.pending_dispose);
            let now = Instant::now();
            let backing_off = retry_at.is_some_and(|t| now < t);
            let quiet_until = inner.ws.shared.last_foreground_end().map(|t| t + st.replenish_delay).filter(|t| *t > now);
            let need = !st.paused && !backing_off && st.standbys.len() < inner.cfg.capacity;
            if (need && quiet_until.is_none()) || !evicted.is_empty() {
                let job = (need && quiet_until.is_none()).then(|| {
                    st.replenishing = true;
                    st.replenish_epoch = st.epoch;
                    st.seq += 1;
                    (st.seq, st.epoch)
                });
                break (evicted, job);
            }
            // Sleep until notified, the oldest standby expires, the retry delay or the quiet
            // period ends.
            let mut wake = retry_at.filter(|t| *t > now);
            if need {
                if let Some(t) = quiet_until {
                    wake = Some(wake.map_or(t, |w| w.min(t)));
                }
            }
            if let Some(max) = inner.cfg.max_standby_age {
                if let Some(expiry) = st.standbys.iter().map(|sb| sb.created + max).min() {
                    wake = Some(wake.map_or(expiry, |w| w.min(expiry)));
                }
            }
            st = match wake {
                Some(t) => {
                    // Wake just after the deadline so the expiry check sees it as passed.
                    let timeout = t.saturating_duration_since(now) + Duration::from_millis(1);
                    inner.cond.wait_timeout(st, timeout).unwrap_or_else(PoisonError::into_inner).0
                }
                None => inner.cond.wait(st).unwrap_or_else(PoisonError::into_inner),
            };
        };
        drop(st);
        if job.is_some() {
            // Waiters on `replenishing` (acquire, wait_until) must see the start too.
            inner.cond.notify_all();
        }
        for sb in evicted {
            inner.dispose(sb);
        }
        let Some((seq, epoch)) = job else { continue };

        let outcome = inner.clone_standby(seq, epoch);
        let mut st = inner.lock();
        st.replenishing = false;
        let discard = match outcome {
            Replenish::Ready(sb, took) if !st.shutdown && sb.epoch == st.epoch => {
                st.standbys.push(sb);
                st.replenished += 1;
                st.last_replenish = Some(took);
                if st.replenish_samples.len() < MAX_SAMPLES {
                    st.replenish_samples.push(took);
                }
                failures = 0;
                retry_at = None;
                None
            }
            Replenish::Ready(sb, _) | Replenish::Raced(sb) => {
                st.races += 1;
                Some(sb)
            }
            Replenish::Failed(e, leftover) => {
                failures = failures.saturating_add(1);
                retry_at = Some(Instant::now() + retry_delay(failures));
                st.failures += 1;
                st.last_error = Some(e.to_string());
                leftover
            }
        };
        drop(st);
        inner.cond.notify_all();
        if let Some(sb) = discard {
            inner.dispose(sb);
        }
    }
}

/// Disposes of standbys whose owning process (pid prefix of the name) no longer exists.
fn reclaim_orphans(inner: &PoolInner) -> Result<usize, StorageError> {
    let me = std::process::id();
    let mut reclaimed = 0;
    for entry in fs::read_dir(&inner.dir).map_err(|e| StorageError::io("readdir", &inner.dir, e))? {
        let entry = entry.map_err(|e| StorageError::io("readdir", &inner.dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.split('-').next().and_then(|p| p.parse::<i32>().ok()) else { continue };
        if pid as u32 == me || sys::process_alive(pid) {
            continue;
        }
        let orphan = Standby {
            name: format!("orphan-{name}"),
            path: entry.path(),
            generation: 0,
            inode: 0,
            epoch: 0,
            created: Instant::now(),
            clone: Duration::ZERO,
        };
        inner.dispose(orphan);
        reclaimed += 1;
    }
    Ok(reclaimed)
}

/// Pre-cloned standby branches of one workspace. `Send + Sync`: share it with `Arc` and call
/// [`WarmBranchPool::acquire`] from any number of threads; every caller gets a distinct branch.
pub struct WarmBranchPool {
    inner: Arc<PoolInner>,
    replenisher: Option<JoinHandle<()>>,
    watcher: Option<Watcher>,
}

impl std::fmt::Debug for WarmBranchPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WarmBranchPool").field("live", &self.inner.ws.live()).field("stats", &self.stats()).finish()
    }
}

impl WarmBranchPool {
    /// Starts the FSEvents watcher (if configured) and then the replenisher, which immediately
    /// begins cloning standbys. Use [`WarmBranchPool::wait_ready`] to wait for them.
    pub fn new(workspace: Arc<Workspace>, cfg: PoolConfig) -> Result<Self, StorageError> {
        static POOL_IDS: AtomicU64 = AtomicU64::new(0);
        let dir = workspace.store().join("standby");
        fs::create_dir_all(&dir).map_err(|e| StorageError::io("mkdir", &dir, e))?;
        let inner = Arc::new(PoolInner {
            ws: Arc::clone(&workspace),
            cfg,
            id: POOL_IDS.fetch_add(1, Ordering::Relaxed),
            dir,
            state: Mutex::new(PoolState::default()),
            cond: Condvar::new(),
            #[cfg(test)]
            hold_clone: (Mutex::new(false), Condvar::new()),
        });
        reclaim_orphans(&inner)?;
        // Watch before the first clone so that no edit after that clone can go unnoticed.
        let watcher = if inner.cfg.watch_fs_events {
            let weak = Arc::downgrade(&inner);
            let watcher = Watcher::start(workspace.live(), FS_EVENT_COALESCE, move |events| {
                if let Some(inner) = weak.upgrade() {
                    inner.on_fs_events(events);
                }
            })
            .map_err(|e| StorageError::io("FSEventStreamStart", workspace.live(), e))?;
            Some(watcher)
        } else {
            None
        };
        workspace.shared.register_pool(Arc::downgrade(&inner));
        let replenisher = std::thread::Builder::new()
            .name("aegis-pool-replenisher".into())
            .spawn({
                let inner = Arc::clone(&inner);
                move || run_replenisher(inner)
            })
            .map_err(|e| StorageError::io("spawn replenisher", workspace.store(), e))?;
        Ok(Self { inner, replenisher: Some(replenisher), watcher })
    }

    /// Hands out a branch: a valid standby moved into `branches/<id>` (hit) or a cold clone
    /// (miss). Stale standbys found on the way are discarded. Safe to call concurrently.
    pub fn acquire(&self, label: &str) -> Result<(Branch, AcquireReceipt), StorageError> {
        let started = Instant::now();
        let foreground = self.inner.ws.shared.foreground();
        let result = self.acquire_inner(label, started);
        // Ending the foreground operation wakes the replenisher, after our rename or cold clone.
        drop(foreground);
        self.inner.wake();
        result
    }

    fn acquire_inner(&self, label: &str, started: Instant) -> Result<(Branch, AcquireReceipt), StorageError> {
        let inner = &*self.inner;
        let mut stale = 0usize;
        let mut waited: Option<Duration> = None;
        loop {
            let (candidate, wait_for_clone) = {
                let mut st = inner.lock();
                let mut invalid = inner.take_invalid(&mut st);
                stale += invalid.len();
                st.pending_dispose.append(&mut invalid);
                let candidate = st.standbys.pop();
                let wait = candidate.is_none() && waited.is_none() && st.replenishing && st.replenish_epoch == st.epoch;
                (candidate, wait)
            };
            let Some(sb) = candidate else {
                if wait_for_clone {
                    // A cold clone would queue behind the clone in flight (APFS serialises them).
                    let wait_started = Instant::now();
                    inner.wait_while_replenishing(REPLENISH_WAIT_LIMIT);
                    waited = Some(wait_started.elapsed());
                    continue;
                }
                break;
            };
            let fresh = match inner.matches_live(&sb) {
                Ok(fresh) => fresh,
                Err(e) => {
                    inner.lock().pending_dispose.push(sb);
                    return Err(e);
                }
            };
            if !fresh {
                stale += 1;
                let mut st = inner.lock();
                st.stale_discards += 1;
                st.pending_dispose.push(sb);
                continue;
            }
            let id = new_branch_id(label);
            let path = inner.ws.store().join("branches").join(&id);
            let rename_started = Instant::now();
            if let Err(e) = sys::rename_excl(&sb.path, &path) {
                let err = StorageError::io("renameatx_np(RENAME_EXCL)", &path, e);
                inner.lock().pending_dispose.push(sb);
                return Err(err);
            }
            let rename = rename_started.elapsed();
            let branch = inner.ws.make_branch(id, path, sb.inode, sb.generation, sb.clone, BranchOrigin::Pool);
            let mut st = inner.lock();
            if st.epoch != sb.epoch {
                // An FSEvents batch arrived while we validated and moved it: roll it back.
                st.stale_discards += 1;
                drop(st);
                stale += 1;
                drop(branch);
                continue;
            }
            st.hits += 1;
            drop(st);
            let receipt = AcquireReceipt {
                source: AcquireSource::Pool,
                total: started.elapsed(),
                rename: Some(rename),
                stale_discarded: stale,
                standby_age: Some(sb.created.elapsed()),
                waited_for_replenish: waited,
            };
            return Ok((branch, receipt));
        }
        let branch = inner.ws.branch(label)?;
        inner.lock().misses += 1;
        let receipt = AcquireReceipt {
            source: AcquireSource::Cold,
            total: started.elapsed(),
            rename: None,
            stale_discarded: stale,
            standby_age: None,
            waited_for_replenish: waited,
        };
        Ok((branch, receipt))
    }

    /// Blocks until `capacity` standbys are valid (by epoch and age) or `timeout` elapses.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.inner.lock();
        loop {
            if self.inner.ready_count(&st) >= self.inner.cfg.capacity {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            st = self.inner.cond.wait_timeout(st, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    pub fn stats(&self) -> PoolStats {
        let st = self.inner.lock();
        self.stats_locked(&st)
    }

    /// Blocks until `condition` holds for the pool's stats or `timeout` elapses. The condition is
    /// re-evaluated whenever the pool state changes (standby added or removed, invalidation,
    /// FSEvents delivery including ignored events, pause changes).
    pub fn wait_until(&self, timeout: Duration, mut condition: impl FnMut(&PoolStats) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.inner.lock();
        loop {
            if condition(&self.stats_locked(&st)) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            st = self.inner.cond.wait_timeout(st, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    fn stats_locked(&self, st: &PoolState) -> PoolStats {
        PoolStats {
            ready: self.inner.ready_count(&st),
            capacity: self.inner.cfg.capacity,
            hits: st.hits,
            misses: st.misses,
            stale_discards: st.stale_discards,
            last_replenish: st.last_replenish,
            replenishing: st.replenishing,
            replenish_delay: st.replenish_delay,
            replenished: st.replenished,
            replenish_races: st.races,
            replenish_failures: st.failures,
            fs_events: st.fs_events,
            fs_events_ignored: st.fs_events_ignored,
            recent_fs_changes: st.recent_fs_changes.iter().cloned().collect(),
            invalidations: st.invalidations,
            epoch: st.epoch,
            watching_fs_events: self.watcher.is_some(),
            paused: st.paused,
            last_error: st.last_error.clone(),
        }
    }

    /// Discards every standby now (they are re-cloned unless paused).
    pub fn invalidate(&self) {
        self.inner.invalidate_all();
    }

    /// Current invalidation epoch; see [`WarmBranchPool::wait_invalidation`].
    pub fn invalidation_epoch(&self) -> u64 {
        self.inner.lock().epoch
    }

    /// Blocks until the epoch is greater than `after` (some invalidation happened) or timeout.
    pub fn wait_invalidation(&self, after: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.inner.lock();
        loop {
            if st.epoch > after {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            st = self.inner.cond.wait_timeout(st, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Synchronously delivers every FSEvents event that occurred before this call
    /// (`FSEventStreamFlushSync`). Returns false when the pool does not watch FSEvents.
    pub fn flush_fs_events(&self) -> bool {
        match &self.watcher {
            Some(watcher) => {
                watcher.flush();
                true
            }
            None => false,
        }
    }

    /// Stops (or resumes) cloning new standbys; existing valid standbys are still handed out.
    pub fn set_replenish_paused(&self, paused: bool) {
        self.inner.lock().paused = paused;
        self.inner.cond.notify_all();
    }

    /// Postpones each replenish clone until no foreground operation (acquire, cold branch, commit,
    /// rollback detach) of this workspace has run for `delay` (default zero: replenish at once).
    /// Use it when branches are often committed or rolled back within about one clone duration of
    /// being acquired, so that their `RENAME_SWAP` or detach does not wait behind the clone.
    pub fn set_replenish_delay(&self, delay: Duration) {
        self.inner.lock().replenish_delay = delay;
        self.inner.cond.notify_all();
    }

    /// Durations of successful replenishes since the last drain (bounded buffer).
    pub fn drain_replenish_durations(&self) -> Vec<Duration> {
        std::mem::take(&mut self.inner.lock().replenish_samples)
    }

    pub fn workspace(&self) -> &Arc<Workspace> {
        &self.inner.ws
    }

    pub fn config(&self) -> &PoolConfig {
        &self.inner.cfg
    }
}

impl Drop for WarmBranchPool {
    fn drop(&mut self) {
        // Stop FSEvents first; Watcher::drop waits for a callback in flight.
        self.watcher.take();
        self.inner.lock().shutdown = true;
        self.inner.cond.notify_all();
        if let Some(handle) = self.replenisher.take() {
            // A panicked replenisher has nothing left to clean up beyond the standbys below.
            let _ = handle.join();
        }
        let standbys = {
            let mut st = self.inner.lock();
            let mut all = std::mem::take(&mut st.standbys);
            all.append(&mut st.pending_dispose);
            all
        };
        for sb in standbys {
            self.inner.dispose(sb);
        }
    }
}

#[allow(dead_code)]
fn assert_thread_safety() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<WarmBranchPool>();
    send_sync::<Workspace>();
    send_sync::<Branch>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{snapshot, Fixture, WAIT};
    use std::sync::Barrier;

    fn quiet() -> PoolConfig {
        PoolConfig { capacity: 1, max_standby_age: None, watch_fs_events: false }
    }

    fn pool(fx: &Fixture, cfg: PoolConfig) -> (Arc<Workspace>, WarmBranchPool) {
        let ws = Arc::new(fx.workspace());
        let pool = WarmBranchPool::new(Arc::clone(&ws), cfg).unwrap();
        (ws, pool)
    }

    fn hold_clones(pool: &WarmBranchPool, held: bool) {
        let (flag, cond) = &pool.inner.hold_clone;
        *flag.lock().unwrap() = held;
        cond.notify_all();
    }

    fn standby_count(ws: &Workspace) -> usize {
        fs::read_dir(ws.store().join("standby")).unwrap().count()
    }

    /// Events for writes made shortly before the stream started (creating the fixture) can still
    /// be delivered after it started, and one write can be reported in several batches (create,
    /// then modify/xattr). FSEvents delivers in event-id order, so: write a sentinel, rename it, and
    /// wait for the rename's event; every event older than the rename has then been delivered.
    fn settle(pool: &WarmBranchPool, ws: &Workspace) {
        let suffix = unique_suffix();
        let written = ws.live().join(format!(".settle-{suffix}"));
        let renamed = ws.live().join(format!(".settled-{suffix}"));
        fs::write(&written, "settle\n").unwrap();
        fs::rename(&written, &renamed).unwrap();
        assert!(
            pool.wait_until(WAIT, |s| s.recent_fs_changes.iter().any(|c| c.path == renamed)),
            "sentinel rename not delivered: {:?}",
            pool.stats()
        );
        assert!(pool.wait_ready(WAIT));
    }

    #[test]
    fn pool_hit_hands_out_an_identical_copy() {
        let fx = Fixture::new("pool-hit");
        let (ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        assert_eq!(standby_count(&ws), 1);
        let (branch, receipt) = pool.acquire("hit").unwrap();
        assert_eq!(receipt.source, AcquireSource::Pool);
        assert!(receipt.rename.is_some());
        assert!(receipt.standby_age.is_some());
        assert_eq!(receipt.stale_discarded, 0);
        assert_eq!(branch.origin(), BranchOrigin::Pool);
        assert!(branch.path().starts_with(ws.store().join("branches")));
        assert!(branch.clone_duration() > Duration::ZERO);
        assert_eq!(snapshot(branch.path()), fx.snapshot());
        assert!(branch.changes().unwrap().is_empty());
        assert_eq!(branch.base_generation(), ws.generation().unwrap());
        // The pool refills itself.
        assert!(pool.wait_ready(WAIT));
        let stats = pool.stats();
        assert_eq!((stats.hits, stats.misses, stats.stale_discards), (1, 0, 0));
        assert!(stats.replenished >= 2 && stats.last_replenish.is_some());
        assert_eq!(pool.drain_replenish_durations().len() as u64, stats.replenished);
        drop(branch);
    }

    #[test]
    fn pool_miss_when_empty_falls_back_to_cold() {
        let fx = Fixture::new("pool-miss");
        let (_ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        pool.set_replenish_paused(true);
        let (first, r1) = pool.acquire("one").unwrap();
        let (second, r2) = pool.acquire("two").unwrap();
        assert_eq!(r1.source, AcquireSource::Pool);
        assert_eq!(r2.source, AcquireSource::Cold);
        assert_eq!(r2.rename, None);
        assert_eq!(second.origin(), BranchOrigin::Cold);
        assert_ne!(first.path(), second.path());
        assert_eq!(snapshot(second.path()), fx.snapshot());
        let stats = pool.stats();
        assert_eq!((stats.hits, stats.misses, stats.ready), (1, 1, 0));
        assert!(stats.paused);
        pool.set_replenish_paused(false);
        assert!(pool.wait_ready(WAIT));
    }

    #[test]
    fn capacity_zero_is_always_cold() {
        let fx = Fixture::new("pool-zero");
        let (ws, pool) = pool(&fx, PoolConfig { capacity: 0, ..quiet() });
        assert!(pool.wait_ready(Duration::ZERO));
        let (_b, receipt) = pool.acquire("z").unwrap();
        assert_eq!(receipt.source, AcquireSource::Cold);
        assert_eq!(standby_count(&ws), 0);
    }

    #[test]
    fn stale_after_in_process_commit_is_discarded() {
        let fx = Fixture::new("pool-stale-local");
        let (ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        let writer = ws.branch("writer").unwrap();
        fs::write(writer.path().join("README.md"), "# committed\n").unwrap();
        writer.commit().unwrap();
        let (branch, _receipt) = pool.acquire("after-commit").unwrap();
        assert_eq!(fs::read_to_string(branch.path().join("README.md")).unwrap(), "# committed\n");
        assert_eq!(snapshot(branch.path()), fx.snapshot());
        assert!(pool.stats().stale_discards >= 1, "{:?}", pool.stats());
        // Its base is the committed live tree, so it commits without a conflict.
        fs::write(branch.path().join("next.txt"), "next\n").unwrap();
        branch.commit().unwrap();
        assert_eq!(fs::read_to_string(ws.live().join("next.txt")).unwrap(), "next\n");
    }

    #[test]
    fn stale_after_commit_by_another_instance_is_discarded() {
        let fx = Fixture::new("pool-stale-remote");
        let (_ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        // Paused so that acquire cannot pick up a standby re-cloned after the discard (which would
        // be a correct hit); this test pins down the discard-then-cold path.
        pool.set_replenish_paused(true);
        // A second Workspace on the same store shares only the on-disk state, like another process.
        let other = fx.workspace();
        let writer = other.branch("remote").unwrap();
        fs::write(writer.path().join("README.md"), "# remote\n").unwrap();
        writer.commit().unwrap();
        let (branch, receipt) = pool.acquire("after-remote").unwrap();
        assert_eq!(receipt.source, AcquireSource::Cold);
        assert_eq!(receipt.stale_discarded, 1);
        assert_eq!(fs::read_to_string(branch.path().join("README.md")).unwrap(), "# remote\n");
        let stats = pool.stats();
        assert_eq!((stats.misses, stats.stale_discards, stats.hits), (1, 1, 0));
        fs::write(branch.path().join("x.txt"), "x").unwrap();
        branch.commit().unwrap();

        // A generation bump alone (no content change, same inode) also invalidates.
        pool.set_replenish_paused(false);
        assert!(pool.wait_ready(WAIT));
        pool.set_replenish_paused(true);
        other.bump_generation().unwrap();
        let (bumped, receipt) = pool.acquire("after-bump").unwrap();
        assert_eq!(receipt.source, AcquireSource::Cold);
        assert_eq!(receipt.stale_discarded, 1);
        assert_eq!(bumped.base_generation(), other.generation().unwrap());
    }

    #[test]
    fn external_edit_invalidates_standbys_via_fsevents() {
        let fx = Fixture::new("pool-fsevents");
        let (ws, pool) = pool(&fx, PoolConfig { capacity: 1, max_standby_age: None, watch_fs_events: true });
        assert!(pool.stats().watching_fs_events);
        assert!(pool.wait_ready(WAIT));
        settle(&pool, &ws);
        let before = pool.invalidation_epoch();
        let fs_events_before = pool.stats().fs_events;
        fs::write(ws.live().join("README.md"), "# edited outside\n").unwrap();
        let written = Instant::now();
        assert!(pool.wait_invalidation(before, WAIT), "no FSEvents invalidation within {WAIT:?}");
        let delivered = written.elapsed();
        eprintln!("fsevents: write -> invalidation in {:.3} ms (preliminary, shared machine)", delivered.as_secs_f64() * 1e3);
        let stats = pool.stats();
        assert!(stats.fs_events > fs_events_before);
        assert!(stats.recent_fs_changes.iter().any(|c| c.path == ws.live().join("README.md")), "{stats:?}");
        assert!(pool.flush_fs_events());
        assert!(pool.wait_ready(WAIT));
        let (branch, _receipt) = pool.acquire("after-edit").unwrap();
        assert_eq!(fs::read_to_string(branch.path().join("README.md")).unwrap(), "# edited outside\n");
    }

    fn watching() -> PoolConfig {
        PoolConfig { capacity: 1, max_standby_age: None, watch_fs_events: true }
    }

    /// Flags (hex) of counted change events reported on the live root path itself.
    fn root_changes(stats: &PoolStats, ws: &Workspace) -> Vec<String> {
        stats.recent_fs_changes.iter().filter(|c| c.path == ws.live()).map(|c| format!("{:#x}", c.flags)).collect()
    }

    /// Acquire (rename in the store), replenish (clone of live), rollback and reaping must not look
    /// like external edits. Runs three acquire/rollback rounds. Each replenish clone of live is
    /// reported by FSEvents on the live root; every round waits for that notification (FlushSync
    /// is not a barrier for events the kernel has not handed to fseventsd yet) and checks that it
    /// was ignored: the epoch, the counted events and the replenish races must not move.
    fn assert_own_activity_is_ignored(pool: &WarmBranchPool, ws: &Workspace, context: &str) {
        let before = pool.stats();
        for round in 0..3 {
            let seen = pool.stats().fs_events_ignored;
            let (branch, receipt) = pool.acquire("self").unwrap();
            assert_eq!(receipt.source, AcquireSource::Pool, "{context}, round {round}: {receipt:?} {:?}", pool.stats());
            branch.rollback().unwrap();
            assert!(pool.wait_ready(WAIT));
            assert!(
                pool.wait_until(WAIT, |s| s.fs_events_ignored > seen || s.epoch != before.epoch),
                "{context}, round {round}: no FSEvents notification for the replenish clone: {:?}",
                pool.stats()
            );
            let now = pool.stats();
            assert_eq!(
                now.epoch,
                before.epoch,
                "{context}, round {round}: epoch moved; counted root events {:?}; {now:?}",
                root_changes(&now, ws)
            );
            assert_eq!(now.replenish_races, before.replenish_races, "{context}, round {round}: {now:?}");
        }
        let after = pool.stats();
        assert_eq!(after.fs_events, before.fs_events, "{context}: {after:?}");
        assert_eq!(after.hits, before.hits + 3, "{context}: {after:?}");
    }

    #[test]
    fn own_activity_does_not_trigger_fsevents_invalidation() {
        let fx = Fixture::new("pool-quiet-self");
        let (ws, pool) = pool(&fx, watching());
        assert!(pool.wait_ready(WAIT));
        settle(&pool, &ws);
        assert_own_activity_is_ignored(&pool, &ws, "before any commit");
        assert!(ws.wait_reaped(WAIT));
    }

    #[test]
    fn own_commits_do_not_trigger_fsevents_invalidation() {
        // The daemon's configuration: FSEvents on and branches committed. After a RENAME_SWAP of
        // live, FSEvents attaches the swap's ItemRenamed bit to every later clone event on the
        // live root path, so each refill used to invalidate the standby it had just produced.
        let fx = Fixture::new("pool-quiet-commit");
        let (ws, pool) = pool(&fx, watching());
        assert!(pool.wait_ready(WAIT));
        settle(&pool, &ws);
        let (branch, _) = pool.acquire("commit").unwrap();
        fs::write(branch.path().join("README.md"), "# committed\n").unwrap();
        let first = branch.commit().unwrap();
        // restore + commit swaps live again, with a tree cloned from versions/.
        ws.restore(first.version).unwrap().commit().unwrap();
        assert_eq!(fs::read_to_string(ws.live().join("README.md")).unwrap(), "# demo\n");
        // Every event of both swaps has been delivered once the sentinel's has.
        settle(&pool, &ws);
        assert_own_activity_is_ignored(&pool, &ws, "after commit and restore+commit");

        // A real external write after the swaps still invalidates (FSEvents watches the path, so
        // the stream follows the tree that is live now).
        let epoch = pool.invalidation_epoch();
        fs::write(ws.live().join("README.md"), "# edited after commits\n").unwrap();
        assert!(pool.wait_invalidation(epoch, WAIT), "no FSEvents invalidation after commits: {:?}", pool.stats());
        let stats = pool.stats();
        assert!(stats.recent_fs_changes.iter().any(|c| c.path == ws.live().join("README.md")), "{stats:?}");
        assert!(pool.flush_fs_events());
        assert!(pool.wait_ready(WAIT));
        let (branch, _) = pool.acquire("after-edit").unwrap();
        assert_eq!(fs::read_to_string(branch.path().join("README.md")).unwrap(), "# edited after commits\n");
        drop(branch);
        assert!(ws.wait_reaped(WAIT));
    }

    #[test]
    fn swap_events_of_an_own_commit_do_not_invalidate() {
        // The commit's own RootChanged / ItemRenamed events on the live root arrive after
        // notify_live_changed has already invalidated the pool; counting them again would throw
        // away the refill that is being cloned from the new tree.
        let fx = Fixture::new("pool-swap-events");
        let (ws, pool) = pool(&fx, watching());
        assert!(pool.wait_ready(WAIT));
        settle(&pool, &ws);
        // Paused across acquire and commit, so that no standby of the pre-commit tree exists when
        // the swap's events are delivered, in whichever order they race with the commit.
        pool.set_replenish_paused(true);
        let (branch, _) = pool.acquire("commit").unwrap();
        fs::write(branch.path().join("README.md"), "# committed\n").unwrap();
        let before = pool.stats();
        branch.commit().unwrap();
        assert_eq!(pool.stats().epoch, before.epoch + 1, "the in-process commit invalidates once");
        pool.set_replenish_paused(false);
        settle(&pool, &ws);
        let after = pool.stats();
        assert!(root_changes(&after, &ws).is_empty(), "the commit's own root events were counted: {after:?}");
        let (branch, _) = pool.acquire("after-commit").unwrap();
        assert_eq!(fs::read_to_string(branch.path().join("README.md")).unwrap(), "# committed\n");
        assert_eq!(branch.base_generation(), ws.generation().unwrap());
    }

    #[test]
    fn commit_by_another_instance_invalidates_through_the_live_root_event() {
        let fx = Fixture::new("pool-foreign-swap");
        let (ws, pool) = pool(&fx, watching());
        assert!(pool.wait_ready(WAIT));
        settle(&pool, &ws);
        // A second Workspace on the same store shares only the on-disk state, like another
        // process: only the FSEvents report of its RENAME_SWAP on the live root reaches this pool.
        let other = fx.workspace();
        let writer = other.branch("remote").unwrap();
        fs::write(writer.path().join("README.md"), "# remote\n").unwrap();
        let epoch = pool.invalidation_epoch();
        writer.commit().unwrap();
        assert!(pool.wait_invalidation(epoch, WAIT), "foreign swap not seen: {:?}", pool.stats());
        let stats = pool.stats();
        assert!(!root_changes(&stats, &ws).is_empty(), "{stats:?}");
        assert!(pool.wait_ready(WAIT));
        let (branch, _) = pool.acquire("after-remote").unwrap();
        assert_eq!(fs::read_to_string(branch.path().join("README.md")).unwrap(), "# remote\n");
        assert_eq!(branch.base_generation(), other.generation().unwrap());
        drop(branch);
        settle(&pool, &ws);
        assert_own_activity_is_ignored(&pool, &ws, "after a foreign commit");
    }

    #[test]
    fn metadata_change_on_the_live_root_does_not_start_a_reclone_loop() {
        let fx = Fixture::new("pool-touch-root");
        let (ws, pool) = pool(&fx, watching());
        assert!(pool.wait_ready(WAIT));
        settle(&pool, &ws);
        // `touch <live>` and a chmod to the same mode: only the root directory's own times and
        // ctime change (ItemInodeMetaMod on the root path).
        let now = std::time::SystemTime::now();
        fs::File::open(ws.live()).unwrap().set_times(fs::FileTimes::new().set_accessed(now).set_modified(now)).unwrap();
        let perms = fs::metadata(ws.live()).unwrap().permissions();
        fs::set_permissions(ws.live(), perms).unwrap();
        settle(&pool, &ws);
        let stats = pool.stats();
        assert!(root_changes(&stats, &ws).is_empty(), "root metadata events were counted: {stats:?}");
        assert_own_activity_is_ignored(&pool, &ws, "after touching the live root");
        assert!(ws.wait_reaped(WAIT));
    }

    #[test]
    fn stale_standby_is_replaced_by_a_fresh_hit_when_replenisher_is_quick() {
        let fx = Fixture::new("pool-stale-then-hit");
        let (_ws, pool) = pool(&fx, PoolConfig { capacity: 2, ..quiet() });
        assert!(pool.wait_ready(WAIT));
        let other = fx.workspace();
        let writer = other.branch("remote").unwrap();
        fs::write(writer.path().join("README.md"), "# remote\n").unwrap();
        writer.commit().unwrap();
        // Whatever the timing, acquire never returns a pre-commit tree: either it discards both
        // stale standbys and goes cold, or it picks up a standby re-cloned after the commit.
        let (branch, receipt) = pool.acquire("after-remote").unwrap();
        assert_eq!(fs::read_to_string(branch.path().join("README.md")).unwrap(), "# remote\n");
        assert_eq!(branch.base_generation(), other.generation().unwrap());
        assert!(receipt.stale_discarded >= 1, "{receipt:?}");
        fs::write(branch.path().join("y.txt"), "y").unwrap();
        branch.commit().unwrap();
    }

    #[test]
    fn external_edit_without_watcher_needs_bump_generation() {
        let fx = Fixture::new("pool-nowatch");
        let (ws, pool) = pool(&fx, PoolConfig { capacity: 2, max_standby_age: None, watch_fs_events: false });
        assert!(pool.wait_ready(WAIT));
        pool.set_replenish_paused(true);
        fs::write(ws.live().join("README.md"), "# direct edit\n").unwrap();
        // Documented limitation: without FSEvents (or a bump) a direct edit is not seen.
        let (stale, receipt) = pool.acquire("unaware").unwrap();
        assert_eq!(receipt.source, AcquireSource::Pool);
        assert_eq!(fs::read_to_string(stale.path().join("README.md")).unwrap(), "# demo\n");
        drop(stale);
        // bump_generation is the explicit remedy.
        ws.bump_generation().unwrap();
        let (fresh, receipt) = pool.acquire("aware").unwrap();
        assert_eq!(receipt.source, AcquireSource::Cold);
        // The remaining pre-edit standby was discarded, by the replenisher or by this acquire.
        assert_eq!(pool.stats().stale_discards, 1);
        assert_eq!(fs::read_to_string(fresh.path().join("README.md")).unwrap(), "# direct edit\n");
    }

    #[test]
    fn max_standby_age_expires_standbys() {
        let fx = Fixture::new("pool-age");
        let age = Duration::from_millis(100);
        let (_ws, pool) = pool(&fx, PoolConfig { capacity: 1, max_standby_age: Some(age), watch_fs_events: false });
        assert!(pool.wait_ready(WAIT));
        pool.set_replenish_paused(true);
        // Letting time pass is the condition under test, not synchronisation.
        std::thread::sleep(age * 2);
        assert_eq!(pool.stats().ready, 0);
        let (_b, receipt) = pool.acquire("old").unwrap();
        assert_eq!(receipt.source, AcquireSource::Cold);
        assert_eq!(pool.stats().stale_discards, 1);
        pool.set_replenish_paused(false);
        assert!(pool.wait_ready(WAIT));
    }

    #[test]
    fn concurrent_acquire_hands_out_distinct_correct_branches() {
        const THREADS: usize = 8;
        let fx = Fixture::new("pool-concurrent");
        let before = fx.snapshot();
        let (ws, pool) = pool(&fx, PoolConfig { capacity: 3, max_standby_age: None, watch_fs_events: false });
        assert!(pool.wait_ready(WAIT));
        let pool = Arc::new(pool);
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let (pool, barrier, expected) = (Arc::clone(&pool), Arc::clone(&barrier), before.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let (branch, receipt) = pool.acquire(&format!("agent-{i}")).unwrap();
                    assert_eq!(snapshot(branch.path()), expected, "branch {i} must equal live");
                    fs::write(branch.path().join(format!("agent-{i}.txt")), format!("{i}\n")).unwrap();
                    (branch, receipt)
                })
            })
            .collect();
        let results: Vec<(Branch, AcquireReceipt)> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let mut paths: Vec<PathBuf> = results.iter().map(|(b, _)| b.path().to_path_buf()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), THREADS, "every acquire must get its own directory");
        for (i, (branch, _)) in results.iter().enumerate() {
            let files = snapshot(branch.path());
            assert_eq!(files.len(), before.len() + 1, "branch {i} sees only its own edit");
            assert!(files.contains_key(&format!("agent-{i}.txt")));
        }
        let hits = results.iter().filter(|(_, r)| r.source == AcquireSource::Pool).count();
        let stats = pool.stats();
        assert!(hits >= 1);
        assert_eq!(stats.hits as usize, hits);
        assert_eq!((stats.hits + stats.misses) as usize, THREADS);
        assert_eq!(fx.snapshot(), before, "live untouched while branches are open");

        // All branches share a base: the first commit wins, the rest conflict.
        let mut results = results.into_iter();
        let (winner, _) = results.next().unwrap();
        let winner_file = winner.path().join("agent-0.txt");
        assert!(winner_file.exists());
        winner.commit().unwrap();
        for (loser, _) in results {
            assert!(matches!(loser.commit(), Err(StorageError::Conflict { .. })));
        }
        assert_eq!(fs::read_to_string(ws.live().join("agent-0.txt")).unwrap(), "0\n");
        drop(pool);
        assert!(ws.wait_reaped(WAIT));
        assert_eq!(fs::read_dir(ws.store().join("branches")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
    }

    #[test]
    fn pooled_commit_swaps_and_pooled_rollback_leaves_live_identical() {
        let fx = Fixture::new("pool-commit");
        let (ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        let (branch, receipt) = pool.acquire("commit").unwrap();
        assert_eq!(receipt.source, AcquireSource::Pool);
        fs::write(branch.path().join("src/main.rs"), "fn main() { println!(\"pooled\"); }\n").unwrap();
        let changes = branch.changes().unwrap();
        assert_eq!(changes.count(crate::ChangeKind::Modified), 1);
        let gen = ws.generation().unwrap();
        let commit = branch.commit().unwrap();
        assert_eq!(commit.generation, gen + 1);
        assert!(fs::read_to_string(ws.live().join("src/main.rs")).unwrap().contains("pooled"));
        let versions = ws.versions().unwrap();
        assert_eq!(versions.last().unwrap().number, commit.version);
        assert!(fs::read_to_string(versions.last().unwrap().path.join("src/main.rs")).unwrap().contains("hi"));
        assert_eq!(fs::read_dir(ws.store().join("branches")).unwrap().count(), 0);

        assert!(pool.wait_ready(WAIT));
        let before = fx.snapshot();
        let (branch, receipt) = pool.acquire("rollback").unwrap();
        assert_eq!(receipt.source, AcquireSource::Pool);
        assert_eq!(snapshot(branch.path()), before, "standby re-cloned from the committed tree");
        fs::write(branch.path().join("src/main.rs"), "broken").unwrap();
        fs::write(branch.path().join("junk.txt"), "junk").unwrap();
        let rollback = branch.rollback().unwrap();
        assert!(rollback.queued);
        assert_eq!(fx.snapshot(), before);
        assert!(ws.wait_reaped(WAIT));
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
    }

    #[test]
    fn dropping_the_pool_leaves_no_standby_behind() {
        let fx = Fixture::new("pool-drop");
        let (ws, pool) = pool(&fx, PoolConfig { capacity: 3, ..PoolConfig::default() });
        assert!(pool.wait_ready(WAIT));
        assert_eq!(standby_count(&ws), 3);
        drop(pool);
        assert_eq!(standby_count(&ws), 0);
        assert!(ws.wait_reaped(WAIT));
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
        assert!(ws.drain_reap_records().iter().all(|r| r.kind == ReapKind::Standby && r.ok));
    }

    #[test]
    fn orphaned_standbys_of_dead_processes_are_reclaimed() {
        let fx = Fixture::new("pool-orphan");
        let ws = Arc::new(fx.workspace());
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let dead = child.id() as i32;
        child.wait().unwrap();
        assert!(!sys::process_alive(dead));
        let orphan = ws.store().join("standby").join(format!("{dead}-0-1"));
        fs::create_dir_all(orphan.join("src")).unwrap();
        fs::write(orphan.join("src/main.rs"), "old").unwrap();
        let unrelated = ws.store().join("standby").join("not-a-standby");
        fs::create_dir_all(&unrelated).unwrap();
        let pool = WarmBranchPool::new(Arc::clone(&ws), quiet()).unwrap();
        assert!(!orphan.exists());
        assert!(unrelated.exists(), "names without a pid prefix are left alone");
        assert!(pool.wait_ready(WAIT));
        drop(pool);
        fs::remove_dir(&unrelated).unwrap();
        assert!(ws.wait_reaped(WAIT));
        assert_eq!(standby_count(&ws), 0);
    }

    #[test]
    fn acquire_waits_for_in_flight_replenish_instead_of_cloning_cold() {
        let fx = Fixture::new("pool-wait-inflight");
        let (ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        hold_clones(&pool, true);
        // Invalidate: the replenisher discards the standby and starts a clone for the new epoch,
        // which the test holds just before clonefile.
        pool.invalidate();
        assert!(pool.wait_until(WAIT, |s| s.replenishing && s.ready == 0), "{:?}", pool.stats());
        let pool = Arc::new(pool);
        let acquirer = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || pool.acquire("waiter").unwrap())
        };
        // The acquirer must be parked on the in-flight clone, not cloning cold.
        assert!(!pool.wait_until(Duration::from_millis(200), |s| s.misses > 0 || s.hits > 0));
        hold_clones(&pool, false);
        let (branch, receipt) = acquirer.join().unwrap();
        assert_eq!(receipt.source, AcquireSource::Pool, "{receipt:?}");
        assert!(receipt.waited_for_replenish.is_some());
        assert!(receipt.total >= receipt.waited_for_replenish.unwrap());
        assert_eq!(snapshot(branch.path()), fx.snapshot());
        assert_eq!(branch.base_generation(), ws.generation().unwrap());
    }

    #[test]
    fn replenisher_yields_to_foreground_operations() {
        let fx = Fixture::new("pool-foreground");
        let (ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        let replenished = pool.stats().replenished;
        {
            let _foreground = ws.shared.foreground();
            pool.invalidate();
            // No clone may start while a foreground directory operation is running.
            assert!(!pool.wait_until(Duration::from_millis(200), |s| s.replenishing || s.replenished > replenished));
        }
        assert!(pool.wait_ready(WAIT));
        assert!(pool.stats().replenished > replenished);
    }

    #[test]
    fn replenish_delay_postpones_the_clone_after_an_acquire() {
        let fx = Fixture::new("pool-delay");
        let (_ws, pool) = pool(&fx, quiet());
        assert!(pool.wait_ready(WAIT));
        let delay = Duration::from_millis(300);
        pool.set_replenish_delay(delay);
        assert_eq!(pool.stats().replenish_delay, delay);
        let replenished = pool.stats().replenished;
        let (branch, receipt) = pool.acquire("short-lived").unwrap();
        let acquired = Instant::now();
        assert_eq!(receipt.source, AcquireSource::Pool);
        // A rollback right after acquire does not meet a replenish clone in flight.
        branch.rollback().unwrap();
        assert!(!pool.wait_until(delay / 3, |s| s.replenishing || s.replenished > replenished));
        assert!(pool.wait_ready(WAIT));
        assert!(acquired.elapsed() >= delay);
        pool.set_replenish_delay(Duration::ZERO);
    }
}
