//! AegisFlow phase-1 hardware benchmark.
//!
//! Measures, on this Mac: clonefile branching, renameatx_np(RENAME_SWAP) commits and rollbacks on
//! APFS fixtures of increasing size; CoreML FP16 inference per compute-unit setting with the
//! MLComputePlan placement report; Cedar evaluation; full branch→score→govern→rollback/commit
//! transactions; PayPal order preparation with ed25519 receipts; tier-0 recovery routing; and MCP
//! tool round-trips. Results (with sample counts and percentiles, missed targets included) go to
//! `artifacts/aegis_phase1_telemetry.json`.
//!
//!   cargo run --release -p aegis-bench -- [--out PATH] [--quick]
//!
//! Remote services are never called unless `AEGIS_BENCH_LIVE=1` and their credentials are set.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adapter_nebius_nemotron::{BuildFailure, RecoveryRouter, RouterConfig, Tier};
use adapter_paypal::{aggrid_export, bryntum_export, telemetry_row, LineItem, OrderIntent, PayPalConfig, PayPalMiddleware, Submission, TelemetryRow};
use adapter_switch_mcp::mcp::tool_definitions;
use adapter_switch_mcp::{ChatConfig, McpServer};
use aegis_ane_scorer::{
    compute_plan, default_batch_model_path, default_model_path, neural_engine_cores, workspace_features, AneScorer,
    ComputePlanReport, ComputeUnits, ExecutionContext, Feature, FeatureVector,
};
use aegis_governance::receipt::verify_json;
use aegis_governance::time::utc_timestamp;
use aegis_governance::{risk_to_context, Action, AuthzRequest, GovernanceEngine, PolicyContext, ReceiptSigner};
use aegis_storage::Workspace;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type Res<T> = Result<T, Box<dyn Error>>;

const TARGET_CLONEFILE_BRANCH_MS: f64 = 0.5;
const TARGET_RENAMEATX_SWAP_MS: f64 = 0.5;
const TARGET_ROLLBACK_MS: f64 = 0.5;
const TARGET_ANE_INFERENCE_MS: f64 = 0.6;
const TARGET_TIER0_ROUTE_MS: f64 = 0.4;

struct FixtureSpec {
    name: &'static str,
    dirs: usize,
    files_per_dir: usize,
    min_bytes: usize,
    max_bytes: usize,
    iterations: usize,
}

const FIXTURES: [FixtureSpec; 3] = [
    FixtureSpec { name: "tiny", dirs: 2, files_per_dir: 4, min_bytes: 512, max_bytes: 4096, iterations: 400 },
    FixtureSpec { name: "small", dirs: 8, files_per_dir: 16, min_bytes: 1024, max_bytes: 16384, iterations: 300 },
    FixtureSpec { name: "medium", dirs: 32, files_per_dir: 32, min_bytes: 1024, max_bytes: 16384, iterations: 60 },
];

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// Nearest-rank percentile summary.
fn stats(mut samples: Vec<f64>) -> Value {
    if samples.is_empty() {
        return json!({"n": 0});
    }
    samples.sort_by(|a, b| a.total_cmp(b));
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

fn p50(v: &Value) -> Option<f64> {
    v["p50"].as_f64()
}

fn scaled(n: usize, scale: f64) -> usize {
    ((n as f64 * scale).round() as usize).max(5)
}

fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn host_info(root: &Path) -> Value {
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
    })
}

fn git_info(root: &Path) -> Value {
    let commit = command_line("git", &["-C", &root.to_string_lossy(), "rev-parse", "--short", "HEAD"]);
    let dirty = Command::new("git")
        .args(["-C", &root.to_string_lossy(), "status", "--porcelain", "--", "crates", "policies", "models", "Cargo.toml", "Cargo.lock"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(true);
    json!({"commit": commit.unwrap_or_else(|| "uncommitted".to_string()), "dirty": dirty || commit_is_missing(root)})
}

fn commit_is_missing(root: &Path) -> bool {
    command_line("git", &["-C", &root.to_string_lossy(), "rev-parse", "HEAD"]).is_none()
}

/// Deterministic xorshift for fixture contents.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() as usize) % (hi - lo + 1)
    }
}

