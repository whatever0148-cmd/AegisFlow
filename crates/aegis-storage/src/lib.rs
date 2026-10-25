//! Transactional copy-on-write workspace versioning on APFS.
//!
//! * [`Workspace::branch`] clones the live tree with `clonefile(2)`
//!   (`CLONE_NOFOLLOW | CLONE_NOOWNERCOPY`): metadata is copied, data blocks are shared until written.
//! * [`Branch::commit`] exchanges branch and live trees with `renameatx_np(RENAME_SWAP)` — one
//!   syscall, so readers see either the old or the new tree. The previous live tree is kept as a
//!   numbered version snapshot that [`Workspace::restore`] can bring back.
//! * Dropping a [`Branch`] without committing rolls it back: the branch is renamed into the store's
//!   trash (detach, O(1)) and then deleted (reclaim, O(entries)). The live tree is never touched.
//!
//! Branch, version and trash directories live in a store next to the live tree
//! (`<parent>/.aegis-<name>/`) because both syscalls require a single APFS volume.
//! Commits are serialised with an `flock` on the store, and a branch records the live
//! directory's inode so a commit on top of a newer tree is rejected as a conflict.
//! Edits made directly to the live tree (outside branches) are not detected.

mod diff;
mod sys;

pub use diff::{diff_trees, ChangeKind, ChangeSet, FileChange};

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// Lock + conflict check + swap + version bookkeeping (excludes pruning).
    pub total: Duration,
    /// Old versions deleted to respect the retention limit.
    pub pruned_versions: usize,
}

/// Result of a rollback.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RollbackReceipt {
    pub branch: String,
    /// Time to make the branch unreachable (rename into the store's trash).
    pub detach: Duration,
    /// Time to delete the detached tree.
    pub reclaim: Duration,
}

/// A committed snapshot of an earlier live tree.
#[derive(Debug, Clone)]
pub struct Version {
    pub number: u64,
    pub path: PathBuf,
    pub created: SystemTime,
}

/// A live directory tree plus its branch/version store.
#[derive(Debug, Clone)]
pub struct Workspace {
    live: PathBuf,
    store: PathBuf,
    retained_versions: usize,
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
        Ok(Self { live, store, retained_versions: DEFAULT_RETAINED_VERSIONS })
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

    /// Creates a copy-on-write branch of the live tree.
    pub fn branch(&self, label: &str) -> Result<Branch, StorageError> {
        let id = new_branch_id(label);
        let path = self.store.join("branches").join(&id);
        let lock_path = self.store.join("lock");
        let _lock = sys::FileLock::exclusive(&lock_path).map_err(|e| StorageError::io("flock", &lock_path, e))?;
        let base_inode = live_inode(&self.live)?;
        let started = Instant::now();
        sys::clone_tree(&self.live, &path).map_err(|e| StorageError::io("clonefile", &path, e))?;
        let clone = started.elapsed();
        Ok(Branch {
            id,
            path,
            live: self.live.clone(),
            store: self.store.clone(),
            base_inode,
            retained_versions: self.retained_versions,
            created: Instant::now(),
            clone,
            finished: false,
        })
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
        let lock_path = self.store.join("lock");
        let _lock = sys::FileLock::exclusive(&lock_path).map_err(|e| StorageError::io("flock", &lock_path, e))?;
        let base_inode = live_inode(&self.live)?;
        let started = Instant::now();
        sys::clone_tree(&version.path, &path).map_err(|e| StorageError::io("clonefile", &path, e))?;
        Ok(Branch {
            id,
            path,
            live: self.live.clone(),
            store: self.store.clone(),
            base_inode,
            retained_versions: self.retained_versions,
            created: Instant::now(),
            clone: started.elapsed(),
            finished: false,
        })
    }

    /// Deletes anything left in the trash (for example after a crash during reclaim).
    pub fn reclaim_trash(&self) -> Result<usize, StorageError> {
        let trash = self.store.join("trash");
        let mut removed = 0;
        for entry in fs::read_dir(&trash).map_err(|e| StorageError::io("readdir", &trash, e))? {
            let path = entry.map_err(|e| StorageError::io("readdir", &trash, e))?.path();
            remove_any(&path)?;
            removed += 1;
        }
        Ok(removed)
    }
}

/// An isolated, copy-on-write view of the workspace. Dropping it without [`Branch::commit`]
/// rolls it back.
#[derive(Debug)]
pub struct Branch {
    id: String,
    path: PathBuf,
    live: PathBuf,
    store: PathBuf,
    base_inode: u64,
    retained_versions: usize,
    created: Instant,
    clone: Duration,
    finished: bool,
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

    /// Time spent inside `clonefile(2)` when the branch was created.
    pub fn clone_duration(&self) -> Duration {
        self.clone
    }

    pub fn age(&self) -> Duration {
        self.created.elapsed()
    }

    /// Files that differ between the live tree and this branch.
    pub fn changes(&self) -> Result<ChangeSet, StorageError> {
        diff_trees(&self.live, &self.path).map_err(|e| StorageError::io("diff", &self.path, e))
    }

