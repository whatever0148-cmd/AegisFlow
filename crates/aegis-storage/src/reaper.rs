//! Background reclamation of detached trees.
//!
//! Rollback, version pruning and pool disposal first make a tree unreachable with one rename into
//! `<store>/trash/` (O(1)); deleting it is O(entries) and is handed to this reaper so the caller
//! does not wait for it. One reaper thread exists per [`crate::Workspace`] (shared by its clones
//! and branches); it is spawned on the first job and drains its queue before exiting once the
//! workspace and every branch made from it are gone. Trees still in `trash/` after a crash are
//! removed by [`crate::Workspace::reclaim_trash`].

use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Upper bound on per-job records kept for [`crate::Workspace::drain_reap_records`]; the oldest
/// records are dropped first when nobody drains them.
const MAX_RECORDS: usize = 1 << 16;

/// What a reaped tree used to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ReapKind {
    /// A rolled-back (or conflicting, or dropped) branch.
    Branch,
    /// A committed version pruned by the retention limit.
    Version,
    /// A warm-pool standby that went stale or was released.
    Standby,
}

/// One finished reclaim job.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReapRecord {
    /// Ticket returned when the job was queued (see [`crate::RollbackReceipt::reap_ticket`]).
    pub ticket: u64,
    pub kind: ReapKind,
    /// File name of the tree inside `trash/`.
    pub name: String,
    /// Time between queueing and the start of deletion.
    pub waited: Duration,
    /// Time spent deleting the tree.
    pub duration: Duration,
    /// False when deletion failed; the tree is then left in `trash/`.
    pub ok: bool,
}

/// Snapshot of the reaper's counters.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReaperStats {
    /// Jobs queued or in progress right now.
    pub queued: usize,
    /// Jobs finished successfully since the workspace was opened.
    pub reaped: u64,
    /// Jobs whose deletion failed (tree left in `trash/` for `reclaim_trash`).
    pub failed: u64,
    /// Deletion time of the most recently finished job.
    pub last_reap: Option<Duration>,
    /// Whether rollback and pruning currently hand trees to the reaper.
    pub background: bool,
    /// Most recent failure (deletion error or thread spawn error).
    pub last_error: Option<String>,
}

struct Job {
    ticket: u64,
    kind: ReapKind,
    path: PathBuf,
    queued: Instant,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    active: Option<OsString>,
    next_ticket: u64,
    reaped: u64,
    failed: u64,
    last_reap: Option<Duration>,
    last_error: Option<String>,
    records: VecDeque<ReapRecord>,
    thread_running: bool,
    shutdown: bool,
    paused: bool,
}

struct Core {
    state: Mutex<State>,
    cond: Condvar,
}

impl Core {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct Reaper {
    core: Arc<Core>,
}

impl std::fmt::Debug for Reaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.core.lock();
        f.debug_struct("Reaper").field("queued", &st.queue.len()).field("reaped", &st.reaped).finish()
    }
}

impl Reaper {
    pub(crate) fn new() -> Self {
        Self { core: Arc::new(Core { state: Mutex::new(State::default()), cond: Condvar::new() }) }
    }

    /// Queues `path` (already inside `trash/`) for deletion and returns its ticket. Fails only if
    /// the reaper thread cannot be spawned; the caller then deletes synchronously.
    pub(crate) fn enqueue(&self, kind: ReapKind, path: PathBuf) -> io::Result<u64> {
        let mut st = self.core.lock();
        if !st.thread_running {
            let core = Arc::clone(&self.core);
            let spawned = std::thread::Builder::new().name("aegis-reaper".into()).spawn(move || run(core));
            match spawned {
                Ok(_detached) => st.thread_running = true,
                Err(e) => {
                    st.last_error = Some(format!("spawn reaper thread: {e}"));
                    return Err(e);
                }
            }
        }
        st.next_ticket += 1;
        let ticket = st.next_ticket;
        st.queue.push_back(Job { ticket, kind, path, queued: Instant::now() });
        drop(st);
        self.core.cond.notify_all();
        Ok(ticket)
    }

    pub(crate) fn stats(&self, background: bool) -> ReaperStats {
        let st = self.core.lock();
        ReaperStats {
            queued: st.queue.len() + usize::from(st.active.is_some()),
            reaped: st.reaped,
            failed: st.failed,
            last_reap: st.last_reap,
            background,
            last_error: st.last_error.clone(),
        }
    }

    pub(crate) fn drain_records(&self) -> Vec<ReapRecord> {
        self.core.lock().records.drain(..).collect()
    }

    /// Blocks until nothing is queued or in progress, or `timeout` elapses.
    pub(crate) fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.core.lock();
        loop {
            if st.queue.is_empty() && st.active.is_none() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            st = self.core.cond.wait_timeout(st, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Trash entry names this reaper owns (queued or being deleted).
    pub(crate) fn pending_names(&self) -> HashSet<OsString> {
        let st = self.core.lock();
        let mut names: HashSet<OsString> =
            st.queue.iter().filter_map(|job| job.path.file_name().map(|n| n.to_os_string())).collect();
        if let Some(active) = &st.active {
            names.insert(active.clone());
        }
        names
    }

    /// Holds the reaper before its next job (tests use this to observe queued state deterministically).
    #[cfg(test)]
    pub(crate) fn set_paused(&self, paused: bool) {
        self.core.lock().paused = paused;
        self.core.cond.notify_all();
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        // The thread finishes the queued jobs and exits; it is not joined so dropping the last
        // workspace handle never blocks on deletion.
        self.core.lock().shutdown = true;
        self.core.cond.notify_all();
    }
}

fn run(core: Arc<Core>) {
    let mut st = core.lock();
    loop {
        if st.paused && !st.shutdown {
            st = core.cond.wait(st).unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        let Some(job) = st.queue.pop_front() else {
            if st.shutdown {
                st.thread_running = false;
                return;
            }
            st = core.cond.wait(st).unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let name = job.path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        st.active = Some(name.clone());
        drop(st);

        let waited = job.queued.elapsed();
        let started = Instant::now();
        let result = crate::remove_any(&job.path);
        let duration = started.elapsed();

        st = core.lock();
        st.active = None;
        let ok = result.is_ok();
        match result {
            Ok(()) => st.reaped += 1,
            Err(e) => {
                st.failed += 1;
                st.last_error = Some(e.to_string());
            }
        }
        st.last_reap = Some(duration);
        if st.records.len() >= MAX_RECORDS {
            st.records.pop_front();
        }
        st.records.push_back(ReapRecord {
            ticket: job.ticket,
            kind: job.kind,
            name: name.to_string_lossy().into_owned(),
            waited,
            duration,
            ok,
        });
        core.cond.notify_all();
    }
}
