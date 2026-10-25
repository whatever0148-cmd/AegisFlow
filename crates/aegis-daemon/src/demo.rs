//! The demo workspace: a small self-contained Rust library crate, and the agent edits the workspace
//! scenarios make inside a branch.
//!
//! Every edit is a real change to real files in the branch directory; nothing here measures or
//! reports anything (timing happens in [`crate::workspace`]).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::util::random_alnum;

/// Name the crate is compiled under by the verification command.
pub const CRATE_NAME: &str = "aegis_demo";

/// The pricing function `safe_refactor` renames, in rotation, so repeated runs keep making edits.
const TOTAL_FN_NAMES: [&str; 3] = ["compute_total", "order_total", "total_with_tax"];

/// Sample stock files under `data/fixtures/` (what `mass_delete` removes, with `docs/`). This count
/// was chosen after measuring, so that `mass_delete`'s context window crosses the high-impact
/// threshold; the scenario description says so.
pub const FIXTURES: usize = 40;

const CARGO_TOML: &str = r#"[package]
name = "aegis-demo"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
path = "src/lib.rs"
"#;

const README: &str = "# aegis-demo\n\nA tiny order-pricing library that the Aegis Console scenarios edit inside APFS copy-on-write\nbranches. `data/fixtures/` holds sample stock files and `docs/` the design notes.\n";

const LIB_RS: &str = r#"//! Aegis demo crate: a tiny order-pricing library that the Aegis Console scenarios edit.

pub mod config;
pub mod inventory;
pub mod orders;
pub mod pricing;
pub mod report;

pub use pricing::compute_total;
"#;

const CONFIG_RS: &str = r#"//! Static configuration for the demo crate.

/// Sales tax in basis points (hundredths of a percent).
pub const TAX_BASIS_POINTS: u64 = 825;

/// Currency code printed in reports.
pub const CURRENCY: &str = "USD";
"#;

const PRICING_RS: &str = r#"//! Price arithmetic.

use crate::config::TAX_BASIS_POINTS;

/// Total in cents for `quantity` units at `unit_cents`, tax included.
pub fn compute_total(unit_cents: u64, quantity: u32) -> u64 {
    let net = unit_cents * u64::from(quantity);
    net + net * TAX_BASIS_POINTS / 10_000
}

/// Discount in cents for a percentage between 0 and 100.
pub fn discount(total_cents: u64, percent: u8) -> u64 {
    total_cents * u64::from(percent.min(100)) / 100
}
"#;

const INVENTORY_RS: &str = r#"//! Stock keeping.

use crate::pricing::compute_total;

#[derive(Debug, Clone)]
pub struct Item {
    pub sku: String,
    pub unit_cents: u64,
    pub on_hand: u32,
}

impl Item {
    /// Value of the stock on hand, tax included.
    pub fn stock_value(&self) -> u64 {
        compute_total(self.unit_cents, self.on_hand)
    }
}

pub fn total_stock_value(items: &[Item]) -> u64 {
    items.iter().map(Item::stock_value).sum()
}
"#;

const ORDERS_RS: &str = r#"//! Order lines.

use crate::inventory::Item;
use crate::pricing::{compute_total, discount};

#[derive(Debug, Clone)]
pub struct OrderLine {
    pub item: Item,
    pub quantity: u32,
    pub discount_percent: u8,
}

impl OrderLine {
    /// Amount after discount, tax included.
    pub fn amount(&self) -> u64 {
        let gross = compute_total(self.item.unit_cents, self.quantity);
        gross - discount(gross, self.discount_percent)
    }
}

pub fn order_amount(lines: &[OrderLine]) -> u64 {
    lines.iter().map(OrderLine::amount).sum()
}
"#;

const REPORT_RS: &str = r#"//! Plain-text reports.

use crate::config::CURRENCY;
use crate::orders::{order_amount, OrderLine};
use crate::pricing::compute_total;

pub fn format_cents(cents: u64) -> String {
    format!("{}.{:02} {}", cents / 100, cents % 100, CURRENCY)
}

pub fn order_summary(lines: &[OrderLine]) -> String {
    let mut out = String::new();
    for line in lines {
        let list = compute_total(line.item.unit_cents, line.quantity);
        out.push_str(&format!(
            "{} x{}: {} (list {})\n",
            line.item.sku,
            line.quantity,
            format_cents(line.amount()),
            format_cents(list)
        ));
    }
    out.push_str(&format!("total: {}\n", format_cents(order_amount(lines))));
    out
}
"#;

