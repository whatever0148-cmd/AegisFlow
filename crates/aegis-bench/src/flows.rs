//! Cedar evaluation, full transactions (pool branch -> edit -> diff -> features -> context-window
//! score -> Cedar -> rollback or commit), PayPal receipts, tier-0 routing and MCP round-trips.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use adapter_nebius_nemotron::{BuildFailure, RecoveryRouter, RouterConfig, Tier};
use adapter_paypal::{telemetry_row, LineItem, OrderIntent, PayPalConfig, PayPalMiddleware, Submission, TelemetryRow};
use adapter_switch_mcp::mcp::tool_definitions;
use adapter_switch_mcp::{ChatConfig, McpServer};
use aegis_ane_scorer::{workspace_features, AneScorer, ExecutionContext, Feature, FeatureVector};
use aegis_governance::receipt::verify_json;
use aegis_governance::{risk_to_context, Action, AuthzRequest, GovernanceEngine, PolicyContext, ReceiptSigner};
use aegis_storage::{AcquireSource, PoolConfig, WarmBranchPool, Workspace};
use serde_json::{json, Value};

use crate::apfs::{settle, Integrity, WAIT};
use crate::util::{ensure, generate_fixture, ms, percentile, round6, scaled, stats, tree_size, us, Res, FIXTURES};

pub fn live_mode() -> bool {
    std::env::var("AEGIS_BENCH_LIVE").as_deref() == Ok("1")
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

pub fn bench_cedar(gov: &GovernanceEngine, scale: f64) -> Res<Value> {
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

/// Cold clones measured on the e2e tree to size the pool's replenish delay.
const DELAY_PROBES: usize = 15;

#[derive(Default)]
struct Sources {
    pool: usize,
    pool_after_wait: usize,
    cold: usize,
}

impl Sources {
    fn add(&mut self, source: AcquireSource, waited: bool) {
        match (source, waited) {
            (AcquireSource::Pool, false) => self.pool += 1,
            (AcquireSource::Pool, true) => self.pool_after_wait += 1,
            (AcquireSource::Cold, _) => self.cold += 1,
        }
    }

    fn json(&self) -> Value {
        json!({"pool_ready": self.pool, "pool_after_waiting_for_refill": self.pool_after_wait, "cold": self.cold})
    }
}

struct Stages {
    names: &'static [&'static str],
    samples: Vec<Vec<f64>>,
}

impl Stages {
    fn new(names: &'static [&'static str]) -> Self {
        Self { names, samples: vec![Vec::new(); names.len()] }
    }

    /// Records the time since `lap` for stage `idx` and restarts the lap.
    fn mark(&mut self, idx: usize, lap: &mut Instant) {
        self.samples[idx].push(us(lap.elapsed()));
        *lap = Instant::now();
    }

    fn p50_json(&self) -> Value {
        let map: serde_json::Map<String, Value> = self
            .names
            .iter()
            .zip(&self.samples)
            .map(|(n, s)| ((*n).to_string(), json!(percentile(s, 50.0).map(round6))))
            .collect();
        Value::Object(map)
    }

    /// Full summary (n, p50, p95, ...) per stage.
    fn stats_json(&self) -> Value {
        let map: serde_json::Map<String, Value> =
            self.names.iter().zip(&self.samples).map(|(n, s)| ((*n).to_string(), stats(s.clone()))).collect();
        Value::Object(map)
    }
}

