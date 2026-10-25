//! Fixtures shared by the crate's unit tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Workspace;

/// Upper bound for every wait in the tests; waits return as soon as their condition holds.
pub(crate) const WAIT: Duration = Duration::from_secs(20);

/// A small live tree under `target/test-fixtures/<name>-<pid>/live` (same APFS volume as the repo).
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
}

impl Fixture {
    pub(crate) fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-fixtures")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("live/src")).unwrap();
        fs::write(root.join("live/src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        fs::write(root.join("live/README.md"), "# demo\n").unwrap();
        let root = fs::canonicalize(&root).unwrap();
        Fixture { root }
    }

    pub(crate) fn live(&self) -> PathBuf {
        self.root.join("live")
    }

    pub(crate) fn workspace(&self) -> Workspace {
        Workspace::open(self.live()).unwrap()
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        snapshot(&self.live())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Relative path -> contents of every file below `root`.
pub(crate) fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    collect(root, root, &mut out);
    out
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
