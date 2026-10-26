//! Redacted, size-capped unified diffs for the console's diff viewer (API.md "UnifiedDiff").
//!
//! [`unified`] renders the files of a [`ChangeSet`] as a unified diff (three context lines) from
//! the two trees on disk. Every file's old and new contents go through the same redaction as the
//! approval excerpt (`aegis_governance::scanner`, plus the caller's scrub of configured secret
//! values) *before* they are split into lines, so a secret never reaches the text, and the
//! assembled text is redacted once more. The result is cut at a line boundary to
//! [`DIFF_CAP_BYTES`] and says so (`truncated`, `bytes_total`). Building it is not part of any
//! stage's measured `ms`: callers time the diff itself and render this afterwards.

use std::fs;
use std::path::Path;

use aegis_governance::scanner;
use aegis_storage::{ChangeKind, ChangeSet};
use serde_json::{json, Value};

/// Size cap of `text` (bytes, cut at a line boundary).
pub const DIFF_CAP_BYTES: usize = 24 * 1024;
/// Files stop being rendered once the full (pre-cap) text reaches this size; they are counted.
const RAW_LIMIT_BYTES: usize = 512 * 1024;
/// Files larger than this are listed with a note instead of their lines.
const MAX_FILE_BYTES: u64 = 256 * 1024;
const CONTEXT: usize = 3;
/// Upper bound on the LCS table; beyond it the changed middle is shown as remove-all/add-all.
const MAX_LCS_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Del,
    Ins,
}

/// Edit script from `old` to `new`: common prefix/suffix trimmed, LCS on the middle.
fn edit_script(old: &[&str], new: &[&str]) -> Vec<Op> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];
    let mut ops = vec![Op::Eq; prefix];
    if a.len().saturating_mul(b.len()) > MAX_LCS_CELLS {
        ops.extend(std::iter::repeat_n(Op::Del, a.len()));
        ops.extend(std::iter::repeat_n(Op::Ins, b.len()));
    } else {
        // lcs[i][j]: LCS length of a[i..] and b[j..].
        let (n, m) = (a.len(), b.len());
        let mut lcs = vec![0u32; (n + 1) * (m + 1)];
        let at = |i: usize, j: usize| i * (m + 1) + j;
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[at(i, j)] = if a[i] == b[j] { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if a[i] == b[j] {
                ops.push(Op::Eq);
                i += 1;
                j += 1;
            } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
                ops.push(Op::Del);
                i += 1;
            } else {
                ops.push(Op::Ins);
                j += 1;
            }
        }
        ops.extend(std::iter::repeat_n(Op::Del, n - i));
        ops.extend(std::iter::repeat_n(Op::Ins, m - j));
    }
    ops.extend(std::iter::repeat_n(Op::Eq, suffix));
    ops
}

/// Hunks of a unified diff between `old` and `new` (no file header).
pub fn hunks(old: &str, new: &str) -> String {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let ops = edit_script(&old, &new);
    // Position of each op in old/new before it applies.
    let mut pos = Vec::with_capacity(ops.len());
    let (mut o, mut n) = (0usize, 0usize);
    for op in &ops {
        pos.push((o, n));
        match op {
            Op::Eq => {
                o += 1;
                n += 1;
            }
            Op::Del => o += 1,
            Op::Ins => n += 1,
        }
    }
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, op)| **op != Op::Eq).map(|(i, _)| i).collect();
    let mut out = String::new();
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(CONTEXT);
        let mut last = changed[k];
        while k + 1 < changed.len() && changed[k + 1] - last <= 2 * CONTEXT {
            k += 1;
            last = changed[k];
        }
        let end = (last + CONTEXT).min(ops.len() - 1);
        let (os, ns) = pos[start];
        let oc = ops[start..=end].iter().filter(|op| **op != Op::Ins).count();
        let nc = ops[start..=end].iter().filter(|op| **op != Op::Del).count();
        let shown = |s: usize, c: usize| if c == 0 { s } else { s + 1 };
        out.push_str(&format!("@@ -{},{oc} +{},{nc} @@\n", shown(os, oc), shown(ns, nc)));
        for (idx, op) in ops.iter().enumerate().take(end + 1).skip(start) {
            let (oi, ni) = pos[idx];
            match op {
                Op::Eq => out.push_str(&format!(" {}\n", old[oi])),
                Op::Del => out.push_str(&format!("-{}\n", old[oi])),
                Op::Ins => out.push_str(&format!("+{}\n", new[ni])),
            }
        }
        k += 1;
    }
    out
}