fn generate_fixture(live: &Path, spec: &FixtureSpec) -> Res<()> {
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
fn tree_size(root: &Path) -> Res<(usize, usize, u64)> {
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

/// SHA-256 over (relative path, contents) of every file, in sorted order.
fn tree_hash(root: &Path) -> Res<String> {
    let mut paths = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if fs::symlink_metadata(&path)?.is_dir() {
                stack.push(path);
            } else {
                paths.push(path);
            }
        }
    }
    paths.sort();
    let mut hasher = Sha256::new();
    for path in paths {
        hasher.update(path.strip_prefix(root)?.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(&path)?);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn bench_fixture(name: &str, live: &Path, iterations: usize) -> Res<Value> {
    let mut ws = Workspace::open(live)?;
    ws.set_retained_versions(4);
    let (files, dirs, bytes) = tree_size(live)?;
    for _ in 0..5 {
        ws.branch("warmup")?.rollback()?;
    }

    let before = tree_hash(live)?;
    let (mut clone, mut branch_total, mut detach, mut reclaim) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..iterations {
        let started = Instant::now();
        let branch = ws.branch("bench")?;
        branch_total.push(ms(started.elapsed()));
        clone.push(ms(branch.clone_duration()));
        fs::write(branch.path().join("agent_edit.rs"), format!("// edit {i}\n"))?;
        let receipt = branch.rollback()?;
        detach.push(ms(receipt.detach));
        reclaim.push(ms(receipt.reclaim));
    }
    let unchanged = tree_hash(live)? == before;

    let (mut swap, mut commit_total) = (Vec::new(), Vec::new());
    for i in 0..iterations {
        let branch = ws.branch("bench-commit")?;
        fs::write(branch.path().join("agent_commit.rs"), format!("// commit {i}\n"))?;
        let receipt = branch.commit()?;
        swap.push(ms(receipt.swap));
        commit_total.push(ms(receipt.total));
    }
    Ok(json!({
        "name": name,
        "files": files,
        "dirs": dirs,
        "entries": files + dirs,
        "bytes": bytes,
        "iterations": iterations,
        "clonefile_ms": stats(clone),
        "branch_total_ms": stats(branch_total),
        "renameatx_swap_ms": stats(swap),
        "commit_total_ms": stats(commit_total),
        "rollback_detach_ms": stats(detach),
        "rollback_reclaim_ms": stats(reclaim),
        "live_tree_unchanged_after_rollback": unchanged,
    }))
}

fn bench_apfs(fixtures: &Path, root: &Path, scale: f64) -> Res<Value> {
    let mut results = Vec::new();
    for spec in &FIXTURES {
        let live = fixtures.join(spec.name).join("live");
        fs::create_dir_all(&live)?;
        generate_fixture(&live, spec)?;
        eprintln!("  apfs · {} ({} files)", spec.name, spec.dirs * spec.files_per_dir + 1);
        results.push(bench_fixture(spec.name, &live, scaled(spec.iterations, scale))?);
    }
    // A copy-on-write copy of this repository's own sources as a realistic tree.
    let src_live = fixtures.join("aegisflow_src").join("live");
    fs::create_dir_all(&src_live)?;
    for dir in ["crates", "policies", "models", "harness", ".claude", ".kiro"] {
        if root.join(dir).exists() {
            let status = Command::new("cp").args(["-Rc"]).arg(root.join(dir)).arg(&src_live).status()?;
            if !status.success() {
                return Err(format!("cp -Rc {dir} failed").into());
            }
        }
    }
    eprintln!("  apfs · aegisflow_src");
    results.push(bench_fixture("aegisflow_src", &src_live, scaled(200, scale))?);

    // Least-squares fit of clonefile p50 (µs) against tree entries.
    let points: Vec<(f64, f64)> = results
        .iter()
        .filter_map(|r| Some((r["entries"].as_f64()?, p50(&r["clonefile_ms"])? * 1000.0)))
        .collect();
    let n = points.len() as f64;
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x, b + y));
    let (sxx, sxy) = points.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x * x, b + x * y));
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    let intercept = (sy - slope * sx) / n;
    let fs_type = command_line("stat", &["-f", "%T", &fixtures.to_string_lossy()]);
    Ok(json!({
        "filesystem": "apfs",
        "stat_type": fs_type,
        "fixtures": results,
        "clonefile_us_per_entry": round6(slope),
        "clonefile_fixed_us": round6(intercept),
    }))
}

fn load_parity(root: &Path) -> Res<(Vec<FeatureVector>, Vec<f64>)> {
    let doc: Value = serde_json::from_str(&fs::read_to_string(root.join("models/parity_vectors.json"))?)?;
    let vectors = doc["vectors"]
        .as_array()
        .ok_or("parity vectors missing")?
        .iter()
        .map(|row| {
            let mut v = FeatureVector::default();
            for (slot, x) in v.0.iter_mut().zip(row.as_array().into_iter().flatten()) {
                *slot = x.as_f64().unwrap_or(0.0) as f32;
            }
            v
        })
        .collect();
    let reference = doc["reference_fp32"].as_array().ok_or("reference missing")?.iter().filter_map(Value::as_f64).collect();
    Ok((vectors, reference))
}