pub fn bench_end_to_end(fixtures: &Path, scorer: &AneScorer, gov: &GovernanceEngine, scale: f64) -> Res<Value> {
    let live = fixtures.join("e2e").join("live");
    fs::create_dir_all(&live)?;
    generate_fixture(&live, &FIXTURES[1])?;
    fs::create_dir_all(live.join("docs"))?;
    let guide_initial = "# Guide\n";
    fs::write(live.join("docs/guide.md"), guide_initial)?;
    fs::write(live.join("src/payments.rs"), "pub fn charge(cents: u64) -> u64 { cents }\n")?;
    let ws = Arc::new(Workspace::open(&live)?);
    let (files, _, _) = tree_size(&live)?;
    let iterations = scaled(200, scale);
    let key = format!("AKIA{}", "IOSFODNN7EXAMPLE");
    let mut integrity = Integrity::new(&live)?;

    // Replenish delay = 2 x the measured cold clone p50 of this tree, so a transaction's commit or
    // rollback does not queue behind the refill clone started by its acquire.
    let mut probes = Vec::new();
    for _ in 0..DELAY_PROBES {
        let branch = ws.branch("probe")?;
        probes.push(ms(branch.clone_duration()));
        branch.rollback()?;
        ensure(ws.wait_reaped(WAIT), "reaper")?;
    }
    let delay_ms = percentile(&probes, 50.0).unwrap_or(0.0) * 2.0;
    let pool = WarmBranchPool::new(Arc::clone(&ws), PoolConfig::default())?;
    pool.set_replenish_delay(std::time::Duration::from_secs_f64(delay_ms / 1e3));

    let mut totals = Vec::new();
    let mut stages = Stages::new(&["branch", "agent_edit", "diff", "features", "ane_score", "cedar", "rollback"]);
    let (mut denied, mut risky_scores, mut risky_single) = (0usize, Vec::new(), Vec::new());
    let mut rollback_sources = Sources::default();
    let mut risky_session = scorer.new_window();
    for i in 0..iterations {
        settle(&ws, &pool)?;
        let t0 = Instant::now();
        let mut lap = Instant::now();
        let (branch, receipt) = pool.acquire("agent")?;
        stages.mark(0, &mut lap);
        rollback_sources.add(receipt.source, receipt.waited_for_replenish.is_some());
        fs::write(
            branch.path().join("src/payments.rs"),
            format!("pub fn charge(cents: u64) -> u64 {{ cents }}\nconst AWS_KEY: &str = \"{key}\"; // edit {i}\npub fn notify() {{ send(\"billing@example.com\"); }}\n"),
        )?;
        fs::write(branch.path().join(".env"), format!("AWS_ACCESS_KEY_ID={key}\n"))?;
        stages.mark(1, &mut lap);
        let changes = branch.changes()?;
        stages.mark(2, &mut lap);
        let features = workspace_features(&changes, &ExecutionContext { agent_uncertainty: 0.3, ..Default::default() });
        stages.mark(3, &mut lap);
        risky_session.push(features);
        let risk = scorer.score_window(&risky_session)?;
        stages.mark(4, &mut lap);
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
        stages.mark(5, &mut lap);
        denied += usize::from(!decision.allowed);
        risky_scores.push(f64::from(risk));
        if decision.allowed {
            return Err("risky edit was allowed by Cedar; end-to-end rollback scenario is invalid".into());
        }
        branch.rollback()?;
        stages.mark(6, &mut lap);
        totals.push(ms(t0.elapsed()));
        // Outside the timed path: the same event scored without session history.
        risky_single.push(f64::from(scorer.score(&features)?));
    }
    settle(&ws, &pool)?;
    let rollback_integrity = integrity.check("e2e_rollback_path")?;

    let mut commit_totals = Vec::new();
    let mut commit_stages = Stages::new(&["branch", "agent_edit", "diff", "features", "ane_score", "cedar", "commit"]);
    let (mut allowed, mut benign_scores, mut benign_single) = (0usize, Vec::new(), Vec::new());
    let mut commit_sources = Sources::default();
    let mut docs_session = scorer.new_window();
    let mut guide_expected = guide_initial.to_string();
    for i in 0..iterations {
        settle(&ws, &pool)?;
        let t0 = Instant::now();
        let mut lap = Instant::now();
        let (branch, receipt) = pool.acquire("docs")?;
        commit_stages.mark(0, &mut lap);
        commit_sources.add(receipt.source, receipt.waited_for_replenish.is_some());
        let guide = branch.path().join("docs/guide.md");
        let mut text = fs::read_to_string(&guide)?;
        text.push_str(&format!("\nRevision {i}: clarified the setup steps.\n"));
        fs::write(&guide, &text)?;
        commit_stages.mark(1, &mut lap);
        let changes = branch.changes()?;
        commit_stages.mark(2, &mut lap);
        let features = workspace_features(&changes, &ExecutionContext { agent_uncertainty: 0.1, ..Default::default() });
        commit_stages.mark(3, &mut lap);
        docs_session.push(features);
        let risk = scorer.score_window(&docs_session)?;
        commit_stages.mark(4, &mut lap);
        let decision = gov.authorize(&AuthzRequest {
            principal: "core-builder".into(),
            action: Action::SpeculativeCommit,
            resource: "e2e".into(),
            context: PolicyContext { ane_risk_score: risk_to_context(risk), ..Default::default() },
        })?;
        commit_stages.mark(5, &mut lap);
        benign_scores.push(f64::from(risk));
        if decision.allowed {
            allowed += 1;
            branch.commit()?;
            guide_expected = text;
        } else {
            branch.rollback()?;
        }
        commit_stages.mark(6, &mut lap);
        commit_totals.push(ms(t0.elapsed()));
        benign_single.push(f64::from(scorer.score(&features)?));
    }
    settle(&ws, &pool)?;
    integrity.expect_file("docs/guide.md", &guide_expected);
    let commit_integrity = integrity.check("e2e_commit_path")?;
    let pool_stats = pool.stats();
    drop(pool);
    ensure(ws.wait_reaped(WAIT), "reaper")?;

    Ok(json!({
        "fixture_files": files,
        "iterations": iterations,
        "branching": "WarmBranchPool::acquire (capacity 1, FSEvents watch) with replenish delay = 2 x measured cold clone p50; pool settled between transactions",
        "replenish_delay_ms": round6(delay_ms),
        "scoring": "AneScorer::score_window over one ContextWindow per agent session (risky agent, docs agent)",
        "window_len": scorer.window_len(),
        "rollback_path_ms": stats(totals),
        "commit_path_ms": stats(commit_totals),
        "stage_p50_us": stages.p50_json(),
        "commit_stage_p50_us": commit_stages.p50_json(),
        "stage_us": stages.stats_json(),
        "commit_stage_us": commit_stages.stats_json(),
        "stage_method": "Instant laps around each stage of every transaction; ane_score is one AneScorer::score_window per transaction (after diff and features, pool settled before the transaction)",
        "acquire_sources": {"rollback_path": rollback_sources.json(), "commit_path": commit_sources.json()},
        "pool_counters": {"hits": pool_stats.hits, "misses": pool_stats.misses, "stale_discards": pool_stats.stale_discards, "fs_events": pool_stats.fs_events},
        "live_integrity_verified": rollback_integrity,
        "commit_integrity_verified": commit_integrity,
        "integrity": integrity.json(),
        "rollback_case": {"denied": denied, "risk_score": stats(risky_scores), "risk_score_without_history": stats(risky_single)},
        "commit_case": {"allowed": allowed, "risk_score": stats(benign_scores), "risk_score_without_history": stats(benign_single)},
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

pub fn bench_paypal(scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>, scale: f64) -> Res<(Value, Vec<TelemetryRow>)> {
    let env_cfg = PayPalConfig::from_env();
    let live = live_mode() && env_cfg.has_credentials();
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
    let (mut prepare, mut redact, mut score) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..scaled(2000, scale) {
        let p = mw.prepare(standard)?;
        prepare.push(p.timings.total_us);
        redact.push(p.timings.redact_us);
        score.push(p.timings.score_us);
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
            "score_us": stats(score),
            "scoring": "PayPalMiddleware::prepare scores each order with AneScorer::score (one event, no session history) on the online window model",
            "ed25519_sign_us": stats(sign),
            "ed25519_verify_us": stats(verify),
            "cases": summaries,
            "sandbox_submission": sandbox,
        }),
        rows,
    ))
}

pub fn bench_router(scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>, scale: f64) -> Res<Value> {
    let cfg = RouterConfig::from_env();
    // No key, or a NEBIUS_BASE_URL the key may not be sent to, disables the remote tiers.
    let unavailable = cfg.remote_unavailable();
    let has_key = unavailable.is_none();
    let live = live_mode() && has_key;
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
        json!({"attempted": false, "reason": unavailable.unwrap_or_default()})
    };
    Ok(json!({
        "tier0_route_us": stats(samples),
        "scoring": "tier 0 scores the failure with AneScorer::score (one event, no session history) on the online window model, then Cedar",
        "triage": triage,
        "live_calls": live_calls,
        "models": {"tier1": router.config().tier1_model, "tier2": router.config().tier2_model, "base_url": router.config().base_url},
    }))
}

pub fn bench_mcp(scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>, scale: f64) -> Res<Value> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_summaries_carry_n_and_p95() {
        let mut stages = Stages::new(&["first", "second"]);
        let mut lap = Instant::now();
        for _ in 0..4 {
            stages.mark(0, &mut lap);
            stages.mark(1, &mut lap);
        }
        let full = stages.stats_json();
        for name in ["first", "second"] {
            assert_eq!(full[name]["n"], 4);
            assert!(full[name]["p50"].is_number() && full[name]["p95"].is_number());
            assert_eq!(stages.p50_json()[name], full[name]["p50"]);
        }
    }
}