/// Reads a text file for the viewer: `Ok(None)` when it is too large.
fn read_text(path: &Path) -> std::io::Result<Option<String>> {
    let meta = fs::symlink_metadata(path)?;
    if meta.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&fs::read(path)?).into_owned()))
}

/// The redacted, capped unified diff of `changes` (`base` = old tree, `head` = new tree).
/// `scrub` removes configured secret values (see `services::scrub`).
pub fn unified(base: &Path, head: &Path, changes: &ChangeSet, scrub: impl Fn(&str) -> String) -> Value {
    let clean = |text: &str| scanner().redact(&scrub(text));
    let mut files: Vec<_> = changes.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut raw = String::new();
    let mut redactions = 0usize;
    let mut omitted = 0usize;
    for file in &files {
        if raw.len() >= RAW_LIMIT_BYTES {
            omitted += 1;
            continue;
        }
        let p = file.path.display().to_string();
        raw.push_str(&format!("diff --git a/{p} b/{p}\n"));
        let (from, to) = match file.kind {
            ChangeKind::Added => ("/dev/null".to_string(), format!("b/{p}")),
            ChangeKind::Deleted => (format!("a/{p}"), "/dev/null".to_string()),
            ChangeKind::Modified => (format!("a/{p}"), format!("b/{p}")),
        };
        match file.kind {
            ChangeKind::Added => raw.push_str("new file\n"),
            ChangeKind::Deleted => raw.push_str("deleted file\n"),
            ChangeKind::Modified => {}
        }
        raw.push_str(&format!("--- {from}\n+++ {to}\n"));
        if file.is_symlink {
            raw.push_str("(symbolic link changed)\n");
            continue;
        }
        if file.binary {
            raw.push_str("(binary file: contents not shown)\n");
            continue;
        }
        let side = |root: &Path, present: bool| -> std::io::Result<Option<String>> {
            if present {
                read_text(&root.join(&file.path))
            } else {
                Ok(Some(String::new()))
            }
        };
        let old = side(base, file.kind != ChangeKind::Added);
        let new = side(head, file.kind != ChangeKind::Deleted);
        match (old, new) {
            (Ok(Some(old)), Ok(Some(new))) => {
                let (old, r1) = clean(&old);
                let (new, r2) = clean(&new);
                redactions += r1.total() + r2.total();
                let body = hunks(&old, &new);
                if body.is_empty() {
                    raw.push_str(if file.mode_changed { "(mode changed only)\n" } else { "(no line changes)\n" });
                } else {
                    raw.push_str(&body);
                }
            }
            (Ok(None), _) | (_, Ok(None)) => raw.push_str(&format!("(file over {MAX_FILE_BYTES} bytes: lines not shown)\n")),
            (Err(e), _) | (_, Err(e)) => raw.push_str(&format!("(could not read the file: {})\n", e.kind())),
        }
    }
    // Paths and notes go through the same redaction (already-masked values stay masked).
    let (full, report) = clean(&raw);
    redactions += report.total();
    let mut cut = full.len().min(DIFF_CAP_BYTES);
    if cut < full.len() {
        while !full.is_char_boundary(cut) {
            cut -= 1;
        }
        cut = full[..cut].rfind('\n').map(|i| i + 1).unwrap_or(0);
    }
    let text = &full[..cut];
    json!({
        "format": "unified",
        "text": text,
        "bytes": text.len(),
        "bytes_total": full.len(),
        "cap_bytes": DIFF_CAP_BYTES,
        "truncated": cut < full.len() || omitted > 0,
        "files": text.matches("diff --git ").count(),
        "files_total": files.len(),
        "files_omitted": omitted,
        "redactions": redactions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_have_context_and_line_numbers() {
        let old: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 10\n", "line ten\n");
        let h = hunks(&old, &new);
        assert!(h.starts_with("@@ -7,7 +7,7 @@\n line 7\n"), "{h}");
        assert!(h.contains("-line 10\n+line ten\n"), "{h}");
        assert_eq!(h.lines().count(), 1 + 7 + 1);
    }

    #[test]
    fn added_and_removed_files_use_zero_counts() {
        assert_eq!(hunks("", "a\nb\n"), "@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert_eq!(hunks("a\n", ""), "@@ -1,1 +0,0 @@\n-a\n");
        assert_eq!(hunks("same\n", "same\n"), "");
    }

    #[test]
    fn distant_changes_make_separate_hunks() {
        let old: String = (1..=40).map(|i| format!("{i}\n")).collect();
        let new = old.replace("\n5\n", "\nfive\n").replace("\n35\n", "\nthirty-five\n");
        assert_eq!(hunks(&old, &new).matches("@@ -").count(), 2);
    }
}
