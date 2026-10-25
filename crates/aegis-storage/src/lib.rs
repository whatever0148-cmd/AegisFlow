//! Transactional copy-on-write workspace versioning on APFS.
//!
//! * [`Workspace::branch`] clones the live tree with one directory-level `clonefile(2)`
//!   (`CLONE_NOFOLLOW | CLONE_NOOWNERCOPY`): metadata is copied, data blocks are shared until
//!   written. There is no userspace walk; the per-entry cost of that single call is inside the
//!   kernel (APFS creates a clone record per inode), see `examples/clone_scaling.rs`.
//! * [`WarmBranchPool`] hides that cost from callers: it keeps pre-cloned standby copies of the
//!   live tree and hands one out with a single no-clobber rename. Standbys are never handed out
//!   stale with respect to commits (see the generation rules below and the `pool` module docs).
//! * [`Branch::commit`] exchanges branch and live trees with `renameatx_np(RENAME_SWAP)` — one
//!   syscall, so readers see either the old or the new tree. The previous live tree is kept as a
//!   numbered version snapshot that [`Workspace::restore`] can bring back.
//! * Dropping a [`Branch`] without committing rolls it back: the branch is renamed into the store's
//!   trash (detach, O(1)) and, by default, a background reaper thread deletes it (reclaim,
//!   O(entries)) so the caller returns right after the detach. [`Workspace::set_background_reclaim`]
//!   switches back to deleting on the caller's thread. Version pruning after a commit uses the same
//!   path. The live tree is never touched by rollback.
//!
//! On APFS a directory `clonefile` in flight blocks every directory rename and clone on the same
//! volume until it finishes (file writes, creation and `stat` are unaffected; tree deletion blocks
//! nothing), measured by `examples/apfs_contention.rs`. The pool and the workspace therefore treat
//! cold clones, acquires, commits and rollback detaches as foreground operations that background
//! replenishing yields to (see the `pool` module docs for what this can and cannot avoid).
//!
//! Branch, version, standby and trash directories live in a store next to the live tree
//! (`<parent>/.aegis-<name>/`) because both syscalls require a single APFS volume.
//! Commits are serialised with an `flock` on the store. Every commit made through this crate (and
//! every [`Workspace::bump_generation`]) increments a generation number persisted in
//! `<store>/generation` under that lock, so other processes see it too. A branch records the live
//! directory's inode and the generation it was based on; a commit on top of a newer tree (inode or
//! generation changed) is rejected as a conflict. Edits made directly to the live tree (outside
//! branches) are only noticed by a pool that watches FSEvents, or after `bump_generation`.

mod diff;
mod fsevents;
mod pool;
mod reaper;
pub mod sys;
#[cfg(test)]
mod test_support;

pub use diff::{diff_trees, ChangeKind, ChangeSet, FileChange};
pub use pool::{AcquireReceipt, AcquireSource, FsChange, PoolConfig, PoolStats, WarmBranchPool, DEFAULT_MAX_STANDBY_AGE};
pub use reaper::{ReapKind, ReapRecord, ReaperStats};

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reaper::Reaper;

/// Number of committed versions kept unless configured otherwise.
pub const DEFAULT_RETAINED_VERSIONS: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("{path}: workspaces must live on APFS (statfs reports `{found}`)")]
    NotApfs { path: PathBuf, found: String },
    #[error("{0}: not a directory")]
    NotADirectory(PathBuf),
    #[error("store {store} is not on the same volume as {live}")]
    CrossDevice { live: PathBuf, store: PathBuf },
    #[error("commit conflict: the live workspace changed after branch {branch} was created; the branch was rolled back")]
    Conflict { branch: String },
    #[error("version {0} does not exist")]
    UnknownVersion(u64),
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl StorageError {
    fn io(op: &'static str, path: &Path, source: io::Error) -> Self {
        StorageError::Io { op, path: path.to_path_buf(), source }
    }
}

/// Result of a successful commit.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CommitReceipt {
    pub branch: String,
    /// Version number under which the previous live tree was stored.
    pub version: u64,
    /// Time spent inside `renameatx_np(RENAME_SWAP)`.
    pub swap: Duration,
    /// Lock + conflict check + swap + version and generation bookkeeping (excludes pruning).
    pub total: Duration,
    /// Old versions removed to respect the retention limit.
    pub pruned_versions: usize,
    /// True when pruned versions were handed to the background reaper rather than deleted inline.
    pub prune_queued: bool,
    /// Live generation after this commit.
    pub generation: u64,
}

/// Result of a rollback.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RollbackReceipt {
    pub branch: String,
    /// Time to make the branch unreachable (rename into the store's trash).
    pub detach: Duration,
    /// Time spent deleting the detached tree on the caller's thread. This is `Duration::ZERO`
    /// when `queued` is true, because deletion was handed to the background reaper; the reaper's
    /// own timing is available from [`Workspace::drain_reap_records`] (match `reap_ticket`).
    pub reclaim: Duration,
    /// True when deletion was queued to the background reaper.
    pub queued: bool,
    /// Reaper ticket of the queued deletion.
    pub reap_ticket: Option<u64>,
}

