//! Do APFS metadata operations stall while a large directory `clonefile` (or a tree deletion)
//! runs on the same volume?
//!
//! The warm pool replenishes right after an acquire, and the reaper deletes right after a
//! rollback, so this decides whether that background work is invisible to the agent. For each
//! probe operation it measures the duration alone (baseline) and while another thread is inside
//! `clonefile` of a 1025-entry tree, or inside `remove_dir_all` of one. The probe starts once the
//! background operation has been running for `PROBE_DELAY` (its own duration is reported).
//! Run: `cargo run --release -p aegis-storage --example apfs_contention [-- --iterations N]`.

mod support;

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use aegis_storage::sys::{clone_tree, rename_excl};
use support::{build_tree, iterations_override, micros, obj, percentile, print_json, scratch_root, Json};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const TRIALS: usize = 20;
const BIG_TREE: usize = 1025;
/// Head start of the background operation before the probe runs (a configured delay, not a result).
const PROBE_DELAY: Duration = Duration::from_millis(1);

#[derive(Clone, Copy)]
enum Probe {
    RenameInStore,
    RenameElsewhere,
    CreateFileElsewhere,
    CreateFileInBranch,
    PwriteExisting,
    Stat,
    CloneTinyTree,
}

impl Probe {
    const ALL: [Probe; 7] = [
        Probe::RenameInStore,
        Probe::RenameElsewhere,
        Probe::CreateFileElsewhere,
        Probe::CreateFileInBranch,
        Probe::PwriteExisting,
        Probe::Stat,
        Probe::CloneTinyTree,
    ];

    fn name(self) -> &'static str {
        match self {
            Probe::RenameInStore => "rename dir inside the store (rollback detach)",
            Probe::RenameElsewhere => "rename dir in an unrelated directory, same volume",
            Probe::CreateFileElsewhere => "create+write+close small file, unrelated directory",
            Probe::CreateFileInBranch => "create+write+close small file inside an acquired branch",
            Probe::PwriteExisting => "pwrite 4 KiB into an already open file",
            Probe::Stat => "stat a file",
            Probe::CloneTinyTree => "clonefile of a 9-entry tree",
        }
    }
}

struct Paths {
    store: PathBuf,
    other: PathBuf,
    branch: PathBuf,
    tiny: PathBuf,
    open_file: File,
}

fn run_probe(probe: Probe, p: &Paths, n: usize) -> Res<Duration> {
    let started;
    match probe {
        Probe::RenameInStore => {
            let (a, b) = (p.store.join(format!("dir-{n}")), p.store.join(format!("trash-{n}")));
            fs::create_dir(&a)?;
            started = Instant::now();
            rename_excl(&a, &b)?;
        }
        Probe::RenameElsewhere => {
            let (a, b) = (p.other.join(format!("dir-{n}")), p.other.join(format!("moved-{n}")));
            fs::create_dir(&a)?;
            started = Instant::now();
            rename_excl(&a, &b)?;
        }
        Probe::CreateFileElsewhere => {
            started = Instant::now();
            let mut f = File::create(p.other.join(format!("file-{n}.txt")))?;
            f.write_all(b"probe\n")?;
        }
        Probe::CreateFileInBranch => {
            started = Instant::now();
            let mut f = File::create(p.branch.join(format!("agent-{n}.rs")))?;
            f.write_all(b"// probe\n")?;
        }
        Probe::PwriteExisting => {
            let block = [n as u8; 4096];
            started = Instant::now();
            p.open_file.write_all_at(&block, 0)?;
        }
        Probe::Stat => {
            started = Instant::now();
            fs::metadata(p.other.join("stat-target"))?;
        }
        Probe::CloneTinyTree => {
            let dst = p.other.join(format!("tiny-clone-{n}"));
            started = Instant::now();
            clone_tree(&p.tiny, &dst)?;
        }
    }
    Ok(started.elapsed())
}

