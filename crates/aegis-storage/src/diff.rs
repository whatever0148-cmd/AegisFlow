//! Tree diff between the live workspace and a branch.
//!
//! Files are compared by type, size and mtime first (`clonefile` preserves mtimes, writes update
//! them); equal sizes with different mtimes fall back to a byte comparison. Line counts use a
//! multiset diff of lines, which is exact for pure additions/removals and an approximation for
//! moved lines.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Text files larger than this are counted but not line-diffed.
const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;
/// Upper bound on the added/removed text kept per file.
const MAX_EXCERPT_BYTES: usize = 64 * 1024;
/// Directories that are skipped: VCS object stores and build outputs.
const SKIP_DIRS: &[&str] = &[".git/objects", ".git/logs", "target", "node_modules", ".build"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileChange {
    /// Path relative to the workspace root.
    pub path: PathBuf,
    pub kind: ChangeKind,
    pub old_len: u64,
    pub new_len: u64,
    pub is_symlink: bool,
    pub mode_changed: bool,
    pub binary: bool,
    pub lines_added: u32,
    pub lines_removed: u32,
    /// Added lines joined with '\n' (text files only, capped).
    pub added_text: String,
    /// Removed lines joined with '\n' (text files only, capped).
    pub removed_text: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ChangeSet {
    pub files: Vec<FileChange>,
    /// Entries (files + symlinks) examined on both sides.
    pub scanned_entries: usize,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn count(&self, kind: ChangeKind) -> usize {
        self.files.iter().filter(|f| f.kind == kind).count()
    }

    /// Bytes written or removed: |new - old| for modifications, full size for adds/deletes.
    pub fn bytes_changed(&self) -> u64 {
        self.files
            .iter()
            .map(|f| match f.kind {
                ChangeKind::Added => f.new_len,
                ChangeKind::Deleted => f.old_len,
                ChangeKind::Modified => f.new_len.abs_diff(f.old_len).max(1),
            })
            .sum()
    }

    pub fn lines_added(&self) -> u64 {
        self.files.iter().map(|f| u64::from(f.lines_added)).sum()
    }

    pub fn lines_removed(&self) -> u64 {
        self.files.iter().map(|f| u64::from(f.lines_removed)).sum()
    }
}

#[derive(Debug, Clone)]
struct Entry {
    abs: PathBuf,
    len: u64,
    mtime: (i64, i64),
    mode: u32,
    symlink: bool,
}

/// Lists every file and symlink below `root`, keyed by relative path.
fn walk(root: &Path) -> io::Result<BTreeMap<PathBuf, Entry>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let abs = entry.path();
            let rel = abs.strip_prefix(root).unwrap_or(&abs).to_path_buf();
            let meta = fs::symlink_metadata(&abs)?;
            if meta.is_dir() {
                let rel_str = rel.to_string_lossy();
                if !SKIP_DIRS.iter().any(|s| rel_str == *s) {
                    stack.push(abs);
                }
                continue;
            }
            out.insert(
                rel,
                Entry {
                    abs,
                    len: meta.len(),
                    mtime: (meta.mtime(), meta.mtime_nsec()),
                    mode: meta.permissions().mode() & 0o7777,
                    symlink: meta.file_type().is_symlink(),
                },
            );
        }
    }
    Ok(out)
}

fn read_capped(entry: &Entry) -> io::Result<Option<Vec<u8>>> {
    if entry.symlink {
        return Ok(Some(fs::read_link(&entry.abs)?.to_string_lossy().into_owned().into_bytes()));
    }
    if entry.len > MAX_TEXT_BYTES {
        return Ok(None);
    }
    fs::read(&entry.abs).map(Some)
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

fn cap(mut text: String) -> String {
    if text.len() > MAX_EXCERPT_BYTES {
        let mut cut = MAX_EXCERPT_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

/// Multiset line diff: (added, removed, added text, removed text).
fn line_delta(old: &str, new: &str) -> (u32, u32, String, String) {
    let mut remaining: HashMap<&str, i64> = HashMap::new();
    for line in old.lines() {
        *remaining.entry(line).or_default() += 1;
    }
    let mut added = Vec::new();
    for line in new.lines() {
        match remaining.get_mut(line) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added.push(line),
        }
    }
    let mut removed = Vec::new();
    for line in old.lines() {
        if let Some(n) = remaining.get_mut(line) {
            if *n > 0 {
                removed.push(line);
                *n -= 1;
            }
        }
    }
    (added.len() as u32, removed.len() as u32, cap(added.join("\n")), cap(removed.join("\n")))
}

fn describe(path: PathBuf, kind: ChangeKind, old: Option<&Entry>, new: Option<&Entry>) -> io::Result<FileChange> {
    let old_bytes = match old {
        Some(e) => read_capped(e)?,
        None => Some(Vec::new()),
    };
    let new_bytes = match new {
        Some(e) => read_capped(e)?,
        None => Some(Vec::new()),
    };
    let binary = old_bytes.as_deref().map(is_binary).unwrap_or(true) || new_bytes.as_deref().map(is_binary).unwrap_or(true);
    let (lines_added, lines_removed, added_text, removed_text) = match (&old_bytes, &new_bytes) {
        (Some(o), Some(n)) if !binary => line_delta(&String::from_utf8_lossy(o), &String::from_utf8_lossy(n)),
        _ => (0, 0, String::new(), String::new()),
    };
    Ok(FileChange {
        path,
        kind,
        old_len: old.map(|e| e.len).unwrap_or(0),
        new_len: new.map(|e| e.len).unwrap_or(0),
        is_symlink: old.map(|e| e.symlink).unwrap_or(false) || new.map(|e| e.symlink).unwrap_or(false),
        mode_changed: matches!((old, new), (Some(o), Some(n)) if o.mode != n.mode),
        binary,
        lines_added,
        lines_removed,
        added_text,
        removed_text,
    })
}

fn same_content(a: &Entry, b: &Entry) -> io::Result<bool> {
    if a.symlink != b.symlink || a.len != b.len {
        return Ok(false);
    }
    if a.mtime == b.mtime && a.mode == b.mode {
        return Ok(true);
    }
    if a.symlink {
        return Ok(fs::read_link(&a.abs)? == fs::read_link(&b.abs)?);
    }
    Ok(a.mode == b.mode && fs::read(&a.abs)? == fs::read(&b.abs)?)
}

/// Changes needed to turn `base` into `head`.
pub fn diff_trees(base: &Path, head: &Path) -> io::Result<ChangeSet> {
    let old = walk(base)?;
    let new = walk(head)?;
    let mut files = Vec::new();
    for (path, new_entry) in &new {
        match old.get(path) {
            None => files.push(describe(path.clone(), ChangeKind::Added, None, Some(new_entry))?),
            Some(old_entry) if !same_content(old_entry, new_entry)? => {
                files.push(describe(path.clone(), ChangeKind::Modified, Some(old_entry), Some(new_entry))?)
            }
            Some(_) => {}
        }
    }
    for (path, old_entry) in &old {
        if !new.contains_key(path) {
            files.push(describe(path.clone(), ChangeKind::Deleted, Some(old_entry), None)?);
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(ChangeSet { files, scanned_entries: old.len() + new.len() })
}