fn plan_summary(plan: &ComputePlanReport) -> Value {
    json!({
        "compute_units": plan.compute_units,
        "operations": plan.operations_dispatched,
        "operations_total": plan.operations_total,
        "preferred": {
            "neural_engine": plan.preferred.get("neural_engine").copied().unwrap_or(0),
            "cpu": plan.preferred.get("cpu").copied().unwrap_or(0),
            "gpu": plan.preferred.get("gpu").copied().unwrap_or(0),
        },
        "ops": plan.ops.iter().filter(|o| o.preferred != "none").map(|o| json!({
            "operator": o.operator, "preferred": o.preferred, "supported": o.supported,
        })).collect::<Vec<_>>(),
    })
}

fn bench_ane(root: &Path, scale: f64) -> Res<Value> {
    let (vectors, reference) = load_parity(root)?;
    let mut runs = json!({});
    for (key, units) in [
        ("cpu_and_neural_engine", ComputeUnits::CpuAndNeuralEngine),
        ("cpu_only", ComputeUnits::CpuOnly),
        ("all", ComputeUnits::All),
    ] {
        let scorer = AneScorer::open(&default_model_path(), units)?;
        for v in vectors.iter().cycle().take(200) {
            scorer.score(v)?;
        }
        let mut samples = Vec::new();
        for v in vectors.iter().cycle().take(scaled(3000, scale)) {
            let started = Instant::now();
            scorer.score(v)?;
            samples.push(ms(started.elapsed()));
        }
        runs[key] = stats(samples);
    }

    let online = AneScorer::open_default()?;
    let diffs: Vec<f64> = vectors
        .iter()
        .zip(&reference)
        .map(|(v, r)| online.score(v).map(|s| (f64::from(s) - r).abs()))
        .collect::<Result<_, _>>()?;
    let online_plan = compute_plan(&default_model_path(), ComputeUnits::CpuAndNeuralEngine)?;

    let batch_plan = compute_plan(&default_batch_model_path(), ComputeUnits::CpuAndNeuralEngine)?;
    let mut batch_runs = json!({});
    let mut batch_size = 0;
    let mut batch_diff = 0.0f64;
    for (key, units) in [("cpu_and_neural_engine", ComputeUnits::CpuAndNeuralEngine), ("cpu_only", ComputeUnits::CpuOnly)] {
        let scorer = AneScorer::open(&default_batch_model_path(), units)?;
        batch_size = scorer.batch_size();
        let full: Vec<FeatureVector> = vectors.iter().cycle().take(batch_size).copied().collect();
        for _ in 0..3 {
            scorer.score_batch(&full)?;
        }
        let mut samples = Vec::new();
        for _ in 0..scaled(40, scale) {
            let started = Instant::now();
            scorer.score_batch(&full)?;
            samples.push(ms(started.elapsed()));
        }
        let summary = stats(samples);
        let per_op_us = p50(&summary).map(|p| round6(p * 1000.0 / batch_size as f64));
        batch_runs[key] = json!({"batch_ms": summary, "amortized_us_per_operation_p50": per_op_us});
        if units == ComputeUnits::CpuAndNeuralEngine {
            let scores = scorer.score_batch(&vectors)?;
            batch_diff = scores.iter().zip(&reference).map(|(s, r)| (f64::from(*s) - r).abs()).fold(0.0, f64::max);
        }
    }
    let note = format!(
        "MLComputePlan (.cpuAndNeuralEngine) placed online-model ops as ANE {} / CPU {} / GPU {} of {}; batch model ({} operations per prediction) as ANE {} / CPU {} / GPU {} of {}.",
        online_plan.on_neural_engine(),
        online_plan.preferred.get("cpu").copied().unwrap_or(0),
        online_plan.preferred.get("gpu").copied().unwrap_or(0),
        online_plan.operations_dispatched,
        batch_size,
        batch_plan.on_neural_engine(),
        batch_plan.preferred.get("cpu").copied().unwrap_or(0),
        batch_plan.preferred.get("gpu").copied().unwrap_or(0),
        batch_plan.operations_dispatched,
    );
    Ok(json!({
        "model": "models/AegisWorkflowFP16.mlmodelc",
        "precision": "float16",
        "feature_count": online.feature_count(),
        "neural_engine_cores": neural_engine_cores()?,
        "runs": runs,
        "compute_plan": plan_summary(&online_plan),
        "parity": {
            "vectors": diffs.len(),
            "max_abs_diff": round6(diffs.iter().copied().fold(0.0, f64::max)),
            "mean_abs_diff": round6(diffs.iter().sum::<f64>() / diffs.len() as f64),
            "reference": "NumPy FP32 forward pass of the same weights (models/parity_vectors.json)",
        },
        "batch": {
            "model": "models/AegisWorkflowFP16Batch.mlmodelc",
            "batch_size": batch_size,
            "compute_plan": plan_summary(&batch_plan),
            "runs": batch_runs,
            "parity_max_abs_diff": round6(batch_diff),
        },
        "placement_note": note,
    }))
}

