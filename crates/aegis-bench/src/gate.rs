//! Checks of a written artifact, run by the harness without re-running the benchmark:
//!
//!   aegis-bench --verify phase1 [--artifact PATH]   T12: the phase-1 sections were measured
//!   aegis-bench --verify gate   [--artifact PATH]   T18: the phase-2 gate (11/11 gated rows)
//!
//! Both require the current schema, a full-scale run and an artifact that still describes the
//! current sources (source fingerprint). The gate also requires the live-tree integrity row and a
//! run that is not preliminary. Pass flags are read from the artifact, never recomputed.

use std::fmt;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::util::{source_fingerprint, Res};
use crate::GATED_ROWS;

pub const SCHEMA: &str = "aegisflow.telemetry.v2";
/// Fixtures every full run measures, in order.
pub const FIXTURE_NAMES: [&str; 4] = ["tiny", "small", "medium", "aegisflow_src"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Phase1,
    Gate,
}

impl Mode {
    pub fn parse(text: &str) -> Res<Self> {
        match text {
            "phase1" => Ok(Self::Phase1),
            "gate" => Ok(Self::Gate),
            other => Err(format!("--verify expects phase1 or gate, got {other}").into()),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Phase1 => "phase1",
            Self::Gate => "gate",
        })
    }
}

#[derive(Debug)]
pub struct Check {
    pub ok: bool,
    pub what: String,
}

fn check(ok: bool, what: impl Into<String>) -> Check {
    Check { ok, what: what.into() }
}

/// A summary row's measurement as text: `measured*` keys, then p95 and sample count.
pub fn row_text(row: &Value) -> String {
    let mut parts: Vec<String> = row
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| k.starts_with("measured"))
        .map(|(k, v)| match v.as_f64() {
            Some(x) if k == "measured_hit_rate" => format!("hit rate {x:.3}"),
            Some(x) if k.ends_with("_ms") => format!("{x:.4} ms"),
            Some(x) if k.ends_with("_us") => format!("{x:.3} us"),
            Some(x) => format!("{x:.4}"),
            None => v.to_string(),
        })
        .collect();
    if let Some(p) = row["p95_ms"].as_f64() {
        parts.push(format!("p95 {p:.4} ms"));
    }
    if let Some(n) = row["n"].as_u64() {
        parts.push(format!("n={n}"));
    }
    parts.join("; ")
}

fn samples(v: &Value) -> u64 {
    v["n"].as_u64().unwrap_or(0)
}

/// Schema and scale.
pub fn common_checks(doc: &Value) -> Vec<Check> {
    let scale = doc["scale"].as_f64();
    vec![
        check(doc["schema"] == SCHEMA, format!("schema {} (expected {SCHEMA})", doc["schema"])),
        check(scale.is_some_and(|s| s >= 1.0), format!("scale {} (a full run has scale >= 1)", doc["scale"])),
    ]
}

/// The artifact was produced from the sources as they are now.
pub fn fresh_checks(doc: &Value, current: &Value) -> Vec<Check> {
    let recorded = doc["sources"]["sha256"].as_str();
    let now = current["sha256"].as_str();
    let short = |v: Option<&str>| v.map(|s| s.chars().take(12).collect::<String>()).unwrap_or_else(|| "missing".into());
    let ok = recorded.is_some() && recorded == now;
    vec![check(
        ok,
        format!(
            "sources fingerprint {} vs current {} over {} files{}",
            short(recorded),
            short(now),
            current["files"],
            match (ok, recorded) {
                (true, _) => "",
                (false, None) => " (not recorded in the artifact; re-run T17)",
                (false, Some(_)) => " (stale: sources changed since the run; re-run T17)",
            }
        ),
    )]
}

