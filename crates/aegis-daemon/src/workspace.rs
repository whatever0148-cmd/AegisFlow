//! Panel 1: the demo workspace, the four workspace scenarios and team approvals.
//!
//! A run is one agent session step: acquire a branch from the [`WarmBranchPool`], edit real files
//! in it, diff it against live, extract features, score the agent's [`ContextWindow`] on the
//! Neural Engine model, ask Cedar, verify with a fixed compiler command, and then commit
//! (`RENAME_SWAP`), roll back, or hold the branch for a team approval. Every `run.stage` carries
//! the duration of the real call it reports, measured with `Instant` around that call.
//!
//! Locks: scenario runs are serialised by the run queue (`queue`), so two scenarios never edit or
//! verify against the live tree at the same time. Approval resolutions do not wait for a run in
//! progress (a run can hold the queue for minutes during a remote recovery); they only take the
//! short-lived `runs` lock (context windows and pending branches) and `resolving`. Commits from a
//! run and from a resolution are ordered by the storage layer's commit lock and generation check:
//! a branch whose base generation is no longer live is rolled back (`commit` `fail` with
//! `conflict: true`, then a normal `rollback` stage). Reset takes the queue and `resolving`, so it
//! never replaces the workspace under a run or a resolution. Lock order: queue, resolving, runs.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use adapter_nebius_nemotron::{count_diagnostics, run_command, BuildFailure, CommandOutput, RecoveryRouter, Tier};
use adapter_switch_mcp::{mattermost_card, post_webhook, slack_card, ApprovalRequest, CardError};
use aegis_ane_scorer::{workspace_features, ContextWindow, ExecutionContext, Feature, FeatureVector, FEATURE_COUNT};
use aegis_governance::{risk_to_context, scanner, Action, AuthzRequest, Decision, PolicyContext};
use aegis_storage::{AcquireSource, Branch, ChangeKind, ChangeSet, PoolConfig, ReapKind, ReapRecord, StorageError, WarmBranchPool, Workspace};
use serde_json::{json, Value};

use crate::approvals::{Approval, Lookup};
use crate::demo::{self, EditSummary};
use crate::diffview;
use crate::keepwarm::Model;
use crate::services::{chat_config, nebius_status, router_config, scrub, tavily_status};
use crate::state::{lock, AppState};
use crate::util::{iso_ms, ms, now_iso, opt_ms, round6, ProcessLock};

/// Resource name used for the demo workspace in Cedar requests and approval cards.
pub const WORKSPACE_RESOURCE: &str = "aegis-demo";
const HIGH_IMPACT_POLICY: &str = "forbid-high-impact-without-team-approval";
/// Unmatched reaper records are kept this long waiting for their ticket to be registered.
const REAP_MATCH_WINDOW: Duration = Duration::from_secs(5);
const CARD_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    SafeRefactor,
    SecretLeak,
    MassDelete,
    BuildBreak,
}

impl Scenario {
    pub const ALL: [Scenario; 4] = [Scenario::SafeRefactor, Scenario::SecretLeak, Scenario::MassDelete, Scenario::BuildBreak];

    pub fn id(self) -> &'static str {
        match self {
            Scenario::SafeRefactor => "safe_refactor",
            Scenario::SecretLeak => "secret_leak",
            Scenario::MassDelete => "mass_delete",
            Scenario::BuildBreak => "build_break",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Scenario::SafeRefactor => "Safe multi-file refactor",
            Scenario::SecretLeak => "Unformatted secret in config",
            Scenario::MassDelete => "Mass delete",
            Scenario::BuildBreak => "Build break and recovery",
        }
    }

    pub fn description(self) -> String {
        match self {
            Scenario::SafeRefactor => "Renames the pricing function across every module of the demo crate (lib, pricing, inventory, orders, report) inside a pooled CoW branch.".to_string(),
            Scenario::SecretLeak => "Writes a plaintext, credential-shaped api_key (random per run) into config/app.toml.".to_string(),
            Scenario::MassDelete => format!(
                "Deletes every sample fixture under data/ and every design note under docs/, and rewrites README.md. \
                 Modelling choices, made after measuring: the demo crate holds {fixtures} fixtures, a count picked so that this \
                 change crosses the high-impact risk threshold ({threshold}); and the cleanup agent's deletes are modelled as \
                 separate tool calls ({batch} fixtures per call, then docs/, then README.md), each one event in its context \
                 window. The score stage also reports single_event_risk: the same change scored as one event with no history, \
                 for comparison.",
                fixtures = demo::FIXTURES,
                threshold = aegis_governance::HIGH_IMPACT_THRESHOLD,
                batch = demo::DELETE_BATCH,
            ),
            Scenario::BuildBreak => "Passes an extra argument to format_cents in src/report.rs; verification runs rustc --emit=metadata on the branch, then the recovery tiers.".to_string(),
        }
    }

    pub fn expected(self) -> &'static str {
        match self {
            Scenario::SafeRefactor => "low risk → Cedar allow → verify → commit (RENAME_SWAP)",
            Scenario::SecretLeak => "unformatted secret → Cedar deny → rollback",
            Scenario::MassDelete => "high impact (window of per-tool-call events, see description) → team approval → commit on Approve, rollback on Reject",
            Scenario::BuildBreak => "verify fails → tier 0 triage; tiers 1–2 only with NEBIUS/TAVILY keys → patched commit or rollback",
        }
    }

    /// Agent identity: the principal in Cedar requests and the key of its context window.
    pub fn agent(self) -> &'static str {
        match self {
            Scenario::SafeRefactor => "refactor-agent",
            Scenario::SecretLeak => "config-agent",
            Scenario::MassDelete => "cleanup-agent",
            Scenario::BuildBreak => "build-agent",
        }
    }

    fn edit(self, root: &Path) -> io::Result<EditSummary> {
        match self {
            Scenario::SafeRefactor => demo::safe_refactor(root),
            Scenario::SecretLeak => demo::secret_leak(root),
            Scenario::MassDelete => demo::mass_delete(root),
            Scenario::BuildBreak => demo::build_break(root),
        }
    }
}

pub fn scenarios_json() -> Value {
    Value::Array(
        Scenario::ALL
            .iter()
            .map(|s| json!({"id": s.id(), "title": s.title(), "description": s.description(), "expected": s.expected()}))
            .collect(),
    )
}

/// The workspace and its pool. Replaced as a whole by a reset.
pub struct WsHandles {
    pub ws: Arc<Workspace>,
    pub pool: WarmBranchPool,
}