struct CedarCase {
    name: &'static str,
    action: Action,
    context: PolicyContext,
    expect_allow: bool,
}

fn cedar_cases() -> Vec<CedarCase> {
    let ctx = |risk: i64, sensitive: bool, approval: bool, receipt: bool, cents: i64| PolicyContext {
        ane_risk_score: risk,
        has_unformatted_sensitive_fields: sensitive,
        team_approval_granted: approval,
        ed25519_receipt_verified: receipt,
        amount_cents: cents,
    };
    vec![
        CedarCase { name: "speculative commit, risk 12", action: Action::SpeculativeCommit, context: ctx(12, false, false, false, 0), expect_allow: true },
        CedarCase { name: "commit with unredacted sensitive fields, risk 5", action: Action::SpeculativeCommit, context: ctx(5, true, false, false, 0), expect_allow: false },
        CedarCase { name: "high-impact commit without approval, risk 61", action: Action::SpeculativeCommit, context: ctx(61, false, false, false, 0), expect_allow: false },
        CedarCase { name: "high-impact commit with team approval, risk 61", action: Action::SpeculativeCommit, context: ctx(61, false, true, false, 0), expect_allow: true },
        CedarCase { name: "order $42.00, risk 20", action: Action::CreateOrder, context: ctx(20, false, false, false, 4200), expect_allow: true },
        CedarCase { name: "order $120.00, receipt unverified, risk 20", action: Action::CreateOrder, context: ctx(20, false, false, false, 12000), expect_allow: false },
        CedarCase { name: "order $120.00, receipt verified, risk 20", action: Action::CreateOrder, context: ctx(20, false, false, true, 12000), expect_allow: true },
        CedarCase { name: "order $20.00, risk 80", action: Action::CreateOrder, context: ctx(80, false, false, true, 2000), expect_allow: false },
        CedarCase { name: "branch, risk 99 with sensitive fields", action: Action::Branch, context: ctx(99, true, false, false, 0), expect_allow: true },
        CedarCase { name: "rollback, risk 99", action: Action::Rollback, context: ctx(99, false, false, false, 0), expect_allow: true },
    ]
}

fn authz(case: &CedarCase) -> AuthzRequest {
    AuthzRequest {
        principal: "core-builder".to_string(),
        action: case.action,
        resource: if case.action == Action::CreateOrder { "sandbox-merchant" } else { "aegisflow" }.to_string(),
        context: case.context.clone(),
    }
}

fn bench_cedar(gov: &GovernanceEngine, scale: f64) -> Res<Value> {
    let cases = cedar_cases();
    let mut matrix = Vec::new();
    let mut passed = 0;
    for case in &cases {
        let d = gov.authorize(&authz(case))?;
        passed += usize::from(d.allowed == case.expect_allow);
        matrix.push(json!({
            "case": case.name,
            "expected": if case.expect_allow { "Allow" } else { "Deny" },
            "actual": if d.allowed { "Allow" } else { "Deny" },
            "policies": d.determining_policies,
        }));
    }
    let requests: Vec<_> = cases.iter().map(|c| gov.build_request(&authz(c))).collect::<Result<_, _>>()?;
    for r in requests.iter().cycle().take(2000) {
        gov.authorize_request(r);
    }
    let eval: Vec<f64> = requests.iter().cycle().take(scaled(20_000, scale)).map(|r| gov.authorize_request(r).eval_ns as f64 / 1000.0).collect();
    let mut full = Vec::new();
    for case in cases.iter().cycle().take(scaled(20_000, scale)) {
        let started = Instant::now();
        gov.authorize(&authz(case))?;
        full.push(us(started.elapsed()));
    }
    Ok(json!({
        "policies": gov.policy_count(),
        "schema_validated": true,
        "is_authorized_us": stats(eval),
        "build_request_and_authorize_us": stats(full),
        "decision_matrix": matrix,
        "decision_matrix_passed": passed,
        "decision_matrix_total": cases.len(),
    }))
}