const APP_TOML: &str = "# Demo service configuration. Credentials do not belong in this file.\n[service]\nname = \"aegis-demo\"\nregion = \"local\"\n";

const DOCS: [(&str, &str); 3] = [
    ("docs/architecture.md", "# Architecture\n\n`pricing` computes totals, `inventory` values stock, `orders` applies discounts and\n`report` prints summaries.\n"),
    ("docs/operations.md", "# Operations\n\nThe crate has no runtime dependencies and builds with a plain `rustc --crate-type lib`.\n"),
    ("docs/pricing.md", "# Pricing\n\nTotals include sales tax, configured in `src/config.rs` in basis points.\n"),
];

/// The line `build_break` rewrites into a call with the wrong number of arguments.
const BREAK_FROM: &str = "format_cents(order_amount(lines))";
const BREAK_TO: &str = "format_cents(order_amount(lines), CURRENCY)";

/// What an agent edit did to the branch.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct EditSummary {
    pub files_modified: usize,
    pub files_added: usize,
    pub files_deleted: usize,
    /// Tool calls the agent made; each is one event in its context window.
    pub tool_calls: usize,
    pub summary: String,
    /// Paths touched by each tool call, in order, when the edit took more than one call.
    #[serde(skip)]
    pub steps: Vec<Vec<PathBuf>>,
}

fn write(root: &Path, rel: &str, body: &str) -> io::Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, body)
}

fn fixture_body(n: usize) -> String {
    format!("sku,unit_cents,on_hand\nSKU-{n:02},{},{}\n", 250 + n * 15, 3 + n % 7)
}

/// Writes the demo crate into the (empty or missing) directory `live`.
pub fn create(live: &Path) -> io::Result<()> {
    fs::create_dir_all(live)?;
    for (rel, body) in [
        ("Cargo.toml", CARGO_TOML),
        ("README.md", README),
        ("src/lib.rs", LIB_RS),
        ("src/config.rs", CONFIG_RS),
        ("src/pricing.rs", PRICING_RS),
        ("src/inventory.rs", INVENTORY_RS),
        ("src/orders.rs", ORDERS_RS),
        ("src/report.rs", REPORT_RS),
        ("config/app.toml", APP_TOML),
    ] {
        write(live, rel, body)?;
    }
    for (rel, body) in DOCS {
        write(live, rel, body)?;
    }
    for n in 1..=FIXTURES {
        write(live, &format!("data/fixtures/sku_{n:02}.csv"), &fixture_body(n))?;
    }
    Ok(())
}

/// True when `live` holds a demo crate this daemon can run scenarios against.
pub fn looks_valid(live: &Path) -> bool {
    live.join("Cargo.toml").is_file() && live.join("src/lib.rs").is_file() && live.join("src/pricing.rs").is_file()
}

/// Regular files under `root`, relative paths, sorted.
pub fn list_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

fn rust_sources(root: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(list_files(root)?.into_iter().filter(|p| p.extension().is_some_and(|e| e == "rs")).collect())
}

/// Replaces whole-identifier occurrences of `from` with `to`.
fn replace_ident(text: &str, from: &str, to: &str) -> (String, usize) {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut rest = text;
    while let Some(pos) = rest.find(from) {
        let before = rest[..pos].chars().next_back();
        let after = rest[pos + from.len()..].chars().next();
        out.push_str(&rest[..pos]);
        if before.is_some_and(is_ident) || after.is_some_and(is_ident) {
            out.push_str(from);
        } else {
            out.push_str(to);
            count += 1;
        }
        rest = &rest[pos + from.len()..];
    }
    out.push_str(rest);
    (out, count)
}

/// The pricing function's current name in `root` (one of the rotation names), if any.
pub fn current_total_fn(root: &Path) -> Option<&'static str> {
    let pricing = fs::read_to_string(root.join("src/pricing.rs")).ok()?;
    TOTAL_FN_NAMES.iter().copied().find(|name| pricing.contains(&format!("pub fn {name}(")))
}

