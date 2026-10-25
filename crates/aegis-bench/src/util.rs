//! Shared helpers: percentile summaries, fixture trees, tree hashing, host and git facts.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub type Res<T> = Result<T, Box<dyn Error>>;

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

pub fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

pub fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// Nearest-rank percentile of unsorted samples (`p` in 0..=100).
pub fn percentile(samples: &[f64], p: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    Some(sorted[((p / 100.0 * n as f64).ceil() as usize).clamp(1, n) - 1])
}

/// Nearest-rank percentile summary.
pub fn stats(mut samples: Vec<f64>) -> Value {
    if samples.is_empty() {
        return json!({"n": 0});
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    let pct = |p: f64| samples[((p / 100.0 * n as f64).ceil() as usize).clamp(1, n) - 1];
    json!({
        "n": n,
        "min": round6(samples[0]),
        "p50": round6(pct(50.0)),
        "p95": round6(pct(95.0)),
        "p99": round6(pct(99.0)),
        "max": round6(samples[n - 1]),
        "mean": round6(samples.iter().sum::<f64>() / n as f64),
    })
}

pub fn p50(v: &Value) -> Option<f64> {
    v["p50"].as_f64()
}

pub fn p95(v: &Value) -> Option<f64> {
    v["p95"].as_f64()
}

/// Iteration count scaled by `--scale`/`--quick`, never below 5.
pub fn scaled(n: usize, scale: f64) -> usize {
    ((n as f64 * scale).round() as usize).max(5)
}

pub fn ensure(ok: bool, what: &str) -> Res<()> {
    if ok {
        Ok(())
    } else {
        Err(format!("timed out waiting for {what}").into())
    }
}

/// Least-squares line `y = slope * x + intercept`; `None` with fewer than two distinct x.
pub fn linear_fit(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (sxx, sxy) = points.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x * x, b + x * y));
    let denom = n * sxx - sx * sx;
    if points.len() < 2 || denom.abs() < f64::EPSILON {
        return None;
    }
    let slope = (n * sxy - sx * sy) / denom;
    Some((slope, (sy - slope * sx) / n))
}

pub fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Performance-core count (`hw.perflevel0.physicalcpu`), else the physical core count.
pub fn performance_cores() -> Option<u32> {
    ["hw.perflevel0.physicalcpu", "hw.physicalcpu"]
        .iter()
        .find_map(|name| command_line("sysctl", &["-n", name]).and_then(|v| v.parse::<u32>().ok()))
}

/// 1-, 5- and 15-minute load averages from `sysctl vm.loadavg` (`{ 1.23 4.56 7.89 }`).
pub fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let values: Vec<f64> = text.split_whitespace().filter_map(|t| t.parse::<f64>().ok()).collect();
    (values.len() >= 3).then(|| [values[0], values[1], values[2]])
}

pub fn load_average() -> Option<[f64; 3]> {
    parse_loadavg(&command_line("sysctl", &["-n", "vm.loadavg"])?)
}

/// Paths whose contents make up the measured build (same scope as the post-task hook).
pub const SOURCE_PATHS: [&str; 5] = ["crates", "policies", "models", "Cargo.toml", "Cargo.lock"];
const SKIP_DIRS: [&str; 5] = ["target", ".build", "__pycache__", ".venv", ".git"];

