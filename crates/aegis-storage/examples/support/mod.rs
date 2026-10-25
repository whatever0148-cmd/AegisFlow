//! Helpers shared by the storage examples: fixture trees, percentiles and a tiny JSON writer
//! (kept local so `aegis-storage` needs no extra dev-dependencies).

#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Files per module directory, close to the phase-1 bench fixtures (16 to 32 files per dir).
const FILES_PER_DIR: usize = 16;

/// Entry counts measured by the examples: about the phase-1 tiny, aegisflow_src, small and
/// medium fixtures.
pub const TREE_ENTRIES: [usize; 4] = [9, 61, 129, 1025];

/// Directory for example fixtures: `<workspace>/target/<name>-<pid>`, on the repo's APFS volume.
pub fn scratch_root(name: &str) -> io::Result<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target").join(format!("{name}-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;
    fs::canonicalize(&root)
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Shape of a generated tree; `entries` counts files and directories below the root.
#[derive(Debug, Clone, Copy)]
pub struct TreeInfo {
    pub entries: usize,
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
}

/// Builds a tree with exactly `entries` entries below `root`: `README.md`, then `module_NNN/`
/// directories holding up to 16 files each, file sizes uniform in `[min_bytes, max_bytes]`.
pub fn build_tree(root: &Path, entries: usize, min_bytes: usize, max_bytes: usize) -> io::Result<TreeInfo> {
    fs::create_dir_all(root)?;
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15 ^ entries as u64);
    let mut info = TreeInfo { entries: 0, files: 0, dirs: 0, bytes: 0 };
    let mut write_file = |path: &Path, info: &mut TreeInfo| -> io::Result<()> {
        let size = min_bytes + (rng.next() as usize) % (max_bytes - min_bytes + 1);
        let mut text = String::with_capacity(size + 64);
        let mut line = 0;
        while text.len() < size {
            text.push_str(&format!("pub const VALUE_{line}: u64 = 0x{:016x};\n", rng.next()));
            line += 1;
        }
        text.truncate(size);
        fs::write(path, &text)?;
        info.files += 1;
        info.entries += 1;
        info.bytes += size as u64;
        Ok(())
    };
    if entries == 0 {
        return Ok(info);
    }
    write_file(&root.join("README.md"), &mut info)?;
    let mut module = 0;
    while info.entries < entries {
        let dir = root.join(format!("module_{module:03}"));
        fs::create_dir(&dir)?;
        info.dirs += 1;
        info.entries += 1;
        let mut file = 0;
        while file < FILES_PER_DIR && info.entries < entries {
            write_file(&dir.join(format!("file_{file:03}.rs")), &mut info)?;
            file += 1;
        }
        module += 1;
    }
    Ok(info)
}

/// Linear-interpolated percentile of `samples` (microseconds); `None` when empty.
pub fn percentile(samples: &[f64], q: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64))
}

pub fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// Least-squares fit y = slope * x + intercept.
pub fn linear_fit(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.len() < 2 {
        return None;
    }
    let n = points.len() as f64;
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (sxx, sxy) = points.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x * x, b + x * y));
    let denom = n * sxx - sx * sx;
    if denom == 0.0 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / denom;
    Some((slope, (sy - slope * sx) / n))
}

/// Minimal JSON value for printing results.
pub enum Json {
    Num(f64),
    Int(u64),
    Bool(bool),
    Str(String),
    Null,
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn opt(value: Option<f64>) -> Json {
        value.map_or(Json::Null, Json::Num)
    }

    /// `{p50, p90, min, max, n}` summary of microsecond samples.
    pub fn summary(samples: &[f64]) -> Json {
        obj(vec![
            ("n", Json::Int(samples.len() as u64)),
            ("p50", Json::opt(percentile(samples, 0.5))),
            ("p90", Json::opt(percentile(samples, 0.9))),
            ("min", Json::opt(percentile(samples, 0.0))),
            ("max", Json::opt(percentile(samples, 1.0))),
        ])
    }

    pub fn render(&self, indent: usize, out: &mut String) {
        let pad = |n: usize| "  ".repeat(n);
        match self {
            Json::Num(v) if v.is_finite() => out.push_str(&format!("{:.3}", v)),
            Json::Num(_) | Json::Null => out.push_str("null"),
            Json::Int(v) => out.push_str(&v.to_string()),
            Json::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Json::Str(s) => {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Json::Arr(items) => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad(indent + 1));
                    item.render(indent + 1, out);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(indent));
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push_str("{\n");
                for (i, (key, value)) in fields.iter().enumerate() {
                    out.push_str(&pad(indent + 1));
                    Json::Str(key.clone()).render(indent + 1, out);
                    out.push_str(": ");
                    value.render(indent + 1, out);
                    out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(indent));
                out.push('}');
            }
        }
    }
}

pub fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn print_json(value: &Json) {
    let mut out = String::new();
    value.render(0, &mut out);
    println!("{out}");
}

/// `--iterations N` overrides every per-tree iteration count.
pub fn iterations_override() -> Option<usize> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2).find(|w| w[0] == "--iterations").and_then(|w| w[1].parse().ok())
}