fn bench_end_to_end(fixtures: &Path, scorer: &AneScorer, gov: &GovernanceEngine, scale: f64) -> Res<Value> {
    let live = fixtures.join("e2e").join("live");
    fs::create_dir_all(&live)?;
    generate_fixture(&live, &FIXTURES[1])?;
    fs::create_dir_all(live.join("docs"))?;
    fs::write(live.join("docs/guide.md"), "# Guide\n")?;
    fs::write(live.join("src/payments.rs"), "pub fn charge(cents: u64) -> u64 { cents }\n")?;
    let ws = Workspace::open(&live)?;
    let (files, _, _) = tree_size(&live)?;
    let iterations = scaled(200, scale);
    let key = format!("AKIA{}", "IOSFODNN7EXAMPLE");
    let before = tree_hash(&live)?;

    let mut totals = Vec::new();
    let mut stages: Vec<(&str, Vec<f64>)> =
        ["branch", "agent_edit", "diff", "features", "ane_score", "cedar", "rollback"].iter().map(|s| (*s, Vec::new())).collect();
    let (mut denied, mut risky_scores) = (0usize, Vec::new());
    for i in 0..iterations {
        let t0 = Instant::now();
        let mut lap = Instant::now();
        let mark = |idx: usize, stages: &mut Vec<(&str, Vec<f64>)>, lap: &mut Instant| {
            stages[idx].1.push(us(lap.elapsed()));
            *lap = Instant::now();
        };
        let branch = ws.branch("agent")?;
        mark(0, &mut stages, &mut lap);
        fs::write(
            branch.path().join("src/payments.rs"),
            format!("pub fn charge(cents: u64) -> u64 {{ cents }}\nconst AWS_KEY: &str = \"{key}\"; // edit {i}\npub fn notify() {{ send(\"billing@example.com\"); }}\n"),
        )?;
        fs::write(branch.path().join(".env"), format!("AWS_ACCESS_KEY_ID={key}\n"))?;
        mark(1, &mut stages, &mut lap);
        let changes = branch.changes()?;
        mark(2, &mut stages, &mut lap);
        let features = workspace_features(&changes, &ExecutionContext { agent_uncertainty: 0.3, ..Default::default() });
        mark(3, &mut stages, &mut lap);
        let risk = scorer.score(&features)?;
        mark(4, &mut stages, &mut lap);
        let decision = gov.authorize(&AuthzRequest {
            principal: "core-builder".into(),
            action: Action::SpeculativeCommit,
            resource: "e2e".into(),
            context: PolicyContext {
                ane_risk_score: risk_to_context(risk),
                has_unformatted_sensitive_fields: features.get(Feature::UnformattedSensitiveFields) > 0.5,
                ..Default::default()
            },
        })?;
        mark(5, &mut stages, &mut lap);
        denied += usize::from(!decision.allowed);
        risky_scores.push(f64::from(risk));
        if decision.allowed {
            return Err("risky edit was allowed by Cedar; end-to-end rollback scenario is invalid".into());
        }
        branch.rollback()?;
        mark(6, &mut stages, &mut lap);
        totals.push(ms(t0.elapsed()));
    }
    let integrity = tree_hash(&live)? == before;

    let mut commit_totals = Vec::new();
    let (mut allowed, mut benign_scores) = (0usize, Vec::new());
    for i in 0..iterations {
        let t0 = Instant::now();
        let branch = ws.branch("docs")?;
        let guide = branch.path().join("docs/guide.md");
        let mut text = fs::read_to_string(&guide)?;
        text.push_str(&format!("\nRevision {i}: clarified the setup steps.\n"));
        fs::write(&guide, text)?;
        let changes = branch.changes()?;
        let features = workspace_features(&changes, &ExecutionContext { agent_uncertainty: 0.1, ..Default::default() });
        let risk = scorer.score(&features)?;
        let decision = gov.authorize(&AuthzRequest {
            principal: "core-builder".into(),
            action: Action::SpeculativeCommit,
            resource: "e2e".into(),
            context: PolicyContext { ane_risk_score: risk_to_context(risk), ..Default::default() },
        })?;
        benign_scores.push(f64::from(risk));
        if decision.allowed {
            allowed += 1;
            branch.commit()?;
        } else {
            branch.rollback()?;
        }
        commit_totals.push(ms(t0.elapsed()));
    }
    let stage_p50: serde_json::Map<String, Value> =
        stages.into_iter().map(|(name, s)| (name.to_string(), stats(s)["p50"].clone())).collect();
    Ok(json!({
        "fixture_files": files,
        "iterations": iterations,
        "rollback_path_ms": stats(totals),
        "commit_path_ms": stats(commit_totals),
        "stage_p50_us": stage_p50,
        "live_integrity_verified": integrity,
        "rollback_case": {"denied": denied, "risk_score": stats(risky_scores)},
        "commit_case": {"allowed": allowed, "risk_score": stats(benign_scores)},
    }))
}

fn order(reference: &str, quantity: u32, unit_cents: u64, note: &str, new_payee: bool, country_risk: f64) -> OrderIntent {
    OrderIntent {
        reference: reference.to_string(),
        merchant_id: "sandbox-lab-supplies".to_string(),
        currency: "USD".to_string(),
        description: "Build-lab hardware for CI runners".to_string(),
        customer_note: note.to_string(),
        items: vec![LineItem { name: "Thunderbolt 4 cable".into(), quantity, unit_amount_cents: unit_cents, sku: Some("TB4-1M".into()) }],
        new_payee,
        payee_country_risk: country_risk,
        customer_pii_fields: 1,
        team_approval_granted: false,
    }
}