impl RollbackReceipt {
    /// Reclaim time if it happened on the caller's thread, `None` if it was queued.
    pub fn reclaim_inline(&self) -> Option<Duration> {
        (!self.queued).then_some(self.reclaim)
    }
}

/// A committed snapshot of an earlier live tree.
#[derive(Debug, Clone)]
pub struct Version {
    pub number: u64,
    pub path: PathBuf,
    pub created: SystemTime,
}

/// How a [`Branch`] was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum BranchOrigin {
    /// Cloned on the caller's thread by [`Workspace::branch`] or [`Workspace::restore`].
    Cold,
    /// Handed out by a [`WarmBranchPool`] from a pre-cloned standby.
    Pool,
}

/// Generation counter persisted in `<store>/generation` as 20 zero-padded decimal digits plus a
/// newline. It is always rewritten in place with one 21-byte `pwrite` at offset 0 while holding the
/// store lock, and read with one `pread`; POSIX makes those atomic with respect to each other, so a
/// reader in any process sees either the old or the new value.
#[derive(Debug)]
struct GenerationFile {
    path: PathBuf,
    file: File,
}

impl GenerationFile {
    fn open(store: &Path) -> Result<Self, StorageError> {
        let path = store.join("generation");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| StorageError::io("open", &path, e))?;
        Ok(Self { path, file })
    }

    fn read(&self) -> Result<u64, StorageError> {
        let mut buf = [0u8; 32];
        let n = self.file.read_at(&mut buf, 0).map_err(|e| StorageError::io("pread generation", &self.path, e))?;
        let text = std::str::from_utf8(&buf[..n]).unwrap_or("").trim();
        if text.is_empty() {
            return Ok(0);
        }
        text.parse::<u64>().map_err(|e| {
            StorageError::io("parse generation", &self.path, io::Error::new(io::ErrorKind::InvalidData, e))
        })
    }

    /// Caller must hold the store lock.
    fn write(&self, value: u64) -> Result<(), StorageError> {
        let text = format!("{value:020}\n");
        self.file
            .write_all_at(text.as_bytes(), 0)
            .map_err(|e| StorageError::io("pwrite generation", &self.path, e))
    }
}

/// State shared by a workspace, its clones, its branches and its pools.
pub(crate) struct Shared {
    generation: GenerationFile,
    reaper: Reaper,
    background_reclaim: AtomicBool,
    pools: Mutex<Vec<Weak<pool::PoolInner>>>,
    /// Caller-facing directory operations (cold clone, acquire, commit, rollback detach) in
    /// progress. On APFS a directory `clonefile` in flight blocks every directory rename and clone
    /// on the volume (see `examples/apfs_contention.rs`), so pools start no replenish clone and
    /// no disposal rename while this is non-zero.
    foreground: AtomicUsize,
    /// Nanoseconds after `created` at which the last foreground operation ended (0 = never).
    last_foreground_end: AtomicU64,
    created: Instant,
}

/// Marks a foreground directory operation; dropping it wakes the pools once none is left.
pub(crate) struct ForegroundGuard<'a> {
    shared: &'a Shared,
}

impl Drop for ForegroundGuard<'_> {
    fn drop(&mut self) {
        let ended = self.shared.created.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.shared.last_foreground_end.fetch_max(ended.max(1), Ordering::AcqRel);
        if self.shared.foreground.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.wake_pools();
        }
    }
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("generation", &self.generation.path)
            .field("background_reclaim", &self.background_reclaim.load(Ordering::Relaxed))
            .field("reaper", &self.reaper)
            .finish()
    }
}

/// Outcome of handing a detached tree to [`Shared::reclaim`].
enum Reclaim {
    Queued(u64),
    Done(Duration),
}