    /// Atomically makes this branch the live tree. On a conflict the branch is rolled back.
    pub fn commit(mut self) -> Result<CommitReceipt, StorageError> {
        let started = Instant::now();
        let lock_path = self.store.join("lock");
        let lock = sys::FileLock::exclusive(&lock_path).map_err(|e| StorageError::io("flock", &lock_path, e))?;
        if live_inode(&self.live)? != self.base_inode {
            drop(lock);
            return Err(StorageError::Conflict { branch: self.id.clone() });
        }
        let swap_started = Instant::now();
        sys::swap(&self.path, &self.live).map_err(|e| StorageError::io("renameatx_np(RENAME_SWAP)", &self.live, e))?;
        let swap = swap_started.elapsed();
        // From here on `self.path` holds the previous live tree, so Drop must not delete it.
        self.finished = true;
        let number = next_version_number(&self.store)?;
        let version_path = self.store.join("versions").join(format!("{number:010}"));
        fs::rename(&self.path, &version_path).map_err(|e| StorageError::io("rename", &version_path, e))?;
        let total = started.elapsed();
        drop(lock);
        let pruned_versions = prune_versions(&self.store, self.retained_versions)?;
        Ok(CommitReceipt { branch: self.id.clone(), version: number, swap, total, pruned_versions })
    }

    /// Discards the branch and reports how long detaching and reclaiming took.
    pub fn rollback(mut self) -> Result<RollbackReceipt, StorageError> {
        self.discard()
    }

    fn discard(&mut self) -> Result<RollbackReceipt, StorageError> {
        self.finished = true;
        let trash = self.store.join("trash").join(&self.id);
        let started = Instant::now();
        fs::rename(&self.path, &trash).map_err(|e| StorageError::io("rename", &trash, e))?;
        let detach = started.elapsed();
        let reclaim_started = Instant::now();
        remove_any(&trash)?;
        Ok(RollbackReceipt { branch: self.id.clone(), detach, reclaim: reclaim_started.elapsed() })
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

fn live_inode(live: &Path) -> Result<u64, StorageError> {
    Ok(fs::symlink_metadata(live).map_err(|e| StorageError::io("stat", live, e))?.ino())
}

fn new_branch_id(label: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let clean: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(40)
        .collect();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!(
        "{}-{}-{}-{:x}",
        if clean.is_empty() { "branch" } else { clean.as_str() },
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        nanos & 0xffff_ffff
    )
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

fn prune_versions(store: &Path, keep: usize) -> Result<usize, StorageError> {
    let versions = list_versions(store)?;
    let excess = versions.len().saturating_sub(keep);
    for version in versions.iter().take(excess) {
        let trash = store.join("trash").join(format!("version-{}", version.number));
        fs::rename(&version.path, &trash).map_err(|e| StorageError::io("rename", &trash, e))?;
        remove_any(&trash)?;
    }
    Ok(excess)
}

fn remove_any(path: &Path) -> Result<(), StorageError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(StorageError::io("stat", path, e)),
    };
    let result = if meta.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
    result.map_err(|e| StorageError::io("remove", path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/test-fixtures")
                .join(format!("{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("live/src")).unwrap();
            fs::write(root.join("live/src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
            fs::write(root.join("live/README.md"), "# demo\n").unwrap();
            Fixture { root }
        }

        fn workspace(&self) -> Workspace {
            Workspace::open(self.root.join("live")).unwrap()
        }

        fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
            let mut out = BTreeMap::new();
            collect(&self.root.join("live"), &self.root.join("live"), &mut out);
            out
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn collect(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(base, &path, out);
            } else {
                out.insert(path.strip_prefix(base).unwrap().display().to_string(), fs::read(&path).unwrap());
            }
        }
    }

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
        assert_eq!(fs::read_dir(ws.store().join("trash")).unwrap().count(), 0);
    }

    #[test]
    fn explicit_rollback_reports_timings() {
        let fx = Fixture::new("rollback");
        let ws = fx.workspace();
        let branch = ws.branch("rb").unwrap();
        let receipt = branch.rollback().unwrap();
        assert!(receipt.detach > Duration::ZERO);
        assert_eq!(fs::read_dir(ws.store().join("branches")).unwrap().count(), 0);
    }

    #[test]
    fn commit_swaps_atomically_and_keeps_previous_version() {
        let fx = Fixture::new("commit");
        let ws = fx.workspace();
        let branch = ws.branch("feature").unwrap();
        fs::write(branch.path().join("src/main.rs"), "fn main() { println!(\"v2\"); }\n").unwrap();
        let receipt = branch.commit().unwrap();
        assert_eq!(receipt.version, 1);
        assert!(fs::read_to_string(ws.live().join("src/main.rs")).unwrap().contains("v2"));
        let versions = ws.versions().unwrap();
        assert_eq!(versions.len(), 1);
        assert!(fs::read_to_string(versions[0].path.join("src/main.rs")).unwrap().contains("hi"));
        assert_eq!(fs::read_dir(ws.store().join("branches")).unwrap().count(), 0);
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