fn bench_paypal(scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>, scale: f64) -> Res<(Value, Vec<TelemetryRow>)> {
    let env_cfg = PayPalConfig::from_env();
    let live = std::env::var("AEGIS_BENCH_LIVE").as_deref() == Ok("1") && env_cfg.has_credentials();
    let mw = PayPalMiddleware::new(env_cfg.clone(), gov.clone(), scorer.clone(), ReceiptSigner::generate());
    let unredacted = PayPalMiddleware::new(PayPalConfig { redact: false, ..env_cfg.clone() }, gov, scorer, ReceiptSigner::generate());
    let cases: Vec<(&str, &PayPalMiddleware, OrderIntent)> = vec![
        ("$42.00 known payee", &mw, order("bench-042", 2, 2100, "Deliver to jane.doe@example.com, call +1 (415) 555-0132", false, 0.05)),
        ("$120.00 known payee", &mw, order("bench-120", 2, 6000, "Ship with the next batch", false, 0.05)),
        ("$480.00 new payee, high-risk country", &mw, order("bench-480", 8, 6000, "Urgent", true, 0.9)),
        ("$25.00 with redaction disabled", &unredacted, order("bench-pii", 1, 2500, "Card 4111 1111 1111 1111", false, 0.05)),
    ];
    let mut rows = Vec::new();
    let mut summaries = Vec::new();
    let mut live_result = Value::Null;
    for (name, middleware, intent) in &cases {
        let prepared = middleware.prepare(intent)?;
        let submission = if !prepared.decision.allowed {
            middleware.submit(&prepared)?
        } else if live && intent.reference == "bench-042" {
            let result = middleware.submit(&prepared)?;
            live_result = serde_json::to_value(&result)?;
            result
        } else if env_cfg.has_credentials() {
            Submission::NotSent { reason: "benchmark is offline unless AEGIS_BENCH_LIVE=1".into() }
        } else {
            Submission::NotSent { reason: "PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set".into() }
        };
        summaries.push(json!({
            "case": name,
            "amount_cents": prepared.amount_cents,
            "risk_score": round6(f64::from(prepared.risk_score)),
            "decision": if prepared.decision.allowed { "Allow" } else { "Deny" },
            "policies": prepared.decision.determining_policies,
            "redactions": prepared.redactions,
            "unformatted_sensitive_fields": prepared.has_unformatted_sensitive_fields,
            "receipt_verified": prepared.receipt_verified,
            "receipt_reverified": prepared.verify().is_ok(),
            "submission": submission,
        }));
        rows.push(telemetry_row(&prepared, &submission, &env_cfg.agent_id));
    }

    let standard = &cases[0].2;
    for _ in 0..200 {
        mw.prepare(standard)?;
    }
    let (mut prepare, mut redact) = (Vec::new(), Vec::new());
    for _ in 0..scaled(2000, scale) {
        let p = mw.prepare(standard)?;
        prepare.push(p.timings.total_us);
        redact.push(p.timings.redact_us);
    }
    let signer = ReceiptSigner::generate();
    let body = mw.prepare(standard)?.body;
    let (mut sign, mut verify) = (Vec::new(), Vec::new());
    for _ in 0..scaled(2000, scale) {
        let t = Instant::now();
        let receipt = signer.sign_json(&body);
        sign.push(us(t.elapsed()));
        let t = Instant::now();
        verify_json(&receipt, &body)?;
        verify.push(us(t.elapsed()));
    }
    let sandbox = if live {
        json!({"attempted": true, "result": live_result})
    } else if env_cfg.has_credentials() {
        json!({"attempted": false, "reason": "credentials present; set AEGIS_BENCH_LIVE=1 to submit the $42 case to the Sandbox"})
    } else {
        json!({"attempted": false, "reason": "PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set"})
    };
    Ok((
        json!({
            "prepare_order_us": stats(prepare),
            "redact_us": stats(redact),
            "ed25519_sign_us": stats(sign),
            "ed25519_verify_us": stats(verify),
            "cases": summaries,
            "sandbox_submission": sandbox,
        }),
        rows,
    ))
}