impl Shared {
    pub(crate) fn foreground(&self) -> ForegroundGuard<'_> {
        self.foreground.fetch_add(1, Ordering::AcqRel);
        ForegroundGuard { shared: self }
    }

    pub(crate) fn foreground_active(&self) -> bool {
        self.foreground.load(Ordering::Acquire) > 0
    }

    pub(crate) fn last_foreground_end(&self) -> Option<Instant> {
        match self.last_foreground_end.load(Ordering::Acquire) {
            0 => None,
            nanos => Some(self.created + Duration::from_nanos(nanos)),
        }
    }

    fn live_pools(&self) -> Vec<Arc<pool::PoolInner>> {
        let mut pools = self.pools.lock().unwrap_or_else(PoisonError::into_inner);
        pools.retain(|p| p.strong_count() > 0);
        pools.iter().filter_map(Weak::upgrade).collect()
    }

    fn wake_pools(&self) {
        for pool in self.live_pools() {
            pool.wake();
        }
    }

    /// Deletes `path` (already inside `trash/`) on the reaper thread in background mode, or inline.
    fn reclaim(&self, kind: ReapKind, path: PathBuf) -> Result<Reclaim, StorageError> {
        if self.background_reclaim.load(Ordering::Acquire) {
            // On a spawn failure the reaper records the error and we fall back to inline deletion.
            if let Ok(ticket) = self.reaper.enqueue(kind, path.clone()) {
                return Ok(Reclaim::Queued(ticket));
            }
        }
        let started = Instant::now();
        remove_any(&path)?;
        Ok(Reclaim::Done(started.elapsed()))
    }

    fn register_pool(&self, pool: Weak<pool::PoolInner>) {
        let mut pools = self.pools.lock().unwrap_or_else(PoisonError::into_inner);
        pools.retain(|p| p.strong_count() > 0);
        pools.push(pool);
    }

    /// Tells every pool of this workspace that live changed (after a commit or generation bump).
    fn notify_live_changed(&self) {
        for pool in self.live_pools() {
            pool.invalidate_all();
        }
    }
}

/// A live directory tree plus its branch/version store.
///
/// `Workspace` is `Send + Sync`; clones share the reaper, the generation file handle and the
/// background-reclaim setting, but each clone has its own retention setting.
#[derive(Debug, Clone)]
pub struct Workspace {
    live: PathBuf,
    store: PathBuf,
    retained_versions: usize,
    shared: Arc<Shared>,
}