struct PendingRun {
    run_id: String,
    agent: &'static str,
    branch: Branch,
    risk: f32,
    sensitive: bool,
    started: Instant,
}

#[derive(Default)]
struct RunState {
    windows: HashMap<String, ContextWindow>,
    pending: HashMap<String, PendingRun>,
}

#[derive(Default)]
struct ReapRouting {
    /// Reaper ticket of a rolled-back branch -> run id.
    tickets: HashMap<u64, String>,
    /// Commits whose pruned versions were queued: (run id, versions still to attribute).
    version_runs: VecDeque<(String, usize)>,
    unmatched: Vec<(Instant, ReapRecord)>,
}

pub struct WorkspaceEngine {
    demo_root: PathBuf,
    pool_capacity: usize,
    handles: RwLock<Option<Arc<WsHandles>>>,
    /// Serialises scenario runs (and reset).
    queue: Mutex<()>,
    /// Serialises approval resolutions (and reset).
    resolving: Mutex<()>,
    runs: Mutex<RunState>,
    reaps: Mutex<ReapRouting>,
    /// Branches left in `<store>/branches` by an earlier daemon (pending approvals of a process that
    /// was killed, or a crash) and discarded at startup.
    pub startup_discarded: usize,
    /// Held for the life of the process: one daemon per demo directory.
    _dir_lock: ProcessLock,
}

/// Moves every `<store>/branches/*` into `<store>/trash/` (deleted by `reclaim_trash`). Only called
/// at startup while holding the demo-directory lock, so no branch there belongs to a live run.
fn discard_orphan_branches(ws: &Workspace) -> usize {
    let (branches, trash) = (ws.store().join("branches"), ws.store().join("trash"));
    let Ok(entries) = fs::read_dir(&branches) else { return 0 };
    let mut moved = 0;
    for entry in entries.flatten() {
        let target = trash.join(format!("orphan-{}-{}", std::process::id(), entry.file_name().to_string_lossy()));
        if fs::rename(entry.path(), target).is_ok() {
            moved += 1;
        }
    }
    moved
}

fn open_handles(live: &Path, capacity: usize, discard_orphans: bool) -> Result<(Arc<WsHandles>, usize), StorageError> {
    let ws = Workspace::open(live)?;
    ws.set_background_reclaim(true);
    let discarded = if discard_orphans { discard_orphan_branches(&ws) } else { 0 };
    let _ = ws.reclaim_trash();
    let ws = Arc::new(ws);
    let pool = WarmBranchPool::new(ws.clone(), PoolConfig { capacity: capacity.max(1), ..Default::default() })?;
    Ok((Arc::new(WsHandles { ws, pool }), discarded))
}

/// Deletes `dir` on a background thread (it was renamed out of the way first, so nothing races it).
fn remove_in_background(dir: PathBuf) {
    let _ = std::thread::Builder::new().name("aegis-demo-cleanup".into()).spawn(move || {
        let _ = fs::remove_dir_all(&dir);
    });
}

impl WorkspaceEngine {
    /// Creates (or reuses) `<demo_root>/live` and opens it with a warm pool.
    pub fn open(demo_root: &Path, pool_capacity: usize) -> Result<Self, String> {
        fs::create_dir_all(demo_root).map_err(|e| format!("mkdir {}: {e}", demo_root.display()))?;
        let lock_path = demo_root.join(".aegis-daemon.lock");
        let dir_lock = ProcessLock::try_exclusive(&lock_path)
            .map_err(|e| format!("locking {}: {e}", lock_path.display()))?
            .ok_or_else(|| format!("another aegis-daemon is using the demo directory {} (lock {})", demo_root.display(), lock_path.display()))?;
        let mut engine = Self {
            demo_root: demo_root.to_path_buf(),
            pool_capacity,
            handles: RwLock::new(None),
            queue: Mutex::new(()),
            resolving: Mutex::new(()),
            runs: Mutex::new(RunState::default()),
            reaps: Mutex::new(ReapRouting::default()),
            startup_discarded: 0,
            _dir_lock: dir_lock,
        };
        let trash = engine.demo_root.join(".trash");
        if trash.exists() {
            let parked = engine.demo_root.join(format!(".trash-startup-{}", std::process::id()));
            if fs::rename(&trash, &parked).is_ok() {
                remove_in_background(parked);
            }
        }
        let live = engine.live_path();
        if !demo::looks_valid(&live) {
            if live.exists() {
                engine.park(&live).map_err(|e| format!("moving aside {}: {e}", live.display()))?;
            }
            demo::create(&live).map_err(|e| format!("creating the demo crate: {e}"))?;
        }
        let (handles, discarded) = open_handles(&live, pool_capacity, true).map_err(|e| e.to_string())?;
        engine.startup_discarded = discarded;
        *engine.handles.write().unwrap_or_else(PoisonError::into_inner) = Some(handles);
        Ok(engine)
    }

    pub fn live_path(&self) -> PathBuf {
        self.demo_root.join("live")
    }

    fn store_path(&self) -> PathBuf {
        self.demo_root.join(".aegis-live")
    }