/// Relative paths of the source files below `root`: tracked plus untracked-but-not-ignored files
/// (`git ls-files`), or a directory walk that skips build output when git is unavailable.
pub fn source_files(root: &Path) -> Res<Vec<String>> {
    let git = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--"])
        .args(SOURCE_PATHS)
        .output();
    if let Ok(out) = git {
        if out.status.success() && !out.stdout.is_empty() {
            let mut files: Vec<String> =
                out.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect();
            files.sort();
            files.dedup();
            return Ok(files);
        }
    }
    let mut files = Vec::new();
    for top in SOURCE_PATHS {
        let mut stack = vec![root.join(top)];
        while let Some(path) = stack.pop() {
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() {
                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    if !SKIP_DIRS.iter().any(|s| entry.file_name() == *s) {
                        stack.push(entry.path());
                    }
                }
            } else if path.file_name().map_or(true, |n| n != ".DS_Store") {
                files.push(path.strip_prefix(root)?.to_string_lossy().into_owned());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// SHA-256 over (relative path, content hash) of every source file: identifies the code a run
/// measured, independent of commits and file times. Deleted tracked files are skipped (which
/// changes the fingerprint).
pub fn source_fingerprint(root: &Path) -> Res<Value> {
    let mut hasher = Sha256::new();
    let mut hashed = 0usize;
    for rel in source_files(root)? {
        let bytes = match fs::read(root.join(&rel)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("reading {rel}: {e}").into()),
        };
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update(Sha256::digest(&bytes));
        hasher.update([0]);
        hashed += 1;
    }
    Ok(json!({
        "sha256": hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "files": hashed,
        "paths": SOURCE_PATHS,
        "method": "SHA-256 over (path, SHA-256 of content) of the tracked and untracked-but-not-ignored files under paths",
    }))
}

pub fn host_info(root: &Path) -> Value {
    let sysctl = |name: &str| command_line("sysctl", &["-n", name]);
    let memory_gb = sysctl("hw.memsize").and_then(|v| v.parse::<f64>().ok()).map(|b| b / 1_073_741_824.0);
    let rustc = command_line("rustc", &["--version"]).or_else(|| {
        let home = std::env::var("HOME").ok()?;
        command_line(&format!("{home}/.cargo/bin/rustc"), &["--version"])
    });
    let card: Value = fs::read_to_string(root.join("models/model_card.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let load = command_line("sysctl", &["-n", "vm.loadavg"]);
    json!({
        "model": sysctl("hw.model"),
        "cpu": sysctl("machdep.cpu.brand_string"),
        "performance_cores": sysctl("hw.perflevel0.physicalcpu").and_then(|v| v.parse::<u32>().ok()),
        "efficiency_cores": sysctl("hw.perflevel1.physicalcpu").and_then(|v| v.parse::<u32>().ok()),
        "memory_gb": memory_gb,
        "macos": command_line("sw_vers", &["-productVersion"]),
        "macos_build": command_line("sw_vers", &["-buildVersion"]),
        "kernel": command_line("uname", &["-r"]),
        "rustc": rustc,
        "swiftc": command_line("xcrun", &["swiftc", "--version"]).and_then(|v| v.lines().find(|l| l.contains("Swift version")).map(str::to_owned)),
        "coremltools": card["toolchain"]["coremltools"],
        "load_average_at_start": load,
    })
}

pub fn git_info(root: &Path) -> Value {
    let root_s = root.to_string_lossy();
    let commit = command_line("git", &["-C", &root_s, "rev-parse", "--short", "HEAD"]);
    let dirty = Command::new("git")
        .args(["-C", &root_s, "status", "--porcelain", "--", "crates", "policies", "models", "Cargo.toml", "Cargo.lock"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(true);
    let missing = command_line("git", &["-C", &root_s, "rev-parse", "HEAD"]).is_none();
    json!({"commit": commit.unwrap_or_else(|| "uncommitted".to_string()), "dirty": dirty || missing})
}

/// Deterministic xorshift for fixture contents.
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() as usize) % (hi - lo + 1)
    }
}

pub struct FixtureSpec {
    pub name: &'static str,
    pub dirs: usize,
    pub files_per_dir: usize,
    pub min_bytes: usize,
    pub max_bytes: usize,
    pub iterations: usize,
}

/// Generated fixtures, the same shapes as phase 1 (plus a README.md each).
pub const FIXTURES: [FixtureSpec; 3] = [
    FixtureSpec { name: "tiny", dirs: 2, files_per_dir: 4, min_bytes: 512, max_bytes: 4096, iterations: 400 },
    FixtureSpec { name: "small", dirs: 8, files_per_dir: 16, min_bytes: 1024, max_bytes: 16384, iterations: 300 },
    FixtureSpec { name: "medium", dirs: 32, files_per_dir: 32, min_bytes: 1024, max_bytes: 16384, iterations: 60 },
];

pub fn generate_fixture(live: &Path, spec: &FixtureSpec) -> Res<()> {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15 ^ spec.dirs as u64);
    for d in 0..spec.dirs {
        let dir = live.join(format!("src/module_{d:03}"));
        fs::create_dir_all(&dir)?;
        for f in 0..spec.files_per_dir {
            let size = rng.range(spec.min_bytes, spec.max_bytes);
            let mut text = String::with_capacity(size + 64);
            let mut line = 0;
            while text.len() < size {
                text.push_str(&format!("pub const VALUE_{line}: u64 = 0x{:016x};\n", rng.next()));
                line += 1;
            }
            fs::write(dir.join(format!("file_{f:03}.rs")), text)?;
        }
    }
    fs::write(live.join("README.md"), format!("# {} fixture\n", spec.name))?;
    Ok(())
}

/// (files, directories, bytes) below `root`.
pub fn tree_size(root: &Path) -> Res<(usize, usize, u64)> {
    let (mut files, mut dirs, mut bytes) = (0, 0, 0);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.is_dir() {
                dirs += 1;
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    Ok((files, dirs, bytes))
}

/// Relative path -> SHA-256 of contents for every file below `root` (directories implied).
pub type Manifest = BTreeMap<String, String>;

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn tree_manifest(root: &Path) -> Res<Manifest> {
    let mut manifest = Manifest::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if fs::symlink_metadata(&path)?.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root)?.to_string_lossy().into_owned();
                manifest.insert(rel, sha256_hex(&fs::read(&path)?));
            }
        }
    }
    Ok(manifest)
}

/// SHA-256 over (relative path, content hash) of every file, in sorted order.
pub fn manifest_hash(manifest: &Manifest) -> String {
    let mut hasher = Sha256::new();
    for (path, digest) in manifest {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(digest.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Number of entries (files and directories, recursively) below `dir`; 0 if it does not exist.
pub fn count_entries(dir: &Path) -> usize {
    let Ok(read) = fs::read_dir(dir) else { return 0 };
    read.flatten()
        .map(|e| {
            let path = e.path();
            1 + if fs::symlink_metadata(&path).map(|m| m.is_dir()).unwrap_or(false) { count_entries(&path) } else { 0 }
        })
        .sum()
}