impl Workspace {
    /// Opens `live` with the default store `<parent>/.aegis-<name>/`.
    pub fn open(live: impl AsRef<Path>) -> Result<Self, StorageError> {
        let live = fs::canonicalize(live.as_ref()).map_err(|e| StorageError::io("canonicalize", live.as_ref(), e))?;
        let name = live.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "root".into());
        let parent = live.parent().ok_or_else(|| StorageError::NotADirectory(live.clone()))?;
        let store = parent.join(format!(".aegis-{name}"));
        Self::open_with_store(&live, &store)
    }

    /// Opens `live` with an explicit store directory, which must be on the same APFS volume.
    pub fn open_with_store(live: &Path, store: &Path) -> Result<Self, StorageError> {
        let live = fs::canonicalize(live).map_err(|e| StorageError::io("canonicalize", live, e))?;
        let meta = fs::metadata(&live).map_err(|e| StorageError::io("stat", &live, e))?;
        if !meta.is_dir() {
            return Err(StorageError::NotADirectory(live));
        }
        let fs_name = sys::fs_type(&live).map_err(|e| StorageError::io("statfs", &live, e))?;
        if fs_name != "apfs" {
            return Err(StorageError::NotApfs { path: live, found: fs_name });
        }
        for sub in ["branches", "versions", "trash"] {
            let dir = store.join(sub);
            fs::create_dir_all(&dir).map_err(|e| StorageError::io("mkdir", &dir, e))?;
        }
        let store = fs::canonicalize(store).map_err(|e| StorageError::io("canonicalize", store, e))?;
        let store_dev = fs::metadata(&store).map_err(|e| StorageError::io("stat", &store, e))?.dev();
        if store_dev != meta.dev() {
            return Err(StorageError::CrossDevice { live, store });
        }
        let shared = Arc::new(Shared {
            generation: GenerationFile::open(&store)?,
            reaper: Reaper::new(),
            background_reclaim: AtomicBool::new(true),
            pools: Mutex::new(Vec::new()),
            foreground: AtomicUsize::new(0),
            last_foreground_end: AtomicU64::new(0),
            created: Instant::now(),
        });
        Ok(Self { live, store, retained_versions: DEFAULT_RETAINED_VERSIONS, shared })
    }

    pub fn live(&self) -> &Path {
        &self.live
    }

    pub fn store(&self) -> &Path {
        &self.store
    }

    pub fn set_retained_versions(&mut self, versions: usize) {
        self.retained_versions = versions.max(1);
    }

    /// Background reclaim (the default) makes rollback, version pruning and pool disposal return
    /// right after the O(1) detach and leaves deletion to the reaper thread. `false` deletes on the
    /// caller's thread, as phase 1 did. Applies to every clone of this workspace and to its
    /// existing branches and pools.
    pub fn set_background_reclaim(&self, enabled: bool) {
        self.shared.background_reclaim.store(enabled, Ordering::Release);
    }

    pub fn background_reclaim(&self) -> bool {
        self.shared.background_reclaim.load(Ordering::Acquire)
    }

    /// Reaper counters (queued includes the job in progress).
    pub fn reaper_stats(&self) -> ReaperStats {
        self.shared.reaper.stats(self.background_reclaim())
    }

    /// Finished reclaim jobs since the last drain, oldest first (bounded buffer).
    pub fn drain_reap_records(&self) -> Vec<ReapRecord> {
        self.shared.reaper.drain_records()
    }

    /// Deletion time of every finished reclaim job since the last drain. Shares its buffer with
    /// [`Workspace::drain_reap_records`]: either call drains both.
    pub fn drain_reap_durations(&self) -> Vec<Duration> {
        self.drain_reap_records().into_iter().map(|r| r.duration).collect()
    }

    /// Blocks until the reaper has nothing queued or in progress; false on timeout.
    pub fn wait_reaped(&self, timeout: Duration) -> bool {
        self.shared.reaper.wait_idle(timeout)
    }

    /// Current live generation (read without the store lock).
    pub fn generation(&self) -> Result<u64, StorageError> {
        self.shared.generation.read()
    }

    /// Declares that the live tree changed outside this crate (for example a direct edit). Bumps
    /// the persisted generation under the store lock, which makes every pool (in any process)
    /// discard its standbys and makes branches based on the previous generation conflict at commit.
    pub fn bump_generation(&self) -> Result<u64, StorageError> {
        let lock = self.lock()?;
        let next = self.shared.generation.read()? + 1;
        self.shared.generation.write(next)?;
        drop(lock);
        self.shared.notify_live_changed();
        Ok(next)
    }

    fn lock(&self) -> Result<sys::FileLock, StorageError> {
        let lock_path = self.store.join("lock");
        sys::FileLock::exclusive(&lock_path).map_err(|e| StorageError::io("flock", &lock_path, e))
    }

    /// Creates a copy-on-write branch of the live tree.
    pub fn branch(&self, label: &str) -> Result<Branch, StorageError> {
        let id = new_branch_id(label);
        let path = self.store.join("branches").join(&id);
        let _foreground = self.shared.foreground();
        let _lock = self.lock()?;
        let base_inode = live_inode(&self.live)?;
        let base_generation = self.shared.generation.read()?;
        let started = Instant::now();
        sys::clone_tree(&self.live, &path).map_err(|e| StorageError::io("clonefile", &path, e))?;
        let clone = started.elapsed();
        Ok(self.make_branch(id, path, base_inode, base_generation, clone, BranchOrigin::Cold))
    }

    pub(crate) fn make_branch(
        &self,
        id: String,
        path: PathBuf,
        base_inode: u64,
        base_generation: u64,
        clone: Duration,
        origin: BranchOrigin,
    ) -> Branch {
        Branch {
            id,
            path,
            live: self.live.clone(),
            store: self.store.clone(),
            base_inode,
            base_generation,
            retained_versions: self.retained_versions,
            created: Instant::now(),
            clone,
            origin,
            finished: false,
            shared: Arc::clone(&self.shared),
        }
    }

    /// Committed versions, oldest first.
    pub fn versions(&self) -> Result<Vec<Version>, StorageError> {
        list_versions(&self.store)
    }

    /// Clones version `number` into a new branch; commit the branch to make it live again.
    pub fn restore(&self, number: u64) -> Result<Branch, StorageError> {
        let version = list_versions(&self.store)?
            .into_iter()
            .find(|v| v.number == number)
            .ok_or(StorageError::UnknownVersion(number))?;
        let id = new_branch_id(&format!("restore-v{number}"));
        let path = self.store.join("branches").join(&id);
        let _foreground = self.shared.foreground();
        let _lock = self.lock()?;
        let base_inode = live_inode(&self.live)?;
        let base_generation = self.shared.generation.read()?;
        let started = Instant::now();
        sys::clone_tree(&version.path, &path).map_err(|e| StorageError::io("clonefile", &path, e))?;
        let clone = started.elapsed();
        Ok(self.make_branch(id, path, base_inode, base_generation, clone, BranchOrigin::Cold))
    }

    /// Deletes anything left in the trash (for example after a crash during reclaim). Entries the
    /// background reaper of this workspace is about to delete are skipped and not counted.
    pub fn reclaim_trash(&self) -> Result<usize, StorageError> {
        let trash = self.store.join("trash");
        let pending = self.shared.reaper.pending_names();
        let mut removed = 0;
        for entry in fs::read_dir(&trash).map_err(|e| StorageError::io("readdir", &trash, e))? {
            let entry = entry.map_err(|e| StorageError::io("readdir", &trash, e))?;
            if pending.contains(&entry.file_name()) {
                continue;
            }
            remove_any(&entry.path())?;
            removed += 1;
        }
        Ok(removed)
    }
}

