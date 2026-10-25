//! Where does branch-creation time go? Evidence for the per-entry cost of `clonefile(2)`.
//!
//! For trees of about 9, 61, 129 and 1025 entries (built under `target/`, i.e. on the repo's APFS
//! volume) this measures, per iteration:
//!
//! * one directory-level `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)`, exactly the
//!   call `Workspace::branch` makes (`aegis_storage::sys::clone_tree`): wall time and the
//!   process's user/system CPU deltas from `getrusage(RUSAGE_SELF)` around the call;
//! * `Workspace::branch()` itself (flock + stat + generation read + the same clonefile);
//! * for comparison, a userspace walk that recreates the tree with `mkdir` + one `clonefile` per
//!   file, i.e. what a userspace loop would cost.
//!
//! Plus two byte-independence controls: 1025 entries with 64 KiB files, and a single 64 MiB file.
//! If the time is per-inode kernel work, wall ~= sys, user ~= 0, cost scales with entries and not
//! with bytes. Run: `cargo run --release -p aegis-storage --example clone_scaling [-- --iterations N]`.
//! The process is single-threaded while measuring (synchronous reclaim, no reaper thread), so
//! RUSAGE_SELF deltas belong to the measured call.

mod support;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use aegis_storage::sys::{clone_tree, process_cpu_times};
use aegis_storage::Workspace;
use support::{build_tree, iterations_override, linear_fit, micros, obj, percentile, print_json, scratch_root, Json, TreeInfo, TREE_ENTRIES};

const ITERATIONS: [usize; 4] = [400, 300, 300, 60];
const WALK_ITERATIONS: [usize; 4] = [200, 100, 100, 20];
const CONTROL_ITERATIONS: usize = 30;
const WARMUP: usize = 5;

struct Samples {
    wall: Vec<f64>,
    sys: Vec<f64>,
    user: Vec<f64>,
}

impl Samples {
    fn new() -> Self {
        Self { wall: Vec::new(), sys: Vec::new(), user: Vec::new() }
    }

    fn json(&self, entries: usize) -> Json {
        let wall50 = percentile(&self.wall, 0.5);
        let sys50 = percentile(&self.sys, 0.5);
        let per_entry = |v: Option<f64>| Json::opt(v.filter(|_| entries > 0).map(|v| v / entries as f64));
        obj(vec![
            ("wall_us", Json::summary(&self.wall)),
            ("sys_us", Json::summary(&self.sys)),
            ("user_us", Json::summary(&self.user)),
            ("wall_us_per_entry_p50", per_entry(wall50)),
            ("sys_us_per_entry_p50", per_entry(sys50)),
            ("sys_share_of_wall_p50", Json::opt(wall50.zip(sys50).filter(|(w, _)| *w > 0.0).map(|(w, s)| s / w))),
            ("sys_zero_samples", Json::Int(self.sys.iter().filter(|s| **s == 0.0).count() as u64)),
        ])
    }
}

/// Runs `op` (which creates `dst`) and records wall time and RUSAGE_SELF deltas; `dst` is removed
/// afterwards, outside the measurement.
fn measure(samples: &mut Samples, dst: &Path, op: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let cpu0 = process_cpu_times()?;
    let started = Instant::now();
    op()?;
    let wall = started.elapsed();
    let cpu1 = process_cpu_times()?;
    samples.wall.push(micros(wall));
    samples.sys.push(micros(cpu1.system.saturating_sub(cpu0.system)));
    samples.user.push(micros(cpu1.user.saturating_sub(cpu0.user)));
    remove(dst)
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Single directory-level clonefile, as in `Workspace::branch`.
fn single_call(live: &Path, scratch: &Path, iterations: usize) -> io::Result<Samples> {
    let dst = scratch.join("clone");
    for _ in 0..WARMUP {
        clone_tree(live, &dst)?;
        remove(&dst)?;
    }
    let mut samples = Samples::new();
    for _ in 0..iterations {
        measure(&mut samples, &dst, || clone_tree(live, &dst))?;
    }
    Ok(samples)
}

/// Userspace reconstruction: mkdir per directory, clonefile per file (entry list collected first).
fn userspace_walk(live: &Path, scratch: &Path, iterations: usize) -> io::Result<Samples> {
    let mut entries: Vec<(PathBuf, bool)> = Vec::new();
    let mut stack = vec![live.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut children: Vec<_> = fs::read_dir(&dir)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let rel = child.path().strip_prefix(live).map(Path::to_path_buf).unwrap_or_default();
            let is_dir = child.file_type()?.is_dir();
            entries.push((rel, is_dir));
            if is_dir {
                stack.push(child.path());
            }
        }
    }
    let dst = scratch.join("walk");
    let walk = |dst: &Path| -> io::Result<()> {
        fs::create_dir(dst)?;
        for (rel, is_dir) in &entries {
            if *is_dir {
                fs::create_dir(dst.join(rel))?;
            } else {
                clone_tree(&live.join(rel), &dst.join(rel))?;
            }
        }
        Ok(())
    };
    for _ in 0..WARMUP.min(iterations) {
        walk(&dst)?;
        remove(&dst)?;
    }
    let mut samples = Samples::new();
    for _ in 0..iterations {
        measure(&mut samples, &dst, || walk(&dst))?;
    }
    Ok(samples)
}