/// Starts `background` in another thread, waits PROBE_DELAY, runs the probe; returns
/// (probe duration, background duration).
fn contended(probe: Probe, p: &Paths, n: usize, background: impl FnOnce() -> Res<Duration> + Send + 'static) -> Res<(Duration, Duration)> {
    let barrier = Arc::new(Barrier::new(2));
    let gate = Arc::clone(&barrier);
    let handle = thread::spawn(move || -> Result<Duration, String> {
        gate.wait();
        background().map_err(|e| e.to_string())
    });
    barrier.wait();
    thread::sleep(PROBE_DELAY);
    let probe_took = run_probe(probe, p, n)?;
    let bg = handle.join().map_err(|_| "background thread panicked")??;
    Ok((probe_took, bg))
}

fn main() -> Res<()> {
    let trials = iterations_override().unwrap_or(TRIALS);
    let root = scratch_root("apfs-contention")?;
    let big = root.join("big-live");
    build_tree(&big, BIG_TREE, 1024, 16 * 1024)?;
    let tiny = root.join("tiny-live");
    build_tree(&tiny, 9, 1024, 4096)?;
    let store = root.join(".store");
    let other = root.join("unrelated");
    fs::create_dir_all(&store)?;
    fs::create_dir_all(&other)?;
    fs::write(other.join("stat-target"), "x")?;
    let branch = store.join("branch");
    clone_tree(&tiny, &branch)?;
    let open_file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(other.join("open.bin"))?;
    open_file.write_all_at(&[0u8; 4096], 0)?;
    let paths = Paths { store: store.clone(), other, branch, tiny, open_file };

    let mut results = Vec::new();
    let mut n = 0usize;
    for probe in Probe::ALL {
        eprintln!("apfs_contention: {}", probe.name());
        let (mut alone, mut during_clone, mut clone_bg, mut during_rm, mut rm_bg) = (vec![], vec![], vec![], vec![], vec![]);
        for _ in 0..trials {
            n += 1;
            alone.push(micros(run_probe(probe, &paths, n)?));

            n += 1;
            let (src, dst) = (big.clone(), store.join(format!("standby-{n}")));
            let dst_bg = dst.clone();
            let (took, bg) = contended(probe, &paths, n, move || {
                let started = Instant::now();
                clone_tree(&src, &dst_bg)?;
                Ok(started.elapsed())
            })?;
            during_clone.push(micros(took));
            clone_bg.push(micros(bg));

            n += 1;
            let doomed = dst;
            let (took, bg) = contended(probe, &paths, n, move || {
                let started = Instant::now();
                fs::remove_dir_all(&doomed)?;
                Ok(started.elapsed())
            })?;
            during_rm.push(micros(took));
            rm_bg.push(micros(bg));
        }
        let ratio = |a: &[f64]| percentile(a, 0.5).zip(percentile(&alone, 0.5)).filter(|(_, b)| *b > 0.0).map(|(x, b)| x / b);
        results.push(obj(vec![
            ("probe", Json::Str(probe.name().into())),
            ("alone_us", Json::summary(&alone)),
            ("during_clonefile_us", Json::summary(&during_clone)),
            ("during_clonefile_over_alone_p50", Json::opt(ratio(&during_clone))),
            ("background_clonefile_us", Json::summary(&clone_bg)),
            ("during_tree_delete_us", Json::summary(&during_rm)),
            ("during_tree_delete_over_alone_p50", Json::opt(ratio(&during_rm))),
            ("background_tree_delete_us", Json::summary(&rm_bg)),
        ]));
    }
    drop(paths);
    fs::remove_dir_all(&root)?;
    print_json(&obj(vec![
        ("example", Json::Str("apfs_contention".into())),
        ("note", Json::Str("preliminary: shared machine with concurrent compiles".into())),
        ("big_tree_entries", Json::Int(BIG_TREE as u64)),
        ("probe_delay_us", Json::Num(micros(PROBE_DELAY))),
        ("trials", Json::Int(trials as u64)),
        ("probes", Json::Arr(results)),
    ]));
    Ok(())
}