/// An isolated, copy-on-write view of the workspace. Dropping it without [`Branch::commit`]
/// rolls it back. Pooled and cold branches behave identically.
#[derive(Debug)]
pub struct Branch {
    id: String,
    path: PathBuf,
    live: PathBuf,
    store: PathBuf,
    base_inode: u64,
    base_generation: u64,
    retained_versions: usize,
    created: Instant,
    clone: Duration,
    origin: BranchOrigin,
    finished: bool,
    shared: Arc<Shared>,
}

impl Branch {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Directory the agent should work in.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn live(&self) -> &Path {
        &self.live
    }

    /// Time spent inside `clonefile(2)` when this tree was cloned. For a pooled branch the clone
    /// ran earlier on the pool's replenisher thread, not on the caller's thread.
    pub fn clone_duration(&self) -> Duration {
        self.clone
    }

    /// Time since the branch was handed to the caller.
    pub fn age(&self) -> Duration {
        self.created.elapsed()
    }

    pub fn origin(&self) -> BranchOrigin {
        self.origin
    }

    /// Live generation this branch is based on (checked again at commit).
    pub fn base_generation(&self) -> u64 {
        self.base_generation
    }

    /// Files that differ between the live tree and this branch.
    pub fn changes(&self) -> Result<ChangeSet, StorageError> {
        diff_trees(&self.live, &self.path).map_err(|e| StorageError::io("diff", &self.path, e))
    }

    /// Atomically makes this branch the live tree. On a conflict the branch is rolled back.
    ///
    /// An error returned after the swap (version bookkeeping or the generation write failing)
    /// means live already holds this branch's tree.
    pub fn commit(mut self) -> Result<CommitReceipt, StorageError> {
        let shared = Arc::clone(&self.shared);
        let foreground = shared.foreground();
        let started = Instant::now();
        let lock_path = self.store.join("lock");
        let lock = sys::FileLock::exclusive(&lock_path).map_err(|e| StorageError::io("flock", &lock_path, e))?;
        let generation = self.shared.generation.read()?;
        if live_inode(&self.live)? != self.base_inode || generation != self.base_generation {
            drop(lock);
            return Err(StorageError::Conflict { branch: self.id.clone() });
        }
        let swap_started = Instant::now();
        sys::swap(&self.path, &self.live).map_err(|e| StorageError::io("renameatx_np(RENAME_SWAP)", &self.live, e))?;
        let swap = swap_started.elapsed();
        // From here on `self.path` holds the previous live tree, so Drop must not delete it.
        self.finished = true;
        let bookkeeping = self.record_version(generation + 1);
        let total = started.elapsed();
        let pruned = match &bookkeeping {
            Ok(_) => detach_excess_versions(&self.store, self.retained_versions),
            Err(_) => Ok(Vec::new()),
        };
        drop(lock);
        // Pools learn about the new tree even if bookkeeping failed after the swap; ending the
        // foreground operation then lets them re-clone (pruned trees are deleted after that).
        self.shared.notify_live_changed();
        drop(foreground);
        let number = bookkeeping?;
        let detached = pruned?;
        let pruned_versions = detached.len();
        let mut prune_queued = false;
        for path in detached {
            prune_queued |= matches!(self.shared.reclaim(ReapKind::Version, path)?, Reclaim::Queued(_));
        }
        Ok(CommitReceipt {
            branch: self.id.clone(),
            version: number,
            swap,
            total,
            pruned_versions,
            prune_queued,
            generation: generation + 1,
        })
    }

    /// Moves the previous live tree (now at `self.path`) into `versions/` and publishes the new
    /// generation. Caller holds the store lock.
    fn record_version(&self, generation: u64) -> Result<u64, StorageError> {
        let number = next_version_number(&self.store)?;
        let version_path = self.store.join("versions").join(format!("{number:010}"));
        sys::rename_excl(&self.path, &version_path)
            .map_err(|e| StorageError::io("renameatx_np(RENAME_EXCL)", &version_path, e))?;
        self.shared.generation.write(generation)?;
        Ok(number)
    }

    /// Discards the branch. In background mode (the default) this returns right after the detach
    /// and the receipt reports the reclaim as queued.
    pub fn rollback(mut self) -> Result<RollbackReceipt, StorageError> {
        self.discard()
    }

    fn discard(&mut self) -> Result<RollbackReceipt, StorageError> {
        self.finished = true;
        let trash = self.store.join("trash").join(&self.id);
        let foreground = self.shared.foreground();
        let started = Instant::now();
        let detached = sys::rename_excl(&self.path, &trash);
        let detach = started.elapsed();
        drop(foreground);
        detached.map_err(|e| StorageError::io("renameatx_np(RENAME_EXCL)", &trash, e))?;
        Ok(match self.shared.reclaim(ReapKind::Branch, trash)? {
            Reclaim::Queued(ticket) => RollbackReceipt {
                branch: self.id.clone(),
                detach,
                reclaim: Duration::ZERO,
                queued: true,
                reap_ticket: Some(ticket),
            },
            Reclaim::Done(reclaim) => {
                RollbackReceipt { branch: self.id.clone(), detach, reclaim, queued: false, reap_ticket: None }
            }
        })
    }
}