/// `Workspace::branch()` wall time and its own clone_duration(), synchronous rollback untimed.
fn workspace_branch(live: &Path, iterations: usize) -> Result<(Samples, Vec<f64>), Box<dyn std::error::Error>> {
    let ws = Workspace::open(live)?;
    ws.set_background_reclaim(false);
    for _ in 0..WARMUP {
        ws.branch("warmup")?.rollback()?;
    }
    let mut samples = Samples::new();
    let mut clone_only = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let cpu0 = process_cpu_times()?;
        let started = Instant::now();
        let branch = ws.branch("scaling")?;
        let wall = started.elapsed();
        let cpu1 = process_cpu_times()?;
        samples.wall.push(micros(wall));
        samples.sys.push(micros(cpu1.system.saturating_sub(cpu0.system)));
        samples.user.push(micros(cpu1.user.saturating_sub(cpu0.user)));
        clone_only.push(micros(branch.clone_duration()));
        branch.rollback()?;
    }
    Ok((samples, clone_only))
}

fn tree_json(info: &TreeInfo, iterations: usize) -> Vec<(&'static str, Json)> {
    vec![
        ("entries", Json::Int(info.entries as u64)),
        ("files", Json::Int(info.files as u64)),
        ("dirs", Json::Int(info.dirs as u64)),
        ("bytes", Json::Int(info.bytes)),
        ("iterations", Json::Int(iterations as u64)),
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = scratch_root("clone-scaling")?;
    let fs_type = aegis_storage::sys::fs_type(&root)?;
    let mut trees = Vec::new();
    let mut wall_points = Vec::new();
    let mut sys_points = Vec::new();
    let mut walk_points = Vec::new();
    for (idx, &entries) in TREE_ENTRIES.iter().enumerate() {
        let iterations = iterations_override().unwrap_or(ITERATIONS[idx]);
        let walk_iterations = iterations_override().unwrap_or(WALK_ITERATIONS[idx]);
        let base = root.join(format!("tree-{entries}"));
        let live = base.join("live");
        let info = build_tree(&live, entries, 1024, 16 * 1024)?;
        let scratch = base.join("scratch");
        fs::create_dir_all(&scratch)?;
        eprintln!("clone_scaling: {entries} entries x {iterations}");

        let single = single_call(&live, &scratch, iterations)?;
        let (branch, branch_clone) = workspace_branch(&live, iterations)?;
        let walk = userspace_walk(&live, &scratch, walk_iterations)?;

        let x = info.entries as f64;
        if let Some(p) = percentile(&single.wall, 0.5) {
            wall_points.push((x, p));
        }
        if let Some(p) = percentile(&single.sys, 0.5) {
            sys_points.push((x, p));
        }
        if let Some(p) = percentile(&walk.wall, 0.5) {
            walk_points.push((x, p));
        }
        let mut fields = tree_json(&info, iterations);
        fields.push(("single_clonefile", single.json(info.entries)));
        fields.push(("workspace_branch", branch.json(info.entries)));
        fields.push(("workspace_branch_clone_duration_us", Json::summary(&branch_clone)));
        fields.push(("userspace_walk_iterations", Json::Int(walk_iterations as u64)));
        fields.push(("userspace_walk", walk.json(info.entries)));
        trees.push(obj(fields));
        fs::remove_dir_all(&base)?;
    }

    // Byte-independence controls: same entry count with 4x-64x larger files, and one big file.
    let mut controls = Vec::new();
    let control_iterations = iterations_override().unwrap_or(CONTROL_ITERATIONS);
    {
        let base = root.join("control-1025-64k");
        let live = base.join("live");
        let info = build_tree(&live, 1025, 64 * 1024, 64 * 1024)?;
        let scratch = base.join("scratch");
        fs::create_dir_all(&scratch)?;
        eprintln!("clone_scaling: control 1025 entries x 64 KiB");
        let single = single_call(&live, &scratch, control_iterations)?;
        let mut fields = vec![("name", Json::Str("1025 entries, 64 KiB files".into()))];
        fields.extend(tree_json(&info, control_iterations));
        fields.push(("single_clonefile", single.json(info.entries)));
        controls.push(obj(fields));
        fs::remove_dir_all(&base)?;
    }
    {
        let base = root.join("control-one-file");
        fs::create_dir_all(&base)?;
        let file = base.join("big.bin");
        let chunk: Vec<u8> = (0..(1 << 20)).map(|i| (i * 31 % 251) as u8).collect();
        let mut data = Vec::with_capacity(64 << 20);
        for _ in 0..64 {
            data.extend_from_slice(&chunk);
        }
        fs::write(&file, &data)?;
        eprintln!("clone_scaling: control single 64 MiB file");
        let single = single_call(&file, &base, control_iterations)?;
        let info = TreeInfo { entries: 1, files: 1, dirs: 0, bytes: data.len() as u64 };
        let mut fields = vec![("name", Json::Str("single 64 MiB file".into()))];
        fields.extend(tree_json(&info, control_iterations));
        fields.push(("single_clonefile", single.json(info.entries)));
        controls.push(obj(fields));
        fs::remove_dir_all(&base)?;
    }
    fs::remove_dir_all(&root)?;

    let fit = |points: &[(f64, f64)]| match linear_fit(points) {
        Some((slope, intercept)) => obj(vec![("us_per_entry", Json::Num(slope)), ("intercept_us", Json::Num(intercept))]),
        None => Json::Null,
    };
    print_json(&obj(vec![
        ("example", Json::Str("clone_scaling".into())),
        ("fs_type", Json::Str(fs_type)),
        (
            "note",
            Json::Str(
                "preliminary: measured on a shared machine with concurrent compiles; the lead runs the quiet benchmark"
                    .into(),
            ),
        ),
        ("trees", Json::Arr(trees)),
        ("fit_single_clonefile_wall_p50", fit(&wall_points)),
        ("fit_single_clonefile_sys_p50", fit(&sys_points)),
        ("fit_userspace_walk_wall_p50", fit(&walk_points)),
        ("byte_independence_controls", Json::Arr(controls)),
    ]));
    Ok(())
}