/// T12: the sections phase 1 measured are present with samples, and the dashboard exports exist.
pub fn phase1_checks(doc: &Value, artifacts_dir: &Path) -> Vec<Check> {
    let mut out = Vec::new();
    let fixtures: Vec<&Value> = doc["apfs"]["fixtures"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let names: Vec<&str> = fixtures.iter().filter_map(|f| f["name"].as_str()).collect();
    out.push(check(names == FIXTURE_NAMES, format!("APFS fixtures {names:?} (expected {FIXTURE_NAMES:?})")));
    for f in &fixtures {
        let keys = ["clonefile_ms", "renameatx_swap_ms", "rollback_detach_ms", "rollback_reclaim_ms"];
        let counts: Vec<String> = keys.iter().map(|k| format!("{k} n={}", samples(&f[*k]))).collect();
        out.push(check(
            keys.iter().all(|k| samples(&f[*k]) > 0) && f["integrity"]["all_ok"] == true,
            format!("apfs · {}: {} · integrity all_ok {}", f["name"].as_str().unwrap_or("?"), counts.join(", "), f["integrity"]["all_ok"]),
        ));
    }
    let ane = &doc["ane"];
    let units = ["cpu_and_neural_engine", "cpu_only", "all"];
    out.push(check(
        units.iter().all(|u| samples(&ane["runs"][*u]) > 0)
            && ane["compute_plan"]["operations"].as_u64().is_some_and(|n| n > 0)
            && ane["batch"]["compute_plan"]["operations"].as_u64().is_some_and(|n| n > 0)
            && ane["parity"]["vectors"].as_u64().is_some_and(|n| n > 0),
        format!(
            "ane: runs n {:?}, MLComputePlan ops online {} / batch {}, parity vectors {}",
            units.iter().map(|u| samples(&ane["runs"][*u])).collect::<Vec<_>>(),
            ane["compute_plan"]["operations"],
            ane["batch"]["compute_plan"]["operations"],
            ane["parity"]["vectors"]
        ),
    ));
    let cedar = &doc["cedar"];
    out.push(check(
        samples(&cedar["is_authorized_us"]) > 0
            && cedar["decision_matrix_total"].as_u64().is_some_and(|t| t > 0)
            && cedar["decision_matrix_passed"] == cedar["decision_matrix_total"],
        format!(
            "cedar: is_authorized n={}, decision matrix {}/{} as expected",
            samples(&cedar["is_authorized_us"]),
            cedar["decision_matrix_passed"],
            cedar["decision_matrix_total"]
        ),
    ));
    let e2e = &doc["end_to_end"];
    out.push(check(
        samples(&e2e["rollback_path_ms"]) > 0 && samples(&e2e["commit_path_ms"]) > 0 && e2e["live_integrity_verified"] == true,
        format!(
            "end_to_end: rollback path n={}, commit path n={}, live tree byte-identical after rollbacks {}",
            samples(&e2e["rollback_path_ms"]),
            samples(&e2e["commit_path_ms"]),
            e2e["live_integrity_verified"]
        ),
    ));
    let pp = &doc["paypal_receipt"];
    let pp_keys = ["prepare_order_us", "ed25519_sign_us", "ed25519_verify_us"];
    out.push(check(
        pp_keys.iter().all(|k| samples(&pp[*k]) > 0),
        format!("paypal_receipt: {}", pp_keys.iter().map(|k| format!("{k} n={}", samples(&pp[*k]))).collect::<Vec<_>>().join(", ")),
    ));
    let tier0 = samples(&doc["nebius_router"]["tier0_route_us"]);
    out.push(check(tier0 > 0, format!("nebius_router: tier0_route n={tier0}")));
    let mcp = samples(&doc["mcp"]["tools_call_roundtrip_us"]);
    out.push(check(mcp > 0, format!("mcp: tools_call_roundtrip n={mcp}")));
    let rows = doc["targets_summary"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    out.push(check(
        !rows.is_empty() && rows.iter().all(|r| r.get("pass").is_some()),
        format!("targets_summary: {} rows, each with a pass flag", rows.len()),
    ));
    for name in ["paypal_aggrid_rows.json", "paypal_bryntum_store.json"] {
        let path = artifacts_dir.join(name);
        let parsed = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let ok = parsed.as_ref().is_some_and(|v| !v.is_null());
        out.push(check(ok, format!("{} {}", path.display(), if ok { "present" } else { "missing or not JSON" })));
    }
    out
}

/// T18: every gated row passed, the live-tree integrity row passed, the run is not preliminary.
/// Returns one printable line per gated row plus the checks.
pub fn gate_checks(doc: &Value) -> (Vec<String>, Vec<Check>) {
    let rows = doc["targets_summary"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut lines = Vec::new();
    for r in rows {
        lines.push(format!(
            "{} | {} | target {} | measured {}",
            if r["pass"] == true { "PASS" } else { "FAIL" },
            r["metric"].as_str().unwrap_or("?"),
            r["target"].as_str().unwrap_or("?"),
            row_text(r)
        ));
    }
    let failed: Vec<&str> = rows.iter().filter(|r| r["pass"] != true).map(|r| r["metric"].as_str().unwrap_or("?")).collect();
    let passed = rows.len() - failed.len();
    let mut checks = vec![
        check(rows.len() == GATED_ROWS, format!("{} gated rows (expected exactly {GATED_ROWS})", rows.len())),
        check(
            !rows.is_empty() && failed.is_empty(),
            format!("gate: {passed} of {} gated rows passed{}", rows.len(), if failed.is_empty() { String::new() } else { format!("; missed: {}", failed.join(" | ")) }),
        ),
    ];
    let integrity: Vec<&Value> =
        doc["informational"].as_array().into_iter().flatten().filter(|r| r["section"] == "integrity").collect();
    checks.push(check(
        !integrity.is_empty() && integrity.iter().all(|r| r["pass"] == true),
        format!(
            "live-tree integrity: {}",
            integrity.iter().map(|r| r["measured"].to_string()).collect::<Vec<_>>().join("; ").chars().take(200).collect::<String>()
        ),
    ));
    let run = &doc["run_conditions"];
    let reasons: Vec<&str> = run["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    checks.push(check(
        run["preliminary"] == false,
        match run["preliminary"].as_bool() {
            Some(false) => "run conditions: quiet full-scale run".to_string(),
            Some(true) => format!("run conditions: PRELIMINARY ({}); only a quiet full-scale run decides the phase result", reasons.join("; ")),
            None => "run conditions not recorded".to_string(),
        },
    ));
    (lines, checks)
}

/// Runs the checks of `mode` on `artifact` and prints them; Err when any check failed.
pub fn verify(mode: Mode, artifact: &Path, root: &Path) -> Res<()> {
    let text = fs::read_to_string(artifact).map_err(|e| format!("{}: {e}", artifact.display()))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", artifact.display()))?;
    let current = source_fingerprint(root)?;
    println!(
        "artifact {} · schema {} · scale {} · generated {} · commit {}{} · preliminary {}",
        artifact.display(),
        doc["schema"],
        doc["scale"],
        doc["generated_at"],
        doc["git"]["commit"].as_str().unwrap_or("?"),
        if doc["git"]["dirty"] == true { " (dirty)" } else { "" },
        doc["run_conditions"]["preliminary"]
    );
    let mut checks = common_checks(&doc);
    checks.extend(fresh_checks(&doc, &current));
    match mode {
        Mode::Phase1 => checks.extend(phase1_checks(&doc, artifact.parent().unwrap_or(root))),
        Mode::Gate => {
            let (lines, gate) = gate_checks(&doc);
            for line in lines {
                println!("{line}");
            }
            checks.extend(gate);
        }
    }
    for c in &checks {
        println!("{} {}", if c.ok { "ok  " } else { "FAIL" }, c.what);
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    if failed == 0 {
        println!("{mode}: all {} checks passed", checks.len());
        Ok(())
    } else {
        Err(format!("{mode}: {failed} of {} checks failed", checks.len()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn passing_doc() -> Value {
        let rows: Vec<Value> = (0..GATED_ROWS).map(|i| json!({"metric": format!("row {i}"), "target": "t", "pass": true})).collect();
        json!({
            "schema": SCHEMA,
            "scale": 1.0,
            "sources": {"sha256": "abc"},
            "targets_summary": rows,
            "informational": [{"metric": "live-tree integrity", "section": "integrity", "measured": {}, "pass": true}],
            "run_conditions": {"preliminary": false, "reasons": []},
        })
    }

    fn all_ok(checks: &[Check]) -> bool {
        checks.iter().all(|c| c.ok)
    }

    #[test]
    fn gate_accepts_a_quiet_full_run_with_eleven_passing_rows() {
        let doc = passing_doc();
        let (lines, checks) = gate_checks(&doc);
        assert_eq!(lines.len(), GATED_ROWS);
        assert!(all_ok(&checks), "{checks:?}");
        assert!(all_ok(&common_checks(&doc)));
        assert!(all_ok(&fresh_checks(&doc, &json!({"sha256": "abc", "files": 1}))));
    }

    #[test]
    fn gate_rejects_missed_rows_integrity_failures_and_preliminary_runs() {
        let mut missed = passing_doc();
        missed["targets_summary"][3]["pass"] = json!(false);
        let mut null_pass = passing_doc();
        null_pass["targets_summary"][0]["pass"] = Value::Null;
        let mut ten = passing_doc();
        ten["targets_summary"].as_array_mut().unwrap().pop();
        let mut integrity = passing_doc();
        integrity["informational"][0]["pass"] = json!(false);
        let mut no_integrity = passing_doc();
        no_integrity["informational"] = json!([]);
        let mut preliminary = passing_doc();
        preliminary["run_conditions"] = json!({"preliminary": true, "reasons": ["busy"]});
        let mut unrecorded = passing_doc();
        unrecorded.as_object_mut().unwrap().remove("run_conditions");
        for (name, doc) in [
            ("missed", missed),
            ("null pass", null_pass),
            ("ten rows", ten),
            ("integrity", integrity),
            ("no integrity row", no_integrity),
            ("preliminary", preliminary),
            ("unrecorded", unrecorded),
        ] {
            let (_, checks) = gate_checks(&doc);
            assert!(!all_ok(&checks), "{name} must fail the gate");
        }
    }

    #[test]
    fn reduced_scale_old_schema_and_stale_sources_fail() {
        let mut reduced = passing_doc();
        reduced["scale"] = json!(0.1);
        assert!(!all_ok(&common_checks(&reduced)));
        let mut old = passing_doc();
        old["schema"] = json!("aegisflow.phase1.telemetry.v1");
        assert!(!all_ok(&common_checks(&old)));
        let doc = passing_doc();
        assert!(!all_ok(&fresh_checks(&doc, &json!({"sha256": "def", "files": 1}))));
        let mut unrecorded = passing_doc();
        unrecorded.as_object_mut().unwrap().remove("sources");
        assert!(!all_ok(&fresh_checks(&unrecorded, &json!({"sha256": "abc", "files": 1}))));
    }

    #[test]
    fn phase1_requires_every_section_with_samples() {
        let dir = std::env::temp_dir().join(format!("aegis-bench-gate-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let n = |count: u64| json!({"n": count});
        let fixture = |name: &str| {
            json!({"name": name, "clonefile_ms": n(5), "renameatx_swap_ms": n(5), "rollback_detach_ms": n(5),
                   "rollback_reclaim_ms": n(5), "integrity": {"all_ok": true}})
        };
        let mut doc = passing_doc();
        doc["apfs"] = json!({"fixtures": FIXTURE_NAMES.iter().map(|f| fixture(f)).collect::<Vec<_>>()});
        doc["ane"] = json!({"runs": {"cpu_and_neural_engine": n(5), "cpu_only": n(5), "all": n(5)},
                            "compute_plan": {"operations": 3}, "batch": {"compute_plan": {"operations": 3}}, "parity": {"vectors": 4}});
        doc["cedar"] = json!({"is_authorized_us": n(5), "decision_matrix_passed": 10, "decision_matrix_total": 10});
        doc["end_to_end"] = json!({"rollback_path_ms": n(5), "commit_path_ms": n(5), "live_integrity_verified": true});
        doc["paypal_receipt"] = json!({"prepare_order_us": n(5), "ed25519_sign_us": n(5), "ed25519_verify_us": n(5)});
        doc["nebius_router"] = json!({"tier0_route_us": n(5)});
        doc["mcp"] = json!({"tools_call_roundtrip_us": n(5)});
        for name in ["paypal_aggrid_rows.json", "paypal_bryntum_store.json"] {
            fs::write(dir.join(name), "[]").unwrap();
        }
        assert!(all_ok(&phase1_checks(&doc, &dir)), "{:?}", phase1_checks(&doc, &dir));

        let mut no_mcp = doc.clone();
        no_mcp["mcp"]["tools_call_roundtrip_us"] = n(0);
        assert!(!all_ok(&phase1_checks(&no_mcp, &dir)));
        let mut three_fixtures = doc.clone();
        three_fixtures["apfs"]["fixtures"].as_array_mut().unwrap().pop();
        assert!(!all_ok(&phase1_checks(&three_fixtures, &dir)));
        let mut matrix = doc.clone();
        matrix["cedar"]["decision_matrix_passed"] = json!(9);
        assert!(!all_ok(&phase1_checks(&matrix, &dir)));
        fs::remove_file(dir.join("paypal_bryntum_store.json")).unwrap();
        assert!(!all_ok(&phase1_checks(&doc, &dir)));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn source_fingerprint_follows_file_contents() {
        // A directory outside any git work tree exercises the directory-walk fallback.
        let root = std::env::temp_dir().join(format!("aegis-bench-fp-{}", std::process::id()));
        let src = root.join("crates/demo/src");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(root.join("crates/demo/target")).unwrap();
        fs::write(src.join("lib.rs"), "pub fn a() {}\n").unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        let first = source_fingerprint(&root).unwrap();
        assert_eq!(first["files"], 2);
        fs::write(root.join("crates/demo/target/out.bin"), "build output").unwrap();
        assert_eq!(source_fingerprint(&root).unwrap()["sha256"], first["sha256"], "build output is not a source");
        fs::write(src.join("lib.rs"), "pub fn b() {}\n").unwrap();
        assert_ne!(source_fingerprint(&root).unwrap()["sha256"], first["sha256"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn row_text_shows_hit_rate_p95_and_n() {
        let (median, tail) = (0.25, 0.5);
        let row = json!({"measured_p50_ms": median, "measured_hit_rate": 1.0, "p95_ms": tail, "n": 7});
        let text = row_text(&row);
        for part in ["0.2500 ms", "hit rate 1.000", "p95 0.5000 ms", "n=7"] {
            assert!(text.contains(part), "{text} lacks {part}");
        }
    }
}