impl Drop for Branch {
    fn drop(&mut self) {
        if !self.finished {
            // Best effort: a failure leaves the tree in branches/ or trash/ for reclaim_trash().
            let _ = self.discard();
        }
    }
}

pub(crate) fn live_inode(live: &Path) -> Result<u64, StorageError> {
    Ok(fs::symlink_metadata(live).map_err(|e| StorageError::io("stat", live, e))?.ino())
}

/// Process-unique suffix for names created in the store.
pub(crate) fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{}-{:x}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed), nanos & 0xffff_ffff)
}

pub(crate) fn new_branch_id(label: &str) -> String {
    let clean: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(40)
        .collect();
    format!("{}-{}", if clean.is_empty() { "branch" } else { clean.as_str() }, unique_suffix())
}

fn list_versions(store: &Path) -> Result<Vec<Version>, StorageError> {
    let dir = store.join("versions");
    let mut versions = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| StorageError::io("readdir", &dir, e))? {
        let entry = entry.map_err(|e| StorageError::io("readdir", &dir, e))?;
        let Ok(number) = entry.file_name().to_string_lossy().parse::<u64>() else { continue };
        let created = entry.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        versions.push(Version { number, path: entry.path(), created });
    }
    versions.sort_by_key(|v| v.number);
    Ok(versions)
}

fn next_version_number(store: &Path) -> Result<u64, StorageError> {
    Ok(list_versions(store)?.last().map(|v| v.number + 1).unwrap_or(1))
}

/// Renames versions beyond the retention limit into `trash/` and returns their new paths.
/// Caller holds the store lock, so concurrent committers never prune the same version.
fn detach_excess_versions(store: &Path, keep: usize) -> Result<Vec<PathBuf>, StorageError> {
    let versions = list_versions(store)?;
    let excess = versions.len().saturating_sub(keep);
    let mut detached = Vec::with_capacity(excess);
    for version in versions.iter().take(excess) {
        let trash = store.join("trash").join(format!("version-{}-{}", version.number, unique_suffix()));
        sys::rename_excl(&version.path, &trash).map_err(|e| StorageError::io("renameatx_np(RENAME_EXCL)", &trash, e))?;
        detached.push(trash);
    }
    Ok(detached)
}