fn bench_router(scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>, scale: f64) -> Res<Value> {
    let cfg = RouterConfig::from_env();
    let live = std::env::var("AEGIS_BENCH_LIVE").as_deref() == Ok("1") && cfg.nebius_api_key.is_some();
    let has_key = cfg.nebius_api_key.is_some();
    let router = RecoveryRouter::new(cfg, scorer, gov);
    let failure = BuildFailure {
        command: vec!["cargo".into(), "build".into(), "--release".into()],
        exit_code: 101,
        stderr: concat!(
            "error[E0425]: cannot find value `receipt` in this scope\n  --> src/lib.rs:42:9\n",
            "error[E0308]: mismatched types\n  --> src/orders.rs:17:20\n",
            "warning: unused import: `std::io`\n  --> src/lib.rs:3:5\n",
            "error: could not compile `demo` (lib) due to 2 previous errors; 1 warning emitted\n"
        )
        .to_string(),
        retry_count: 1,
    };
    for _ in 0..200 {
        router.triage(&failure)?;
    }
    let mut samples = Vec::new();
    for _ in 0..scaled(5000, scale) {
        let t = Instant::now();
        router.triage(&failure)?;
        samples.push(us(t.elapsed()));
    }
    let triage = router.triage(&failure)?;
    let live_calls = if live {
        match router.probe_model(Tier::NemotronSuper) {
            Ok((reply, latency_ms)) => json!({"attempted": true, "result": format!("tier-1 replied {reply:?} in {latency_ms:.1} ms")}),
            Err(e) => json!({"attempted": true, "result": format!("tier-1 call failed: {e}")}),
        }
    } else if has_key {
        json!({"attempted": false, "reason": "NEBIUS_API_KEY present; set AEGIS_BENCH_LIVE=1 for a live tier-1 probe"})
    } else {
        json!({"attempted": false, "reason": "NEBIUS_API_KEY not set"})
    };
    Ok(json!({
        "tier0_route_us": stats(samples),
        "triage": triage,
        "live_calls": live_calls,
        "models": {"tier1": router.config().tier1_model, "tier2": router.config().tier2_model, "base_url": router.config().base_url},
    }))
}

fn bench_mcp(scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>, scale: f64) -> Res<Value> {
    let mut server = McpServer::new(scorer, gov, ChatConfig::default());
    let policy = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "aegis_policy_evaluate",
        "arguments": {"action": "paypal::create_order", "ane_risk_score": 20, "amount_cents": 4200}}})
    .to_string();
    let mut vector = FeatureVector::default();
    vector.put(Feature::FilesTouched, 6.0);
    vector.put(Feature::LinesAdded, 120.0);
    let score = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "aegis_score_features",
        "arguments": {"features": vector.as_slice()}}})
    .to_string();
    for _ in 0..200 {
        server.handle_line(&policy);
        server.handle_line(&score);
    }
    let (mut policy_us, mut score_us, mut both) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..scaled(3000, scale) {
        let t = Instant::now();
        server.handle_line(&policy).ok_or("no response")?;
        let a = us(t.elapsed());
        let t = Instant::now();
        server.handle_line(&score).ok_or("no response")?;
        let b = us(t.elapsed());
        policy_us.push(a);
        score_us.push(b);
        both.push(a);
        both.push(b);
    }
    Ok(json!({
        "tools": tool_definitions().len(),
        "tools_call_roundtrip_us": stats(both),
        "policy_evaluate_roundtrip_us": stats(policy_us),
        "score_features_roundtrip_us": stats(score_us),
        "webhooks": {"attempted": false, "reason": "the benchmark never posts to chat; aegis_request_approval posts only to configured webhooks"},
    }))
}

fn target_row(metric: &str, target_ms: f64, measured: Option<f64>) -> Value {
    json!({
        "metric": metric,
        "target": format!("< {target_ms} ms"),
        "measured_p50_ms": measured,
        "pass": measured.map(|m| m < target_ms),
    })
}