/// `safe_refactor`: renames the pricing function across every module that uses it.
pub fn safe_refactor(root: &Path) -> io::Result<EditSummary> {
    let from = current_total_fn(root)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "src/pricing.rs has no pricing function to rename"))?;
    let idx = TOTAL_FN_NAMES.iter().position(|n| *n == from).unwrap_or(0);
    let to = TOTAL_FN_NAMES[(idx + 1) % TOTAL_FN_NAMES.len()];
    let mut modified = 0;
    let mut occurrences = 0;
    for rel in rust_sources(root)? {
        let path = root.join(&rel);
        let text = fs::read_to_string(&path)?;
        let (new_text, count) = replace_ident(&text, from, to);
        if count > 0 {
            fs::write(&path, new_text)?;
            modified += 1;
            occurrences += count;
        }
    }
    Ok(EditSummary {
        files_modified: modified,
        tool_calls: 1,
        summary: format!("renamed pricing::{from} to pricing::{to} ({occurrences} occurrences in {modified} modules)"),
        ..Default::default()
    })
}

/// `secret_leak`: writes an unformatted, credential-shaped value (random, generated per run) into
/// the service configuration file.
pub fn secret_leak(root: &Path) -> io::Result<EditSummary> {
    let path = root.join("config/app.toml");
    let mut text = fs::read_to_string(&path).unwrap_or_default();
    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&format!("\n[upstream]\nendpoint = \"inventory-sync\"\napi_key = \"{}\"\n", random_alnum(32)));
    fs::write(&path, text)?;
    Ok(EditSummary {
        files_modified: 1,
        tool_calls: 1,
        summary: "added an [upstream] section with a plaintext api_key to config/app.toml".to_string(),
        ..Default::default()
    })
}

/// Files `mass_delete` removes per tool call (fixtures are deleted in batches of this size).
pub const DELETE_BATCH: usize = 8;

/// `mass_delete`: the cleanup agent removes every sample fixture (in batches of
/// [`DELETE_BATCH`], one tool call each), then every design note, then rewrites the README.
pub fn mass_delete(root: &Path) -> io::Result<EditSummary> {
    let files = list_files(root)?;
    let fixtures: Vec<PathBuf> = files.iter().filter(|p| p.starts_with("data")).cloned().collect();
    let docs: Vec<PathBuf> = files.iter().filter(|p| p.starts_with("docs")).cloned().collect();
    let mut steps: Vec<Vec<PathBuf>> = fixtures.chunks(DELETE_BATCH).map(<[PathBuf]>::to_vec).collect();
    if !docs.is_empty() {
        steps.push(docs);
    }
    let mut deleted = 0;
    for step in &steps {
        for rel in step {
            fs::remove_file(root.join(rel))?;
            deleted += 1;
        }
    }
    for dir in ["data/fixtures", "data", "docs"] {
        let _ = fs::remove_dir(root.join(dir));
    }
    write(root, "README.md", "# aegis-demo\n\nSample data and design notes were removed by the cleanup agent.\n")?;
    steps.push(vec![PathBuf::from("README.md")]);
    Ok(EditSummary {
        files_modified: 1,
        files_deleted: deleted,
        tool_calls: steps.len(),
        summary: format!("deleted {deleted} files under data/ and docs/ in {} tool calls and rewrote README.md", steps.len() - 1),
        steps,
        ..Default::default()
    })
}

/// `build_break`: passes an extra argument to `format_cents`, which does not compile.
pub fn build_break(root: &Path) -> io::Result<EditSummary> {
    let path = root.join("src/report.rs");
    let text = fs::read_to_string(&path)?;
    let (new_text, summary) = if text.contains(BREAK_FROM) {
        (text.replacen(BREAK_FROM, BREAK_TO, 1), "report::order_summary now calls format_cents with two arguments".to_string())
    } else {
        (format!("{text}\npub fn grand_total() -> u64 {{\n    \"not a number\"\n}}\n"), "appended report::grand_total returning a string as u64".to_string())
    };
    fs::write(&path, new_text)?;
    Ok(EditSummary { files_modified: 1, tool_calls: 1, summary, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_ident_respects_word_boundaries() {
        let (out, n) = replace_ident("compute_total(x); recompute_total(); compute_totals; crate::compute_total", "compute_total", "order_total");
        assert_eq!(out, "order_total(x); recompute_total(); compute_totals; crate::order_total");
        assert_eq!(n, 2);
    }
}