/// Removes a file or directory tree. A tree that disappears underneath us (removed concurrently
/// by another process's reaper or `reclaim_trash`) counts as removed.
pub(crate) fn remove_any(path: &Path) -> Result<(), StorageError> {
    const ATTEMPTS: usize = 3;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(StorageError::io("stat", path, e)),
        };
        let result = if meta.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
        match result {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound && attempt < ATTEMPTS => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound && fs::symlink_metadata(path).is_err() => return Ok(()),
            Err(e) => return Err(StorageError::io("remove", path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Fixture, WAIT};
    use std::collections::BTreeMap;

    #[test]
    fn workspace_is_on_apfs() {
        let fx = Fixture::new("apfs");
        assert_eq!(sys::fs_type(&fx.root).unwrap(), "apfs");
        let ws = fx.workspace();
        assert!(ws.store().ends_with(".aegis-live"));
    }

    #[test]
    fn branch_is_isolated_copy() {
        let fx = Fixture::new("isolated");
        let ws = fx.workspace();
        let branch = ws.branch("agent").unwrap();
        assert_eq!(fs::read(branch.path().join("src/main.rs")).unwrap(), fs::read(ws.live().join("src/main.rs")).unwrap());
        fs::write(branch.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        assert!(fs::read_to_string(ws.live().join("src/main.rs")).unwrap().contains("println"));
        assert!(branch.clone_duration() > Duration::ZERO);
        assert_eq!(branch.origin(), BranchOrigin::Cold);
    }

    #[test]
    fn dropped_branch_rolls_back_and_leaves_live_identical() {
        let fx = Fixture::new("drop");
        let before = fx.snapshot();
        let ws = fx.workspace();
        let path;
        {
            let branch = ws.branch("doomed").unwrap();
            path = branch.path().to_path_buf();
            fs::write(branch.path().join("src/main.rs"), "broken").unwrap();
            fs::write(branch.path().join("new.txt"), "x").unwrap();
        }
        assert!(!path.exists(), "branch directory must be gone after drop");
        assert_eq!(fx.snapshot(), before);
        assert!(ws.wait_reaped(WAIT));
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
    }

    #[test]
    fn explicit_rollback_reports_timings() {
        let fx = Fixture::new("rollback");
        let ws = fx.workspace();
        let branch = ws.branch("rb").unwrap();
        let receipt = branch.rollback().unwrap();
        assert!(receipt.detach > Duration::ZERO);
        assert!(receipt.queued, "background reclaim is the default");
        assert_eq!(fs::read_dir(ws.store().join("branches")).unwrap().count(), 0);
        assert!(ws.wait_reaped(WAIT));
        let records = ws.drain_reap_records();
        assert_eq!(records.len(), 1);
        assert_eq!(Some(records[0].ticket), receipt.reap_ticket);
        assert!(records[0].ok && records[0].duration > Duration::ZERO);
    }

    #[test]
    fn synchronous_mode_reclaims_inline() {
        let fx = Fixture::new("sync-reclaim");
        let ws = fx.workspace();
        ws.set_background_reclaim(false);
        let receipt = ws.branch("rb").unwrap().rollback().unwrap();
        assert!(!receipt.queued);
        assert_eq!(receipt.reap_ticket, None);
        assert!(receipt.reclaim > Duration::ZERO);
        assert_eq!(receipt.reclaim_inline(), Some(receipt.reclaim));
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
        assert_eq!(ws.reaper_stats().reaped, 0);
    }

    #[test]
    fn rollback_returns_before_reclaim_in_background_mode() {
        let fx = Fixture::new("bg-reclaim");
        let ws = fx.workspace();
        // Hold the reaper so "returned before deletion" is observed deterministically.
        ws.shared.reaper.set_paused(true);
        let receipt = ws.branch("rb").unwrap().rollback().unwrap();
        assert!(receipt.queued);
        assert_eq!(receipt.reclaim, Duration::ZERO);
        assert_eq!(receipt.reclaim_inline(), None);
        let detached = ws.store().join("trash").join(&receipt.branch);
        assert!(detached.join("src/main.rs").exists(), "tree must still be in trash while the reaper is held");
        assert_eq!(ws.reaper_stats().queued, 1);
        assert!(!ws.wait_reaped(Duration::from_millis(50)), "nothing can be reaped while paused");
        // reclaim_trash must not race the reaper for entries it owns.
        assert_eq!(ws.reclaim_trash().unwrap(), 0);
        ws.shared.reaper.set_paused(false);
        assert!(ws.wait_reaped(WAIT));
        assert!(!detached.exists());
        let stats = ws.reaper_stats();
        assert_eq!((stats.queued, stats.reaped, stats.failed), (0, 1, 0));
        assert!(stats.last_reap.is_some());
        assert_eq!(ws.drain_reap_durations().len(), 1);
        assert!(ws.drain_reap_durations().is_empty(), "drain empties the buffer");
    }

    #[test]
    fn reaper_eventually_empties_trash() {
        let fx = Fixture::new("reaper");
        let mut ws = fx.workspace();
        ws.set_retained_versions(1);
        for i in 0..6 {
            let b = ws.branch("rb").unwrap();
            fs::write(b.path().join(format!("f{i}.txt")), "x").unwrap();
            if i % 2 == 0 {
                b.rollback().unwrap();
            } else {
                drop(b);
            }
        }
        for i in 0..3 {
            let b = ws.branch("c").unwrap();
            fs::write(b.path().join("README.md"), format!("# {i}\n")).unwrap();
            let receipt = b.commit().unwrap();
            assert_eq!(receipt.pruned_versions, usize::from(i > 0));
            assert_eq!(receipt.prune_queued, i > 0);
        }
        assert!(ws.wait_reaped(WAIT));
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
        let stats = ws.reaper_stats();
        assert_eq!((stats.reaped, stats.failed), (8, 0));
        let kinds: Vec<ReapKind> = ws.drain_reap_records().iter().map(|r| r.kind).collect();
        assert_eq!(kinds.iter().filter(|k| **k == ReapKind::Branch).count(), 6);
        assert_eq!(kinds.iter().filter(|k| **k == ReapKind::Version).count(), 2);
    }

    #[test]
    fn reclaim_trash_removes_crash_leftovers() {
        let fx = Fixture::new("leftovers");
        let ws = fx.workspace();
        let leftover = ws.store().join("trash/crashed-branch");
        fs::create_dir_all(leftover.join("deep/dir")).unwrap();
        fs::write(leftover.join("deep/dir/file"), "x").unwrap();
        assert_eq!(ws.reclaim_trash().unwrap(), 1);
        assert!(!leftover.exists());
    }

    #[test]
    fn commit_swaps_atomically_and_keeps_previous_version() {
        let fx = Fixture::new("commit");
        let ws = fx.workspace();
        let gen_before = ws.generation().unwrap();
        let branch = ws.branch("feature").unwrap();
        fs::write(branch.path().join("src/main.rs"), "fn main() { println!(\"v2\"); }\n").unwrap();
        let receipt = branch.commit().unwrap();
        assert_eq!(receipt.version, 1);
        assert_eq!(receipt.generation, gen_before + 1);
        assert_eq!(ws.generation().unwrap(), gen_before + 1);
        assert!(fs::read_to_string(ws.live().join("src/main.rs")).unwrap().contains("v2"));
        let versions = ws.versions().unwrap();
        assert_eq!(versions.len(), 1);
        assert!(fs::read_to_string(versions[0].path.join("src/main.rs")).unwrap().contains("hi"));
        assert_eq!(fs::read_dir(ws.store().join("branches")).unwrap().count(), 0);
    }

    #[test]
    fn generation_is_shared_between_instances() {
        let fx = Fixture::new("generation");
        let a = fx.workspace();
        let b = fx.workspace();
        let start = a.generation().unwrap();
        let branch = b.branch("x").unwrap();
        branch.commit().unwrap();
        assert_eq!(a.generation().unwrap(), start + 1);
        assert_eq!(a.bump_generation().unwrap(), start + 2);
        assert_eq!(b.generation().unwrap(), start + 2);
        let raw = fs::read_to_string(a.store().join("generation")).unwrap();
        assert_eq!(raw.trim().parse::<u64>().unwrap(), start + 2);
    }

    #[test]
    fn restore_brings_back_an_earlier_version() {
        let fx = Fixture::new("restore");
        let ws = fx.workspace();
        let branch = ws.branch("change").unwrap();
        fs::write(branch.path().join("README.md"), "# changed\n").unwrap();
        let receipt = branch.commit().unwrap();
        ws.restore(receipt.version).unwrap().commit().unwrap();
        assert_eq!(fs::read_to_string(ws.live().join("README.md")).unwrap(), "# demo\n");
    }

    #[test]
    fn stale_branch_commit_is_a_conflict() {
        let fx = Fixture::new("conflict");
        let ws = fx.workspace();
        let first = ws.branch("first").unwrap();
        let second = ws.branch("second").unwrap();
        fs::write(first.path().join("README.md"), "# first\n").unwrap();
        fs::write(second.path().join("README.md"), "# second\n").unwrap();
        first.commit().unwrap();
        let err = second.commit().unwrap_err();
        assert!(matches!(err, StorageError::Conflict { .. }), "{err}");
        assert_eq!(fs::read_to_string(ws.live().join("README.md")).unwrap(), "# first\n");
    }

    #[test]
    fn bump_generation_makes_open_branch_conflict() {
        let fx = Fixture::new("bump-conflict");
        let ws = fx.workspace();
        let branch = ws.branch("open").unwrap();
        fs::write(branch.path().join("README.md"), "# branch\n").unwrap();
        ws.bump_generation().unwrap();
        let err = branch.commit().unwrap_err();
        assert!(matches!(err, StorageError::Conflict { .. }), "{err}");
        assert_eq!(fs::read_to_string(ws.live().join("README.md")).unwrap(), "# demo\n");
    }

    #[test]
    fn retention_prunes_old_versions() {
        let fx = Fixture::new("retain");
        let mut ws = fx.workspace();
        ws.set_retained_versions(2);
        for i in 0..4 {
            let b = ws.branch("v").unwrap();
            fs::write(b.path().join("README.md"), format!("# {i}\n")).unwrap();
            b.commit().unwrap();
        }
        let numbers: Vec<u64> = ws.versions().unwrap().iter().map(|v| v.number).collect();
        assert_eq!(numbers, vec![3, 4]);
        assert!(ws.wait_reaped(WAIT));
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
    }

    #[test]
    fn rename_excl_never_clobbers() {
        let fx = Fixture::new("excl");
        let a = fx.root.join("a");
        let b = fx.root.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let err = sys::rename_excl(&a, &b).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EEXIST));
        assert!(a.exists() && b.exists());
        sys::rename_excl(&a, &fx.root.join("c")).unwrap();
        assert!(!a.exists() && fx.root.join("c").exists());
    }

    #[test]
    fn changes_report_added_modified_deleted() {
        let fx = Fixture::new("changes");
        let ws = fx.workspace();
        let branch = ws.branch("diff").unwrap();
        fs::write(branch.path().join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n    run();\n}\n").unwrap();
        fs::write(branch.path().join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        fs::remove_file(branch.path().join("README.md")).unwrap();
        let changes = branch.changes().unwrap();
        let kinds: BTreeMap<String, ChangeKind> =
            changes.files.iter().map(|f| (f.path.display().to_string(), f.kind)).collect();
        assert_eq!(kinds.get("src/main.rs"), Some(&ChangeKind::Modified));
        assert_eq!(kinds.get("src/lib.rs"), Some(&ChangeKind::Added));
        assert_eq!(kinds.get("README.md"), Some(&ChangeKind::Deleted));
        let main = changes.files.iter().find(|f| f.path.ends_with("main.rs")).unwrap();
        assert_eq!((main.lines_added, main.lines_removed), (1, 0));
        assert!(main.added_text.contains("run();"));
    }
}