    pub fn handles(&self) -> Option<Arc<WsHandles>> {
        self.handles.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Renames `path` into `<demo_root>/.trash/` and deletes it in the background.
    fn park(&self, path: &Path) -> io::Result<()> {
        let trash = self.demo_root.join(".trash").join(crate::util::random_token().get(..12).unwrap_or("x").to_string());
        fs::create_dir_all(&trash)?;
        fs::rename(path, trash.join(path.file_name().unwrap_or_default()))?;
        remove_in_background(trash);
        Ok(())
    }

    /// `GET /api/workspace`. Blocking (directory walk, store reads).
    pub fn workspace_json(&self) -> Value {
        let live = self.live_path();
        let files = demo::list_files(&live).map(|f| f.len()).unwrap_or(0);
        let (pool, reaper, versions) = match self.handles() {
            Some(h) => {
                let s = h.pool.stats();
                let r = h.ws.reaper_stats();
                let recent: Vec<Value> = s
                    .recent_fs_changes
                    .iter()
                    .rev()
                    .take(4)
                    .map(|c| {
                        let rel = c.path.strip_prefix(h.ws.live()).map(|p| p.display().to_string()).unwrap_or_else(|_| c.path.display().to_string());
                        json!({"path": if rel.is_empty() { ".".to_string() } else { rel }, "flags": format!("{:#x}", c.flags)})
                    })
                    .collect();
                let versions: Vec<Value> = h
                    .ws
                    .versions()
                    .unwrap_or_default()
                    .iter()
                    .map(|v| json!({"number": v.number, "created_at": iso_ms(v.created)}))
                    .collect();
                (
                    json!({
                        "ready": s.ready, "capacity": s.capacity, "hits": s.hits, "misses": s.misses,
                        "stale_discards": s.stale_discards, "last_replenish_ms": opt_ms(s.last_replenish),
                        "replenishing": s.replenishing, "replenished": s.replenished, "fs_events": s.fs_events,
                        "fs_events_ignored": s.fs_events_ignored, "watching_fs_events": s.watching_fs_events,
                        "recent_fs_changes": recent, "last_error": s.last_error,
                    }),
                    json!({"queued": r.queued, "reaped": r.reaped, "failed": r.failed, "last_reap_ms": opt_ms(r.last_reap), "background": r.background}),
                    versions,
                )
            }
            None => (Value::Null, Value::Null, Vec::new()),
        };
        json!({
            "live": fs::canonicalize(&live).unwrap_or(live).display().to_string(),
            "files": files,
            "pool": pool,
            "reaper": reaper,
            "versions": versions,
            "scenarios": scenarios_json(),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Run context and the steps every scenario shares.

struct RunCtx<'a> {
    state: &'a AppState,
    run_id: String,
    /// When the run got the run queue (or, for a resolution, when the original run did).
    started: Instant,
    /// Time spent waiting for the run queue before `started` (runs only).
    queued: Option<Duration>,
}

impl<'a> RunCtx<'a> {
    fn stage(&self, stage: &str, status: &str, ms: Option<f64>, detail: Value) {
        self.state.events.stage(&self.run_id, stage, status, ms, detail);
    }

    fn finish(&self, outcome: &str) {
        self.state.events.send(
            "run.finished",
            json!({"run_id": self.run_id, "outcome": outcome, "total_ms": ms(self.started.elapsed()), "queued_ms": opt_ms(self.queued)}),
        );
    }
}

/// Result of a commit or rollback, as reported in `Approval.result`.
struct StepResult {
    outcome: &'static str,
    action: &'static str,
    ms: Option<f64>,
    detail: Value,
}

impl StepResult {
    fn result_json(&self) -> Value {
        json!({"action": self.action, "ms": self.ms, "detail": self.detail})
    }
}

fn rollback(run: &RunCtx, branch: Branch, reason: &str) -> StepResult {
    let state = run.state;
    match branch.rollback() {
        Ok(r) => {
            let detach = ms(r.detach);
            {
                let mut live = lock(&state.live);
                live.record("rollback_detach_ms", detach);
                live.counters.rollbacks += 1;
            }
            let detail = json!({"reason": reason, "branch": r.branch, "queued": r.queued, "reap_ticket": r.reap_ticket});
            run.stage("rollback", "ok", Some(detach), detail.clone());
            if let Some(ticket) = r.reap_ticket {
                lock(&state.workspace.reaps).tickets.insert(ticket, run.run_id.clone());
            } else if let Some(reclaim) = r.reclaim_inline() {
                state.record("reap_ms", ms(reclaim));
                run.stage("reap", "ok", Some(ms(reclaim)), json!({"kind": "branch", "inline": true}));
            }
            StepResult { outcome: "rolled_back", action: "rollback", ms: Some(detach), detail }
        }
        Err(e) => {
            let detail = json!({"reason": reason, "error": e.to_string()});
            run.stage("rollback", "fail", None, detail.clone());
            StepResult { outcome: "error", action: "rollback", ms: None, detail }
        }
    }
}

fn commit(run: &RunCtx, branch: Branch) -> StepResult {
    let state = run.state;
    let started = Instant::now();
    // A branch based on a generation that is no longer live would conflict inside
    // `Branch::commit`, which then discards it itself (no detach time, no reaper ticket for us).
    // Checking first lets the conflict go through the normal rollback path.
    let live_generation = state.workspace.handles().and_then(|h| h.ws.generation().ok());
    if let Some(live_generation) = live_generation.filter(|g| *g != branch.base_generation()) {
        let error = format!(
            "commit conflict: the live workspace moved to generation {live_generation} after branch {} was created at generation {}",
            branch.id(),
            branch.base_generation()
        );
        run.stage("commit", "fail", Some(ms(started.elapsed())), json!({"error": error, "conflict": true}));
        let mut step = rollback(run, branch, "commit conflict: the live workspace changed after this branch was created");
        step.detail["conflict"] = json!(true);
        return step;
    }
    match branch.commit() {
        Ok(r) => {
            let swap = ms(r.swap);
            {
                let mut live = lock(&state.live);
                live.record("commit_swap_ms", swap);
                live.counters.commits += 1;
            }
            if r.prune_queued {
                lock(&state.workspace.reaps).version_runs.push_back((run.run_id.clone(), r.pruned_versions));
            }
            let detail = json!({
                "version": r.version, "generation": r.generation, "total_ms": ms(r.total),
                "pruned_versions": r.pruned_versions, "prune_queued": r.prune_queued, "branch": r.branch,
            });
            run.stage("commit", "ok", Some(swap), detail.clone());
            StepResult { outcome: "committed", action: "commit", ms: Some(swap), detail }
        }
        Err(e @ StorageError::Conflict { .. }) => {
            // Raced past the generation check above (another process committed in between, or live
            // was replaced outside the storage layer): storage discarded the branch itself.
            lock(&state.live).counters.rollbacks += 1;
            let detail = json!({"error": e.to_string(), "conflict": true, "rollback": "discarded inside Branch::commit; detach time not available"});
            run.stage("commit", "fail", Some(ms(started.elapsed())), detail.clone());
            StepResult { outcome: "rolled_back", action: "rollback", ms: Some(ms(started.elapsed())), detail }
        }
        Err(e) => {
            let detail = json!({"error": e.to_string()});
            run.stage("commit", "fail", Some(ms(started.elapsed())), detail.clone());
            StepResult { outcome: "error", action: "commit", ms: None, detail }
        }
    }
}

fn authorize(state: &AppState, agent: &str, ctx: PolicyContext) -> Result<(Decision, Duration), String> {
    let request = AuthzRequest { principal: agent.to_string(), action: Action::SpeculativeCommit, resource: WORKSPACE_RESOURCE.to_string(), context: ctx };
    let started = Instant::now();
    let decision = state.gov.authorize(&request).map_err(|e| format!("Cedar: {e}"))?;
    let elapsed = started.elapsed();
    state.record("cedar_eval_us", decision.eval_ns as f64 / 1e3);
    Ok((decision, elapsed))
}

fn policy_detail(decision: &Decision, ctx: &PolicyContext) -> Value {
    json!({
        "allowed": decision.allowed,
        "determining_policies": decision.determining_policies,
        "eval_us": decision.eval_ns as f64 / 1e3,
        "action": Action::SpeculativeCommit.cedar_id(),
        "ane_risk_score": ctx.ane_risk_score,
        "has_unformatted_sensitive_fields": ctx.has_unformatted_sensitive_fields,
        "team_approval_granted": ctx.team_approval_granted,
    })
}

fn features_detail(fv: &FeatureVector) -> Value {
    let active = fv.active();
    let shown: Vec<String> = active.iter().take(8).map(|(name, value)| format!("{name} {value:.3}")).collect();
    json!({
        "features": FEATURE_COUNT,
        "nonzero": active.len(),
        "active": shown,
        "unformatted_sensitive_fields": fv.get(Feature::UnformattedSensitiveFields) > 0.5,
    })
}

fn diff_detail(changes: &ChangeSet) -> Value {
    json!({
        "files_changed": changes.files.len(),
        "added": changes.count(ChangeKind::Added),
        "modified": changes.count(ChangeKind::Modified),
        "deleted": changes.count(ChangeKind::Deleted),
        "lines_added": changes.lines_added(),
        "lines_removed": changes.lines_removed(),
        "bytes_changed": changes.bytes_changed(),
        "scanned_entries": changes.scanned_entries,
    })
}

/// `diff` stage detail: the counts, plus the redacted, capped unified diff of the branch against
/// live for the console's diff viewer (rendered after the timed `changes()` call).
fn diff_stage_detail(state: &AppState, branch: &Branch, changes: &ChangeSet) -> Value {
    let settings = state.settings();
    let mut detail = diff_detail(changes);
    detail["unified_diff"] = diffview::unified(branch.live(), branch.path(), changes, |t| scrub(&settings, t));
    detail
}

fn model_name(state: &AppState) -> String {
    state.scorer.model_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Pushes `history` and then `fv` into the agent's context window and scores the window (the risk
/// of `fv` given everything before it). Emits nothing.
fn score_event(state: &AppState, agent: &str, history: &[FeatureVector], fv: FeatureVector) -> Result<(f32, Duration, usize), String> {
    let mut runs = lock(&state.workspace.runs);
    let window = runs.windows.entry(agent.to_string()).or_insert_with(|| state.scorer.new_window());
    for event in history {
        window.push(*event);
    }
    window.push(fv);
    let started = Instant::now();
    let risk = state.scorer.score_window(window);
    let elapsed = started.elapsed();
    state.keepwarm.touch(Model::Workspace);
    let risk = risk.map_err(|e| format!("CoreML: {e}"))?;
    state.record("ane_score_ms", ms(elapsed));
    Ok((risk, elapsed, window.len()))
}

/// The same event scored alone (a session with no history), for comparison with the window score.
fn score_single_event(state: &AppState, fv: &FeatureVector) -> Result<(f32, Duration), String> {
    let started = Instant::now();
    let risk = state.scorer.score(fv);
    let elapsed = started.elapsed();
    state.keepwarm.touch(Model::Workspace);
    let risk = risk.map_err(|e| format!("CoreML: {e}"))?;
    state.record("ane_score_ms", ms(elapsed));
    Ok((risk, elapsed))
}

fn pending_count(state: &AppState) -> u32 {
    lock(&state.workspace.runs).pending.len() as u32
}

fn score_detail(state: &AppState, agent: &str, risk: f32, window_events: usize) -> Value {
    json!({
        "risk": round6(f64::from(risk)),
        "units": state.scorer.units().name(),
        "agent": agent,
        "window_events": window_events,
        "window_len": state.scorer.window_len(),
        "model": model_name(state),
    })
}

/// `rustc` used by the fixed verification command: `RUSTC`, else `~/.cargo/bin/rustc`, else PATH.
fn rustc_path() -> String {
    if let Some(r) = std::env::var("RUSTC").ok().filter(|v| !v.trim().is_empty()) {
        return r;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = Path::new(&home).join(".cargo/bin/rustc");
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }
    "rustc".to_string()
}

/// The fixed verification command (never built from request input). Runs with the branch as cwd
/// and writes its metadata outside the branch, so verification adds no files to the diff.
fn verify_command(demo_root: &Path) -> Vec<String> {
    let out_dir = demo_root.join("verify-out");
    vec![
        rustc_path(),
        "--crate-type".into(),
        "lib".into(),
        "--crate-name".into(),
        demo::CRATE_NAME.into(),
        "--edition".into(),
        "2021".into(),
        "--emit=metadata".into(),
        "--out-dir".into(),
        out_dir.display().to_string(),
        "src/lib.rs".into(),
    ]
}

struct Verification {
    passed: bool,
    command: Vec<String>,
    output: CommandOutput,
}

fn verify(run: &RunCtx, dir: &Path, attempt: &str) -> Result<Verification, String> {
    let demo_root = &run.state.opts.demo_root;
    let command = verify_command(demo_root);
    let _ = fs::create_dir_all(demo_root.join("verify-out"));
    let started = Instant::now();
    let output = run_command(&command, dir, run.state.opts.verify_timeout).map_err(|e| format!("running rustc: {e}"))?;
    let elapsed = started.elapsed();
    let passed = output.exit_code == 0 && !output.timed_out;
    let text = format!("{}{}", output.stderr, output.stdout);
    let (errors, warnings) = count_diagnostics(&text);
    let first_error = text.lines().map(str::trim).find(|l| l.starts_with("error")).map(|l| scanner().redact(l).0);
    let shown = format!("rustc --crate-type lib --crate-name {} --edition 2021 --emit=metadata src/lib.rs", demo::CRATE_NAME);
    run.stage(
        "verify",
        if passed { "ok" } else { "fail" },
        Some(ms(elapsed)),
        json!({"attempt": attempt, "command": shown, "exit_code": output.exit_code, "timed_out": output.timed_out,
               "errors": errors, "warnings": warnings, "first_error": first_error}),
    );
    Ok(Verification { passed, command, output })
}

fn post_cards(state: &AppState, req: &ApprovalRequest) -> BTreeMap<String, String> {
    let chat = chat_config(&state.settings());
    let agent = ureq::AgentBuilder::new().timeout(CARD_TIMEOUT).build();
    // Error texts are built here, never from CardError's Display: webhook URLs are secrets.
    let deliver = |url: Option<&String>, payload: Value| match url {
        None => "disabled".to_string(),
        Some(url) => match post_webhook(&agent, url, &payload) {
            Ok(status) if (200..300).contains(&status) => "sent".to_string(),
            Ok(status) => format!("error: HTTP {status}"),
            Err(CardError::Http { status, .. }) => format!("error: HTTP {status}"),
            Err(CardError::Transport { .. }) => "error: webhook unreachable".to_string(),
        },
    };
    let mut cards = BTreeMap::new();
    cards.insert("mattermost".to_string(), deliver(chat.mattermost_webhook.as_ref(), mattermost_card(req, chat.callback_url.as_deref())));
    cards.insert("slack".to_string(), deliver(chat.slack_webhook.as_ref(), slack_card(req)));
    cards
}

fn tier_level(tier: Tier) -> u32 {
    match tier {
        Tier::Local => 0,
        Tier::NemotronSuper => 1,
        Tier::TavilyNemotronUltra => 2,
    }
}

// ---------------------------------------------------------------------------------------------
// Scenario pipeline.

/// Runs one scenario to completion (or to a pending approval). Blocking: call from
/// `spawn_blocking`. Emits `run.started`, `run.stage`* and `run.finished`.
pub fn run_scenario(state: &AppState, run_id: &str, scenario: Scenario) {
    let requested = Instant::now();
    state.events.send("run.started", json!({"run_id": run_id, "scenario": scenario.id(), "title": scenario.title()}));
    lock(&state.live).counters.runs += 1;
    let _queue = lock(&state.workspace.queue);
    // total_ms counts from here: time spent behind earlier runs or a reset is queued_ms.
    let run = RunCtx { state, run_id: run_id.to_string(), started: Instant::now(), queued: Some(requested.elapsed()) };
    match pipeline(&run, scenario) {
        Ok(outcome) => run.finish(outcome),
        Err(e) => {
            state.events.log("error", format!("{run_id} {}: {e}", scenario.id()));
            run.finish("error");
        }
    }
}

/// Rolls the branch back after a failed step and reports the step's error.
fn abort(run: &RunCtx, branch: Branch, error: String) -> Result<&'static str, String> {
    rollback(run, branch, "pipeline error");
    Err(error)
}

fn pipeline(run: &RunCtx, scenario: Scenario) -> Result<&'static str, String> {
    let state = run.state;
    let handles = state.workspace.handles().ok_or("demo workspace is not open")?;
    let agent = scenario.agent();

    // branch
    let started = Instant::now();
    let (branch, receipt) = handles.pool.acquire(scenario.id()).map_err(|e| format!("acquire: {e}"))?;
    let acquire = started.elapsed();
    let from_pool = receipt.source == AcquireSource::Pool;
    {
        let mut live = lock(&state.live);
        live.record("branch_acquire_ms", ms(acquire));
        if from_pool {
            live.counters.pool_hits += 1;
        } else {
            live.counters.pool_misses += 1;
            live.record("branch_cold_ms", ms(branch.clone_duration()));
        }
    }
    let clone_key = if from_pool { "standby_clone_ms" } else { "clone_ms" };
    let mut detail = json!({
        "source": if from_pool { "pool" } else { "cold" },
        "branch": branch.id(),
        "rename_ms": opt_ms(receipt.rename),
        "standby_age_ms": opt_ms(receipt.standby_age),
        "waited_for_replenish_ms": opt_ms(receipt.waited_for_replenish),
        "stale_discarded": receipt.stale_discarded,
    });
    detail[clone_key] = json!(ms(branch.clone_duration()));
    run.stage("branch", "ok", Some(ms(acquire)), detail);

    // edit
    let started = Instant::now();
    let edit = match scenario.edit(branch.path()) {
        Ok(edit) => edit,
        Err(e) => return abort(run, branch, format!("edit: {e}")),
    };
    let elapsed = started.elapsed();
    state.record("edit_ms", ms(elapsed));
    run.stage("edit", "ok", Some(ms(elapsed)), json!(edit));

    // diff
    let started = Instant::now();
    let changes = match branch.changes() {
        Ok(c) => c,
        Err(e) => return abort(run, branch, format!("diff: {e}")),
    };
    let elapsed = started.elapsed();
    state.record("diff_ms", ms(elapsed));
    run.stage("diff", "ok", Some(ms(elapsed)), diff_stage_detail(state, &branch, &changes));

    // features
    let exec = ExecutionContext {
        branch_age_minutes: branch.age().as_secs_f64() / 60.0,
        concurrent_branches: pending_count(state),
        ..Default::default()
    };
    // An edit made in several tool calls contributes one event per earlier call (its own files
    // only); the speculative commit of the whole branch is the newest event.
    let started = Instant::now();
    let earlier = edit.steps.len().saturating_sub(1);
    let history: Vec<FeatureVector> = edit.steps[..earlier]
        .iter()
        .map(|paths| {
            let step = ChangeSet { files: changes.files.iter().filter(|f| paths.contains(&f.path)).cloned().collect(), scanned_entries: changes.scanned_entries };
            workspace_features(&step, &exec)
        })
        .collect();
    let fv = workspace_features(&changes, &exec);
    let elapsed = started.elapsed();
    let sensitive = fv.get(Feature::UnformattedSensitiveFields) > 0.5;
    let mut detail = features_detail(&fv);
    detail["session_events"] = json!(history.len() + 1);
    run.stage("features", "ok", Some(ms(elapsed)), detail);

    // score
    let (risk, elapsed, window_events) = match score_event(state, agent, &history, fv) {
        Ok(r) => r,
        Err(e) => return abort(run, branch, e),
    };
    let mut detail = score_detail(state, agent, risk, window_events);
    if !history.is_empty() {
        // Multi-call edits: show what the whole change scores as one event, measured now.
        let (single, single_elapsed) = match score_single_event(state, &fv) {
            Ok(r) => r,
            Err(e) => return abort(run, branch, e),
        };
        detail["single_event_risk"] = json!(round6(f64::from(single)));
        detail["single_event_score_ms"] = json!(ms(single_elapsed));
    }
    run.stage("score", "ok", Some(ms(elapsed)), detail);

    // policy
    let ctx = PolicyContext { ane_risk_score: risk_to_context(risk), has_unformatted_sensitive_fields: sensitive, ..Default::default() };
    let (decision, elapsed) = match authorize(state, agent, ctx.clone()) {
        Ok(d) => d,
        Err(e) => return abort(run, branch, e),
    };
    run.stage("policy", if decision.allowed { "ok" } else { "fail" }, Some(ms(elapsed)), policy_detail(&decision, &ctx));

    let needs_approval =
        !decision.allowed && !sensitive && decision.determining_policies.iter().any(|p| p == HIGH_IMPACT_POLICY);
    if !decision.allowed && !needs_approval {
        run.stage("verify", "skip", None, json!({"reason": "Cedar denied the speculative commit; nothing to verify"}));
        return Ok(rollback(run, branch, "Cedar denied the speculative commit").outcome);
    }

    // verify
    let verification = match verify(run, branch.path(), "initial") {
        Ok(v) => v,
        Err(e) => return abort(run, branch, e),
    };
    if verification.passed {
        if needs_approval {
            request_approval(run, scenario, branch, risk, sensitive, &decision, &changes, &edit);
            return Ok("pending_approval");
        }
        return Ok(commit(run, branch).outcome);
    }
    if needs_approval {
        return Ok(rollback(run, branch, "verification failed; not sent for approval").outcome);
    }
    recover(run, agent, branch, &changes, verification)
}

#[allow(clippy::too_many_arguments)]
fn request_approval(
    run: &RunCtx,
    scenario: Scenario,
    branch: Branch,
    risk: f32,
    sensitive: bool,
    decision: &Decision,
    changes: &ChangeSet,
    edit: &EditSummary,
) {
    let state = run.state;
    let agent = scenario.agent();
    let req = ApprovalRequest::new(branch.id(), WORKSPACE_RESOURCE, agent, risk, decision, changes);
    let cards = post_cards(state, &req);
    let id = lock(&state.approvals).next_id();
    let approval = Approval {
        id: id.clone(),
        run_id: Some(run.run_id.clone()),
        kind: "workspace",
        title: format!("{} in demo workspace", scenario.title()),
        summary: format!(
            "{} by {agent}. ANE risk {:.1} is at or above {}: Cedar requires team approval before the speculative commit.",
            edit.summary,
            risk,
            aegis_governance::HIGH_IMPACT_THRESHOLD
        ),
        risk: round6(f64::from(risk)),
        files_changed: changes.files.len(),
        excerpt: req.excerpt.clone(),
        determining_policies: decision.determining_policies.clone(),
        created_at: now_iso(),
        status: "pending",
        resolved_by: None,
        resolved_at: None,
        result: None,
        cards: cards.clone(),
    };
    lock(&state.workspace.runs)
        .pending
        .insert(id.clone(), PendingRun { run_id: run.run_id.clone(), agent, branch, risk, sensitive, started: run.started });
    lock(&state.approvals).insert(approval.clone());
    run.stage("approval", "pending", None, json!({"approval_id": id, "cards": cards}));
    state.events.send("approval.created", json!({"approval": approval}));
}

/// Verification failed: tier 0 always; tiers 1–2 through `RecoveryRouter::recover` only when
/// `NEBIUS_API_KEY` is configured (Tavily only with `TAVILY_API_KEY`).
fn recover(
    run: &RunCtx,
    agent: &'static str,
    branch: Branch,
    changes: &ChangeSet,
    verification: Verification,
) -> Result<&'static str, String> {
    let state = run.state;
    let settings = state.settings();
    let cfg = router_config(&settings, state.opts.verify_timeout);
    let router = RecoveryRouter::new(cfg.clone(), state.scorer.clone(), state.gov.clone());
    let failure = BuildFailure {
        command: verification.command.clone(),
        exit_code: verification.output.exit_code,
        stderr: format!("{}{}", verification.output.stderr, verification.output.stdout),
        retry_count: 0,
    };

    // Tier 0: local triage (CoreML + Cedar), always.
    let started = Instant::now();
    let triage = router.triage(&failure);
    state.keepwarm.touch(Model::Workspace);
    let triage = match triage {
        Ok(t) => t,
        Err(e) => {
            run.stage("recover", "fail", Some(ms(started.elapsed())), json!({"tier": 0, "enabled": true, "error": e.to_string()}));
            return Ok(rollback(run, branch, "tier-0 triage failed").outcome);
        }
    };
    let triage_elapsed = started.elapsed();
    let exec = ExecutionContext {
        build_failed: true,
        compile_errors: triage.compile_errors,
        compile_warnings: triage.compile_warnings,
        branch_age_minutes: branch.age().as_secs_f64() / 60.0,
        concurrent_branches: pending_count(state),
        ..Default::default()
    };
    let (window_risk, window_elapsed, window_events) = match score_event(state, agent, &[], workspace_features(changes, &exec)) {
        Ok(r) => r,
        Err(e) => return abort(run, branch, e),
    };
    let start_tier = tier_level(triage.start_tier);
    run.stage(
        "recover",
        "ok",
        Some(ms(triage_elapsed)),
        json!({
            "tier": 0,
            "model": "local: CoreML risk + Cedar (RecoveryRouter::triage)",
            "enabled": true,
            "reason": format!("local triage done; remote recovery would start at tier {start_tier}"),
            "patch_applied": false,
            "risk": round6(f64::from(triage.risk_score)),
            "window_risk": round6(f64::from(window_risk)),
            "window_events": window_events,
            "window_score_ms": ms(window_elapsed),
            "compile_errors": triage.compile_errors,
            "compile_warnings": triage.compile_warnings,
            "start_tier": start_tier,
            "autonomous_commit_allowed": triage.autonomous_commit_allowed,
            "determining_policies": triage.determining_policies,
            "triage_us": round6(triage.elapsed_us),
        }),
    );

    if cfg.remote_unavailable().is_some() {
        // "NEBIUS_API_KEY not set", why the configured endpoint was refused the key, or which
        // tier model has neither an override nor a provider default.
        let (_, reason) = nebius_status(&settings);
        let reason = reason.as_str();
        run.stage("recover", "skip", None, json!({"tier": 1, "model": cfg.tier1_model, "enabled": false, "reason": reason, "patch_applied": false}));
        run.stage(
            "recover",
            "skip",
            None,
            json!({"tier": 2, "model": cfg.tier2_model, "enabled": false, "reason": reason, "patch_applied": false, "tavily": tavily_status(&settings)}),
        );
        return Ok(rollback(run, branch, "build still failing and remote recovery tiers are disabled").outcome);
    }

    // Tiers 1-2: the router clones the failing branch, applies the model's patch there, re-runs the
    // verification command and commits into this branch only if it builds and Cedar allows it.
    let inner_store = state.opts.demo_root.join(".recovery").join(&run.run_id);
    let inner = match Workspace::open_with_store(branch.path(), &inner_store) {
        Ok(ws) => ws,
        Err(e) => return abort(run, branch, format!("opening the recovery workspace: {e}")),
    };
    inner.set_background_reclaim(false);
    // Untimed copy-on-write snapshot of the branch before any patch: the viewer's repaired-file
    // diff is this snapshot against the branch after a tier committed its patch into it.
    let snapshot = inner.branch("pre-recovery").ok();
    let started = Instant::now();
    let result = router.recover(&inner, &failure);
    let elapsed = started.elapsed();
    state.keepwarm.touch(Model::Workspace);
    let patch_diff = match (&result, &snapshot) {
        (Ok(o), Some(snap)) if o.recovered => Some(match aegis_storage::diff_trees(snap.path(), branch.path()) {
            Ok(cs) => diffview::unified(snap.path(), branch.path(), &cs, |t| scrub(&settings, t)),
            Err(e) => json!({"error": format!("diff of the repaired branch: {}", e.kind())}),
        }),
        _ => None,
    };
    drop(snapshot);
    drop(inner);
    let _ = fs::remove_dir_all(&inner_store);
    let outcome = match result {
        Ok(o) => o,
        Err(e) => {
            run.stage(
                "recover",
                "fail",
                Some(ms(elapsed)),
                json!({"tier": start_tier, "enabled": true, "error": scrub(&settings, &e.to_string()), "patch_applied": false}),
            );
            return Ok(rollback(run, branch, "remote recovery failed").outcome);
        }
    };
    if start_tier == 2 {
        run.stage(
            "recover",
            "skip",
            None,
            json!({"tier": 1, "model": cfg.tier1_model, "enabled": true, "reason": "tier-0 triage routed this failure straight to tier 2", "patch_applied": false}),
        );
    }
    let tavily_note = outcome.unavailable.iter().find(|u| u.starts_with("tier 2 search")).cloned();
    let mut last_tier = start_tier;
    for attempt in &outcome.attempts {
        let tier = tier_level(attempt.tier);
        last_tier = tier;
        let mut detail = json!({
            "tier": tier,
            "model": attempt.model,
            "enabled": true,
            "reason": scrub(&settings, &attempt.detail),
            "patch_applied": attempt.patch_applied,
            "build_passed": attempt.build_passed,
            "committed_to_branch": attempt.committed,
            "policy_update_valid": attempt.policy_update_valid,
            "risk": attempt.risk_score.map(|r| round6(f64::from(r))),
        });
        if attempt.committed {
            if let Some(view) = &patch_diff {
                detail["patch_diff"] = view.clone();
            }
        }
        if tier == 2 {
            detail["tavily"] = match &tavily_note {
                Some(note) => json!({"enabled": false, "reason": scrub(&settings, note)}),
                None => json!({"enabled": true, "reason": "documentation search results were added to the prompt"}),
            };
        }
        run.stage("recover", if attempt.committed { "ok" } else { "fail" }, attempt.elapsed_ms.map(round6), detail);
    }
    for note in &outcome.unavailable {
        state.events.log("warn", format!("{}: {}", run.run_id, scrub(&settings, note)));
    }
    if !outcome.recovered {
        return Ok(rollback(run, branch, "no recovery tier produced a verified, allowed patch").outcome);
    }

    // The patch is in this branch now: diff, score and authorise the whole change again, re-verify.
    let started = Instant::now();
    let changes = match branch.changes() {
        Ok(c) => c,
        Err(e) => return abort(run, branch, format!("diff: {e}")),
    };
    let elapsed = started.elapsed();
    state.record("diff_ms", ms(elapsed));
    run.stage("diff", "ok", Some(ms(elapsed)), diff_stage_detail(state, &branch, &changes));
    let exec = ExecutionContext {
        retry_count: 1,
        escalation_tier: last_tier,
        branch_age_minutes: branch.age().as_secs_f64() / 60.0,
        concurrent_branches: pending_count(state),
        ..Default::default()
    };
    let fv = workspace_features(&changes, &exec);
    let sensitive = fv.get(Feature::UnformattedSensitiveFields) > 0.5;
    let (risk, elapsed, window_events) = match score_event(state, agent, &[], fv) {
        Ok(r) => r,
        Err(e) => return abort(run, branch, e),
    };
    run.stage("score", "ok", Some(ms(elapsed)), score_detail(state, agent, risk, window_events));
    let ctx = PolicyContext { ane_risk_score: risk_to_context(risk), has_unformatted_sensitive_fields: sensitive, ..Default::default() };
    let (decision, elapsed) = match authorize(state, agent, ctx.clone()) {
        Ok(d) => d,
        Err(e) => return abort(run, branch, e),
    };
    run.stage("policy", if decision.allowed { "ok" } else { "fail" }, Some(ms(elapsed)), policy_detail(&decision, &ctx));
    if !decision.allowed {
        return Ok(rollback(run, branch, "Cedar denied the recovered change").outcome);
    }
    let verification = match verify(run, branch.path(), "after recovery") {
        Ok(v) => v,
        Err(e) => return abort(run, branch, e),
    };
    if !verification.passed {
        return Ok(rollback(run, branch, "recovered branch does not verify").outcome);
    }
    let result = commit(run, branch);
    Ok(if result.outcome == "committed" { "recovered" } else { result.outcome })
}

// ---------------------------------------------------------------------------------------------
// Approvals.

/// Why an approval could not be resolved.
#[derive(Debug)]
pub enum ResolveError {
    NotFound(String),
    Conflict(String),
    Internal(String),
}

/// Approve: re-evaluate Cedar with `team_approval_granted = true` and commit if allowed.
/// Reject: roll back. Blocking.
pub fn resolve_approval(state: &AppState, id: &str, approve: bool, by: &str) -> Result<Approval, ResolveError> {
    match lock(&state.approvals).lookup(id) {
        Lookup::Unknown => return Err(ResolveError::NotFound(format!("approval {id} does not exist"))),
        Lookup::Resolved(status) => return Err(ResolveError::Conflict(format!("approval {id} is already {status}"))),
        Lookup::Pending => {}
    }
    // Not the run queue: a resolution does not wait for a scenario that is running (storage orders
    // the commits); `resolving` keeps reset from swapping the workspace underneath it.
    let _resolving = lock(&state.workspace.resolving);
    let taken = lock(&state.workspace.runs).pending.remove(id);
    let Some(pending) = taken else {
        return match lock(&state.approvals).lookup(id) {
            Lookup::Resolved(status) => Err(ResolveError::Conflict(format!("approval {id} is already {status}"))),
            _ => Err(ResolveError::Conflict(format!("approval {id} has no branch waiting"))),
        };
    };
    let run = RunCtx { state, run_id: pending.run_id.clone(), started: pending.started, queued: None };
    run.stage(
        "approval",
        if approve { "ok" } else { "fail" },
        None,
        json!({"approval_id": id, "status": if approve { "approved" } else { "rejected" }, "by": by}),
    );
    let step = if approve {
        let ctx = PolicyContext {
            ane_risk_score: risk_to_context(pending.risk),
            has_unformatted_sensitive_fields: pending.sensitive,
            team_approval_granted: true,
            ..Default::default()
        };
        match authorize(state, pending.agent, ctx.clone()) {
            Ok((decision, elapsed)) => {
                run.stage("policy", if decision.allowed { "ok" } else { "fail" }, Some(ms(elapsed)), policy_detail(&decision, &ctx));
                let mut step = if decision.allowed {
                    commit(&run, pending.branch)
                } else {
                    rollback(&run, pending.branch, "Cedar still denies the commit after team approval")
                };
                step.detail["determining_policies"] = json!(decision.determining_policies);
                step.detail["eval_us"] = json!(decision.eval_ns as f64 / 1e3);
                step
            }
            Err(e) => {
                let mut step = rollback(&run, pending.branch, "Cedar evaluation failed");
                step.detail["error"] = json!(e);
                step
            }
        }
    } else {
        rollback(&run, pending.branch, &format!("rejected by {by}"))
    };
    let approval = lock(&state.approvals)
        .resolve(id, approve, by, step.result_json())
        .ok_or_else(|| ResolveError::Internal(format!("approval {id} vanished while resolving")))?;
    state.events.send("approval.resolved", json!({"approval": approval}));
    run.finish(step.outcome);
    Ok(approval)
}

// ---------------------------------------------------------------------------------------------
// Reset and reaper routing.

/// Rolls back pending workspace approvals, discards the demo tree and its store, and creates a
/// fresh demo crate with a new workspace and pool. Blocking.
pub fn reset(state: &AppState) -> Result<(), String> {
    let engine = &state.workspace;
    let _queue = lock(&engine.queue);
    let _resolving = lock(&engine.resolving);
    let mut runs = lock(&engine.runs);
    let pending: Vec<(String, PendingRun)> = runs.pending.drain().collect();
    for (id, p) in pending {
        let run = RunCtx { state, run_id: p.run_id.clone(), started: p.started, queued: None };
        run.stage("approval", "skip", None, json!({"approval_id": id, "status": "rejected", "by": "workspace reset"}));
        let step = rollback(&run, p.branch, "workspace reset while the approval was pending");
        if let Some(approval) = lock(&state.approvals).resolve(&id, false, "system (workspace reset)", step.result_json()) {
            state.events.send("approval.resolved", json!({"approval": approval}));
        }
        run.finish(step.outcome);
    }
    runs.windows.clear();

    let old = engine.handles.write().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(mut old) = old {
        // Wait briefly for in-flight readers (GET /api/workspace) so the pool drops here, before
        // its directories move.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match Arc::try_unwrap(old) {
                Ok(h) => {
                    let ws = h.ws.clone();
                    drop(h.pool);
                    ws.wait_reaped(Duration::from_secs(2));
                    break;
                }
                Err(still_shared) if Instant::now() < deadline => {
                    old = still_shared;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    }
    {
        let mut reaps = lock(&engine.reaps);
        reaps.tickets.clear();
        reaps.version_runs.clear();
        reaps.unmatched.clear();
    }
    let live = engine.live_path();
    for path in [live.clone(), engine.store_path()] {
        if path.exists() {
            engine.park(&path).map_err(|e| format!("moving aside {}: {e}", path.display()))?;
        }
    }
    demo::create(&live).map_err(|e| format!("creating the demo crate: {e}"))?;
    let (handles, _) = open_handles(&live, engine.pool_capacity, false).map_err(|e| e.to_string())?;
    *engine.handles.write().unwrap_or_else(PoisonError::into_inner) = Some(handles);
    state.events.log("info", "demo workspace reset: fresh crate, new store and warm pool");
    Ok(())
}

fn reap_kind(kind: ReapKind) -> &'static str {
    match kind {
        ReapKind::Branch => "branch",
        ReapKind::Version => "version",
        ReapKind::Standby => "standby",
    }
}

/// Drains finished reaper jobs, records `reap_ms` for each and emits `run.stage` "reap" for the
/// ones that belong to a run (rolled-back branches by ticket, pruned versions by commit order).
pub fn poll_reaps(state: &AppState) {
    let Some(handles) = state.workspace.handles() else { return };
    let records = handles.ws.drain_reap_records();
    let mut reaps = lock(&state.workspace.reaps);
    for record in records {
        state.record("reap_ms", ms(record.duration));
        reaps.unmatched.push((Instant::now(), record));
    }
    if reaps.unmatched.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut reaps.unmatched);
    for (seen, record) in pending {
        let mut run_id = reaps.tickets.remove(&record.ticket);
        if run_id.is_none() && record.kind == ReapKind::Version {
            if let Some((id, left)) = reaps.version_runs.front_mut() {
                run_id = Some(id.clone());
                *left = left.saturating_sub(1);
                if *left == 0 {
                    reaps.version_runs.pop_front();
                }
            }
        }
        match run_id {
            Some(run_id) => state.events.stage(
                &run_id,
                "reap",
                if record.ok { "ok" } else { "fail" },
                Some(ms(record.duration)),
                json!({"kind": reap_kind(record.kind), "ticket": record.ticket, "waited_ms": ms(record.waited), "tree": record.name}),
            ),
            None if seen.elapsed() < REAP_MATCH_WINDOW => reaps.unmatched.push((seen, record)),
            None => {}
        }
    }
}

/// Pending approval ids whose branch is still held (diagnostics for shutdown).
pub fn pending_branches(state: &AppState) -> usize {
    lock(&state.workspace.runs).pending.len()
}

/// Rolls back pending approval branches, drops the pool (stops FSEvents, joins the refill thread,
/// disposes standbys) and gives the reaper up to `grace` to finish. Called on shutdown. Does not
/// wait for a scenario that is still running (it only takes the short-lived `runs` lock); such a
/// run's branch is left in `<store>/branches` and discarded by the next start.
pub fn shutdown(state: &AppState, grace: Duration) {
    let mut runs = lock(&state.workspace.runs);
    runs.pending.clear();
    drop(runs);
    let old = state.workspace.handles.write().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(old) = old {
        let ws = old.ws.clone();
        drop(old);
        ws.wait_reaped(grace);
    }
}