fn main() -> Res<()> {
    let mut out = PathBuf::from("artifacts/aegis_phase1_telemetry.json");
    let mut scale = 1.0;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a path")?),
            "--quick" => scale = 0.2,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let root = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))?;
    if out.is_relative() {
        out = root.join(out);
    }
    let started = Instant::now();
    let fixtures = root.join("target/aegis-bench");
    if fixtures.exists() {
        fs::remove_dir_all(&fixtures)?;
    }
    fs::create_dir_all(&fixtures)?;

    eprintln!("aegis-bench: APFS branching / commit / rollback");
    let apfs = bench_apfs(&fixtures, &root, scale)?;
    eprintln!("aegis-bench: CoreML FP16 + MLComputePlan");
    let ane = bench_ane(&root, scale)?;
    let scorer = Arc::new(AneScorer::open_default()?);
    let gov = Arc::new(GovernanceEngine::load_default()?);
    eprintln!("aegis-bench: Cedar");
    let cedar = bench_cedar(&gov, scale)?;
    eprintln!("aegis-bench: end-to-end transactions");
    let e2e = bench_end_to_end(&fixtures, &scorer, &gov, scale)?;
    eprintln!("aegis-bench: PayPal receipts");
    let (paypal, rows) = bench_paypal(scorer.clone(), gov.clone(), scale)?;
    eprintln!("aegis-bench: tier-0 routing + MCP");
    let router = bench_router(scorer.clone(), gov.clone(), scale)?;
    let mcp = bench_mcp(scorer, gov, scale)?;

    let artifacts = out.parent().ok_or("output path has no parent")?.to_path_buf();
    fs::create_dir_all(&artifacts)?;
    fs::write(artifacts.join("paypal_aggrid_rows.json"), serde_json::to_string_pretty(&aggrid_export(&rows))? + "\n")?;
    fs::write(artifacts.join("paypal_bryntum_store.json"), serde_json::to_string_pretty(&bryntum_export(&rows))? + "\n")?;

    let fixture = |name: &str| apfs["fixtures"].as_array().and_then(|f| f.iter().find(|x| x["name"] == name)).cloned().unwrap_or(Value::Null);
    let small = fixture("small");
    let mut summary: Vec<Value> = apfs["fixtures"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| target_row(&format!("clonefile branch · {} ({} files)", f["name"].as_str().unwrap_or("?"), f["files"]), TARGET_CLONEFILE_BRANCH_MS, p50(&f["clonefile_ms"])))
        .collect();
    summary.push(target_row("renameatx_np RENAME_SWAP · small", TARGET_RENAMEATX_SWAP_MS, p50(&small["renameatx_swap_ms"])));
    summary.push(target_row("rollback detach · small", TARGET_ROLLBACK_MS, p50(&small["rollback_detach_ms"])));
    summary.push(target_row("rollback detach + reclaim · small", TARGET_ROLLBACK_MS,
        p50(&small["rollback_detach_ms"]).zip(p50(&small["rollback_reclaim_ms"])).map(|(a, b)| round6(a + b))));
    summary.push(target_row("CoreML FP16 inference · online · cpuAndNeuralEngine", TARGET_ANE_INFERENCE_MS, p50(&ane["runs"]["cpu_and_neural_engine"])));
    let online_on_ane = ane["compute_plan"]["preferred"]["neural_engine"] == ane["compute_plan"]["operations"];
    let batch_on_ane = ane["batch"]["compute_plan"]["preferred"]["neural_engine"] == ane["batch"]["compute_plan"]["operations"];
    summary.push(json!({"metric": "online model executes on the Neural Engine (MLComputePlan)", "target": "all ops on ANE", "measured": ane["compute_plan"]["preferred"], "pass": online_on_ane}));
    summary.push(json!({"metric": "batch model executes on the Neural Engine (MLComputePlan)", "target": "all ops on ANE", "measured": ane["batch"]["compute_plan"]["preferred"], "pass": batch_on_ane}));
    summary.push(target_row("tier-0 route (ANE + Cedar)", TARGET_TIER0_ROUTE_MS, p50(&router["tier0_route_us"]).map(|u| round6(u / 1000.0))));

    let doc = json!({
        "schema": "aegisflow.phase1.telemetry.v1",
        "generated_at": utc_timestamp(),
        "git": git_info(&root),
        "host": host_info(&root),
        "benchmark_seconds": round6(started.elapsed().as_secs_f64()),
        "scale": scale,
        "targets": {
            "clonefile_branch_ms": TARGET_CLONEFILE_BRANCH_MS,
            "renameatx_swap_ms": TARGET_RENAMEATX_SWAP_MS,
            "rollback_ms": TARGET_ROLLBACK_MS,
            "ane_inference_ms": TARGET_ANE_INFERENCE_MS,
            "tier0_route_ms": TARGET_TIER0_ROUTE_MS,
        },
        "apfs": apfs,
        "ane": ane,
        "cedar": cedar,
        "end_to_end": e2e,
        "paypal_receipt": paypal,
        "nebius_router": router,
        "mcp": mcp,
        "external_services": {
            "nebius": "see nebius_router.live_calls",
            "paypal_sandbox": "see paypal_receipt.sandbox_submission",
            "chat_webhooks": "see mcp.webhooks",
        },
        "targets_summary": summary,
    });
    fs::write(&out, serde_json::to_string_pretty(&doc)? + "\n")?;
    fs::remove_dir_all(&fixtures)?;

    println!("wrote {}", out.display());
    for row in doc["targets_summary"].as_array().into_iter().flatten() {
        let verdict = match row["pass"].as_bool() {
            Some(true) => "PASS",
            Some(false) => "MISS",
            None => "n/a ",
        };
        let measured = row.get("measured_p50_ms").and_then(Value::as_f64).map(|v| format!("{v:.4} ms")).unwrap_or_else(|| row["measured"].to_string());
        println!("  [{verdict}] {} — {} (target {})", row["metric"].as_str().unwrap_or(""), measured, row["target"].as_str().unwrap_or(""));
    }
    Ok(())
}
