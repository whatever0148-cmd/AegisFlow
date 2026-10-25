//! Three-tier build recovery.
//!
//! * **Tier 0 — local.** Features from the failure (errors, warnings, retries) are scored by the
//!   CoreML model and checked against Cedar; this picks the starting tier and whether a verified
//!   fix may be committed without a human.
//! * **Tier 1 — Nemotron Super** (`nvidia/Llama-3_3-Nemotron-Super-49B-v1` on Nebius Token
//!   Factory, the default provider at [`DEFAULT_BASE_URL`]), reasoning off, for low-risk failures.
//! * **Tier 2 — Tavily documentation search + Nemotron Ultra**
//!   (`nvidia/Llama-3_1-Nemotron-Ultra-253B-v1` on Nebius), reasoning on, for high-risk failures
//!   or when tier 1's patch does not verify.
//!
//! The remote tiers call the OpenAI-compatible `/chat/completions` of one of the allowlisted
//! providers in [`aegis_config::NEMOTRON_PROVIDERS`] (Nebius Token Factory, NVIDIA NIM,
//! OpenRouter), selected with `NEBIUS_BASE_URL`. Tier model ids default per provider
//! ([`aegis_config::default_tier_models`]) where one is confirmed, otherwise
//! `NEBIUS_TIER1_MODEL` / `NEBIUS_TIER2_MODEL` are required.
//!
//! Every proposed patch (and optional Cedar policy update) is applied inside a fresh APFS CoW
//! branch, the failing command is re-run there, and the result is committed only if the build
//! passes and Cedar allows it; otherwise the branch rolls back. Remote tiers run only when
//! `NEBIUS_API_KEY` is set, both tier models are known and the base URL passes
//! [`aegis_config::validate_base_url`] (an allowlisted provider, or http(s) to a loopback host for
//! tests; see [`endpoint_problem`]), and tier-2 search only when
//! `TAVILY_API_KEY` is set. The key is never sent to, or printed with, a refused endpoint.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use aegis_ane_scorer::{workspace_features, AneScorer, ExecutionContext, Feature};
use aegis_governance::{risk_to_context, Action, AuthzRequest, Decision, GovernanceEngine, PolicyContext};
use aegis_storage::Workspace;
use regex::Regex;
use serde_json::{json, Value};

/// OpenAI-compatible base URL of Nebius Token Factory (the service formerly called Nebius AI Studio,
/// whose `api.studio.nebius.ai` host this replaces), the default provider. Select another
/// allowlisted provider with `NEBIUS_BASE_URL`.
pub const DEFAULT_BASE_URL: &str = aegis_config::DEFAULT_NEMOTRON_BASE_URL;
/// Nebius Token Factory's tier-1 model id (also the default for loopback test endpoints).
pub const TIER1_MODEL: &str = aegis_config::NEBIUS_TIER1_MODEL_ID;
/// Nebius Token Factory's tier-2 model id (also the default for loopback test endpoints).
pub const TIER2_MODEL: &str = aegis_config::NEBIUS_TIER2_MODEL_ID;
pub const TAVILY_URL: &str = "https://api.tavily.com/search";

const MAX_STDERR_CHARS: usize = 8_000;
const MAX_FILE_CHARS: usize = 12_000;
const MAX_CONTEXT_FILES: usize = 4;
/// Tier 0 sends failures with more compile errors than this straight to tier 2.
const TIER1_MAX_ERRORS: u32 = 10;

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error(transparent)]
    Storage(#[from] aegis_storage::StorageError),
    #[error(transparent)]
    Governance(#[from] aegis_governance::GovernanceError),
    #[error(transparent)]
    Ane(#[from] aegis_ane_scorer::AneError),
    #[error("HTTP {status} from {url}: {body}")]
    Http { status: u16, url: String, body: String },
    #[error("transport error calling {url}: {message}")]
    Transport { url: String, message: String },
    #[error("unexpected response from {url}: {message}")]
    Response { url: String, message: String },
    #[error("patch rejected: {0}")]
    Patch(String),
    #[error("{0} is not configured")]
    NotConfigured(&'static str),
    /// The endpoint an API key would be sent to fails the base-URL rule; nothing was sent.
    #[error("{0}")]
    Endpoint(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub nebius_api_key: Option<String>,
    pub base_url: String,
    /// `None` when neither `NEBIUS_TIER1_MODEL` nor a provider default exists (remote tiers off).
    pub tier1_model: Option<String>,
    pub tier2_model: Option<String>,
    pub tavily_api_key: Option<String>,
    pub tavily_url: String,
    pub agent_id: String,
    pub http_timeout: Duration,
    pub verify_timeout: Duration,
}

impl RouterConfig {
    /// `NEBIUS_API_KEY`, `NEBIUS_BASE_URL` (default [`DEFAULT_BASE_URL`]), `NEBIUS_TIER1_MODEL`,
    /// `NEBIUS_TIER2_MODEL` (default: the provider's, [`aegis_config::default_tier_models`]),
    /// `TAVILY_API_KEY`, `TAVILY_URL`, `AEGIS_AGENT_ID`.
    ///
    /// A `NEBIUS_BASE_URL` that fails [`endpoint_problem`] is kept as configured but disables the
    /// remote tiers: [`RouterConfig::remote_unavailable`] gives the reason, and no request carrying
    /// the key is made to it (the check is repeated before every call, whoever built the config).
    pub fn from_env() -> Self {
        let var = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        let base_url = var("NEBIUS_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let (tier1, tier2) = aegis_config::default_tier_models(&base_url);
        Self {
            nebius_api_key: var("NEBIUS_API_KEY"),
            tier1_model: var("NEBIUS_TIER1_MODEL").or_else(|| tier1.map(str::to_string)),
            tier2_model: var("NEBIUS_TIER2_MODEL").or_else(|| tier2.map(str::to_string)),
            base_url,
            tavily_api_key: var("TAVILY_API_KEY"),
            tavily_url: var("TAVILY_URL").unwrap_or_else(|| TAVILY_URL.to_string()),
            agent_id: var("AEGIS_AGENT_ID").unwrap_or_else(|| "build-recovery".to_string()),
            http_timeout: Duration::from_secs(120),
            verify_timeout: Duration::from_secs(600),
        }
    }

    /// Why tiers 1-2 will not run, if they will not: no `NEBIUS_API_KEY`, a base URL the key may
    /// not be sent to, or a tier model with neither an override nor a provider default. Never
    /// contains the key or a refused URL.
    pub fn remote_unavailable(&self) -> Option<String> {
        if self.nebius_api_key.is_none() {
            return Some("NEBIUS_API_KEY not set".to_string());
        }
        endpoint_problem(&self.base_url).or_else(|| missing_models(&self.base_url, self.tier1_model.as_deref(), self.tier2_model.as_deref()))
    }
}

/// Why the tier models are incomplete, if they are: names each missing `NEBIUS_TIER{1,2}_MODEL`
/// and the provider that has no confirmed default for it.
pub fn missing_models(base_url: &str, tier1: Option<&str>, tier2: Option<&str>) -> Option<String> {
    let missing: Vec<&str> = [("NEBIUS_TIER1_MODEL", tier1), ("NEBIUS_TIER2_MODEL", tier2)]
        .into_iter()
        .filter(|(_, model)| model.is_none_or(|m| m.trim().is_empty()))
        .map(|(name, _)| name)
        .collect();
    if missing.is_empty() {
        return None;
    }
    let provider = aegis_config::nemotron_provider(base_url).map(|p| p.label).unwrap_or("this endpoint");
    Some(format!("{} not set ({provider} has no confirmed default model id for it)", missing.join(" / ")))
}

/// Why an API key may not be sent to `base_url`, if it may not
/// ([`aegis_config::validate_base_url`]: one of the allowlisted Nemotron providers, or http(s) to
/// `127.0.0.1`, `localhost` or `[::1]`). The message lists the providers and never repeats the URL.
pub fn endpoint_problem(base_url: &str) -> Option<String> {
    aegis_config::validate_base_url(base_url)
        .err()
        .map(|why| format!("NEBIUS_BASE_URL rejected: {why}; the API key is not sent"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Tier {
    Local,
    NemotronSuper,
    TavilyNemotronUltra,
}

impl Tier {
    fn level(self) -> u32 {
        match self {
            Tier::Local => 0,
            Tier::NemotronSuper => 1,
            Tier::TavilyNemotronUltra => 2,
        }
    }
}

/// Output of a command run with a timeout.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Runs `cmd` in `cwd`, killing it after `timeout`.
pub fn run_command(cmd: &[String], cwd: &Path, timeout: Duration) -> std::io::Result<CommandOutput> {
    let (program, args) = cmd
        .split_first()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty command"))?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let out_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            timed_out = true;
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Ok(CommandOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
        timed_out,
    })
}

/// A failed build: the command, its exit code and output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildFailure {
    pub command: Vec<String>,
    pub exit_code: i32,
    pub stderr: String,
    pub retry_count: u32,
}

impl BuildFailure {
    /// Runs `command` in `dir` and returns the failure, or `None` when it succeeds.
    pub fn capture(command: &[String], dir: &Path, retry_count: u32, timeout: Duration) -> std::io::Result<Option<Self>> {
        let out = run_command(command, dir, timeout)?;
        if out.exit_code == 0 && !out.timed_out {
            return Ok(None);
        }
        Ok(Some(Self {
            command: command.to_vec(),
            exit_code: out.exit_code,
            stderr: format!("{}{}", out.stderr, out.stdout),
            retry_count,
        }))
    }
}

/// (errors, warnings) in compiler output, excluding cargo's summary lines.
pub fn count_diagnostics(output: &str) -> (u32, u32) {
    let mut errors = 0;
    let mut warnings = 0;
    for line in output.lines().map(str::trim_start) {
        if (line.starts_with("error:") || line.starts_with("error["))
            && !line.contains("aborting due to")
            && !line.contains("could not compile")
        {
            errors += 1;
        } else if line.starts_with("warning:") && !line.contains("generated") {
            warnings += 1;
        }
    }
    (errors, warnings)
}

/// Fraction of failed tests reported by `cargo test`-style summaries.
fn tests_failed_ratio(output: &str) -> f64 {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(\d+) passed; (\d+) failed").expect("static regex"));
    let (mut passed, mut failed) = (0u64, 0u64);
    for caps in re.captures_iter(output) {
        passed += caps[1].parse::<u64>().unwrap_or(0);
        failed += caps[2].parse::<u64>().unwrap_or(0);
    }
    if passed + failed == 0 {
        0.0
    } else {
        failed as f64 / (passed + failed) as f64
    }
}

/// Patch and optional Cedar policy extracted from a model reply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelReply {
    pub patch: Option<String>,
    pub cedar_policy: Option<String>,
}

/// Extracts the first ```diff (or ```patch) block and the first ```cedar block, ignoring any
/// `<think>…</think>` reasoning trace.
pub fn parse_reply(text: &str) -> ModelReply {
    static THINK: OnceLock<Regex> = OnceLock::new();
    let think = THINK.get_or_init(|| Regex::new(r"(?s)<think>.*?</think>").expect("static regex"));
    let cleaned = think.replace_all(text, "");
    let mut reply = ModelReply::default();
    let mut lang: Option<String> = None;
    let mut body = String::new();
    for line in cleaned.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            match lang.take() {
                None => {
                    lang = Some(rest.trim().to_lowercase());
                    body.clear();
                }
                Some(l) => {
                    if (l == "diff" || l == "patch") && reply.patch.is_none() {
                        reply.patch = Some(body.clone());
                    } else if l == "cedar" && reply.cedar_policy.is_none() {
                        reply.cedar_policy = Some(body.clone());
                    }
                }
            }
            continue;
        }
        if lang.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    reply
}

/// Applies a unified diff inside `dir` with `/usr/bin/patch -p1`, refusing absolute paths and
/// `..` components. Returns the paths named in the diff headers. Backups are disabled (`-V none`):
/// patch's default `--backup-if-mismatch` would leave `<file>.orig` next to every hunk applied
/// with fuzz, and the branch, backups included, may be committed.
pub fn apply_patch(dir: &Path, patch: &str) -> Result<Vec<PathBuf>, RouterError> {
    let mut touched: Vec<PathBuf> = Vec::new();
    for line in patch.lines() {
        let Some(rest) = line.strip_prefix("--- ").or_else(|| line.strip_prefix("+++ ")) else { continue };
        let raw = rest.split('\t').next().unwrap_or("").trim();
        if raw == "/dev/null" {
            continue;
        }
        let rel = raw.strip_prefix("a/").or_else(|| raw.strip_prefix("b/")).unwrap_or(raw);
        let path = Path::new(rel);
        if rel.is_empty() || path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(RouterError::Patch(format!("unsafe path in diff header: {raw}")));
        }
        if !touched.iter().any(|p| p == path) {
            touched.push(path.to_path_buf());
        }
    }
    if touched.is_empty() {
        return Err(RouterError::Patch("diff has no file headers".into()));
    }
    let mut child = Command::new("/usr/bin/patch")
        .args(["-p1", "-N", "-t", "-s", "-V", "none", "-d"])
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().expect("stdin is piped").write_all(patch.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(RouterError::Patch(format!(
            "patch exited {}: {}{}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(touched)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Triage {
    pub risk_score: f32,
    pub compile_errors: u32,
    pub compile_warnings: u32,
    pub start_tier: Tier,
    /// Whether Cedar would let a verified fix at this risk land without team approval.
    pub autonomous_commit_allowed: bool,
    pub determining_policies: Vec<String>,
    pub elapsed_us: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Attempt {
    pub tier: Tier,
    pub model: String,
    pub patch: Option<String>,
    pub patch_applied: bool,
    pub policy_update_valid: Option<bool>,
    pub build_passed: bool,
    pub risk_score: Option<f32>,
    pub decision: Option<Decision>,
    pub committed: bool,
    pub detail: String,
    /// Wall time of the whole attempt, measured the same way on every path: the tier-2
    /// documentation search, the model call, then patch verification in a branch and the commit or
    /// rollback. `None` until it has finished.
    pub elapsed_ms: Option<f64>,
    /// Duration of the model call alone (included in `elapsed_ms`); `None` until it has returned.
    pub model_ms: Option<f64>,
}

impl Attempt {
    fn new(tier: Tier, model: &str) -> Self {
        Self {
            tier,
            model: model.to_string(),
            patch: None,
            patch_applied: false,
            policy_update_valid: None,
            build_passed: false,
            risk_score: None,
            decision: None,
            committed: false,
            detail: String::new(),
            elapsed_ms: None,
            model_ms: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecoveryOutcome {
    pub triage: Triage,
    pub attempts: Vec<Attempt>,
    pub recovered: bool,
    /// Tiers that could not run and why (missing credentials).
    pub unavailable: Vec<String>,
}

pub struct RecoveryRouter {
    cfg: RouterConfig,
    scorer: Arc<AneScorer>,
    gov: Arc<GovernanceEngine>,
    http: ureq::Agent,
}

impl RecoveryRouter {
    pub fn new(cfg: RouterConfig, scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>) -> Self {
        let http = ureq::AgentBuilder::new().timeout(cfg.http_timeout).build();
        Self { cfg, scorer, gov, http }
    }

    pub fn config(&self) -> &RouterConfig {
        &self.cfg
    }

    /// Sends a one-line prompt to a tier's model and returns (reply, latency in ms). Used by the
    /// benchmark's opt-in live check; requires `NEBIUS_API_KEY`.
    pub fn probe_model(&self, tier: Tier) -> Result<(String, f64), RouterError> {
        let model = self.model(tier)?;
        let started = Instant::now();
        let reply = self.chat(&model, "detailed thinking off", "Reply with the single word OK.", 8)?;
        Ok((reply.trim().chars().take(40).collect(), started.elapsed().as_secs_f64() * 1e3))
    }

    /// Tier 0: local scoring and policy check.
    pub fn triage(&self, failure: &BuildFailure) -> Result<Triage, RouterError> {
        let started = Instant::now();
        let (errors, warnings) = count_diagnostics(&failure.stderr);
        let exec = ExecutionContext {
            build_failed: true,
            tests_failed_ratio: tests_failed_ratio(&failure.stderr),
            retry_count: failure.retry_count,
            compile_errors: errors,
            compile_warnings: warnings,
            ..Default::default()
        };
        let features = workspace_features(&Default::default(), &exec);
        let risk = self.scorer.score(&features)?;
        let decision = self.gov.authorize(&AuthzRequest {
            principal: self.cfg.agent_id.clone(),
            action: Action::SpeculativeCommit,
            resource: "workspace".to_string(),
            context: PolicyContext { ane_risk_score: risk_to_context(risk), ..Default::default() },
        })?;
        let start_tier = if decision.allowed && errors <= TIER1_MAX_ERRORS {
            Tier::NemotronSuper
        } else {
            Tier::TavilyNemotronUltra
        };
        Ok(Triage {
            risk_score: risk,
            compile_errors: errors,
            compile_warnings: warnings,
            start_tier,
            autonomous_commit_allowed: decision.allowed,
            determining_policies: decision.determining_policies,
            elapsed_us: started.elapsed().as_secs_f64() * 1e6,
        })
    }

    /// Runs tier 0, then the remote tiers that are configured, verifying each patch in a branch.
    pub fn recover(&self, ws: &Workspace, failure: &BuildFailure) -> Result<RecoveryOutcome, RouterError> {
        let triage = self.triage(failure)?;
        let mut outcome = RecoveryOutcome { triage: triage.clone(), attempts: Vec::new(), recovered: false, unavailable: Vec::new() };
        if let Some(reason) = self.cfg.remote_unavailable() {
            outcome.unavailable.push(format!("tiers 1-2: {reason}"));
            return Ok(outcome);
        }
        let tiers: &[Tier] = match triage.start_tier {
            Tier::NemotronSuper => &[Tier::NemotronSuper, Tier::TavilyNemotronUltra],
            _ => &[Tier::TavilyNemotronUltra],
        };
        for &tier in tiers {
            // One timer for the whole attempt, read once on every path (failure or not).
            let started = Instant::now();
            let model = self.model(tier)?;
            let docs = if tier == Tier::TavilyNemotronUltra {
                if self.cfg.tavily_api_key.is_some() {
                    match self.search_docs(&search_query(failure)) {
                        Ok(text) => Some(text),
                        Err(e) => {
                            outcome.unavailable.push(format!("tier 2 search failed: {e}"));
                            None
                        }
                    }
                } else {
                    outcome.unavailable.push("tier 2 search: TAVILY_API_KEY not set".to_string());
                    None
                }
            } else {
                None
            };
            let (system, user) = build_prompt(tier, ws.live(), failure, docs.as_deref());
            let model_started = Instant::now();
            let chat = self.chat(&model, &system, &user, if tier == Tier::NemotronSuper { 2048 } else { 4096 });
            let model_ms = Some(millis(model_started.elapsed()));
            let reply = match chat {
                Ok(text) => parse_reply(&text),
                Err(e) => {
                    let mut attempt = Attempt::new(tier, &model);
                    attempt.detail = format!("model call failed: {e}");
                    attempt.model_ms = model_ms;
                    attempt.elapsed_ms = Some(millis(started.elapsed()));
                    outcome.attempts.push(attempt);
                    continue;
                }
            };
            let mut attempt = self.verify_in_branch(ws, failure, &reply, tier, &model)?;
            attempt.model_ms = model_ms;
            attempt.elapsed_ms = Some(millis(started.elapsed()));
            let passed = attempt.build_passed;
            outcome.recovered = attempt.committed;
            outcome.attempts.push(attempt);
            if passed {
                break;
            }
        }
        Ok(outcome)
    }

    /// Applies the reply in a fresh branch, re-runs the failing command there and commits if Cedar
    /// allows it. The caller times the attempt (see [`Attempt::elapsed_ms`]).
    fn verify_in_branch(
        &self,
        ws: &Workspace,
        failure: &BuildFailure,
        reply: &ModelReply,
        tier: Tier,
        model: &str,
    ) -> Result<Attempt, RouterError> {
        let mut attempt = Attempt::new(tier, model);
        attempt.patch = reply.patch.clone();
        let branch = ws.branch(&format!("recovery-t{}", tier.level()))?;
        let Some(patch) = &reply.patch else {
            attempt.detail = "model returned no ```diff block".to_string();
            return Ok(attempt);
        };
        if let Err(e) = apply_patch(branch.path(), patch) {
            attempt.detail = e.to_string();
            return Ok(attempt);
        }
        attempt.patch_applied = true;
        if let Some(policy) = &reply.cedar_policy {
            match self.gov.validate_policy_text(policy) {
                Ok(_) => {
                    let target = branch.path().join("policies/aegis_rules.cedar");
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(target, policy)?;
                    attempt.policy_update_valid = Some(true);
                }
                Err(e) => {
                    attempt.policy_update_valid = Some(false);
                    attempt.detail = format!("proposed Cedar policy rejected: {e}");
                    return Ok(attempt);
                }
            }
        }
        let out = run_command(&failure.command, branch.path(), self.cfg.verify_timeout)?;
        if out.exit_code != 0 || out.timed_out {
            attempt.detail = format!("build still failing in branch (exit {}, timed out: {})", out.exit_code, out.timed_out);
            return Ok(attempt);
        }
        attempt.build_passed = true;
        let changes = branch.changes()?;
        let exec = ExecutionContext {
            retry_count: failure.retry_count + 1,
            escalation_tier: tier.level(),
            branch_age_minutes: branch.age().as_secs_f64() / 60.0,
            ..Default::default()
        };
        let features = workspace_features(&changes, &exec);
        let risk = self.scorer.score(&features)?;
        let decision = self.gov.authorize(&AuthzRequest {
            principal: self.cfg.agent_id.clone(),
            action: Action::SpeculativeCommit,
            resource: "workspace".to_string(),
            context: PolicyContext {
                ane_risk_score: risk_to_context(risk),
                has_unformatted_sensitive_fields: features.get(Feature::UnformattedSensitiveFields) > 0.5,
                ..Default::default()
            },
        })?;
        attempt.risk_score = Some(risk);
        if decision.allowed {
            let receipt = branch.commit()?;
            attempt.committed = true;
            attempt.detail = format!("verified and committed as version {}", receipt.version);
        } else {
            attempt.detail = format!(
                "verified, held for team approval (risk {risk:.1}; {})",
                if decision.determining_policies.is_empty() { "default deny".to_string() } else { decision.determining_policies.join(", ") }
            );
        }
        attempt.decision = Some(decision);
        Ok(attempt)
    }

    /// The configured model id for a remote tier.
    fn model(&self, tier: Tier) -> Result<String, RouterError> {
        let (model, name) = if tier == Tier::TavilyNemotronUltra {
            (&self.cfg.tier2_model, "NEBIUS_TIER2_MODEL")
        } else {
            (&self.cfg.tier1_model, "NEBIUS_TIER1_MODEL")
        };
        model.clone().filter(|m| !m.trim().is_empty()).ok_or(RouterError::NotConfigured(name))
    }

    fn chat(&self, model: &str, system: &str, user: &str, max_tokens: u32) -> Result<String, RouterError> {
        let key = self.cfg.nebius_api_key.as_deref().ok_or(RouterError::NotConfigured("NEBIUS_API_KEY"))?;
        if let Some(problem) = endpoint_problem(&self.cfg.base_url) {
            return Err(RouterError::Endpoint(problem));
        }
        let url = format!("{}/chat/completions", self.cfg.base_url.trim().trim_end_matches('/'));
        let body = json!({
            "model": model,
            "temperature": 0.1,
            "max_tokens": max_tokens,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        });
        let response = self
            .http
            .post(&url)
            .set("Authorization", &format!("Bearer {key}"))
            .set("Content-Type", "application/json")
            .send_json(body);
        let value = read_json(response, &url)?;
        value["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| RouterError::Response { url, message: "no choices[0].message.content".into() })
    }

    fn search_docs(&self, query: &str) -> Result<String, RouterError> {
        let key = self.cfg.tavily_api_key.as_deref().ok_or(RouterError::NotConfigured("TAVILY_API_KEY"))?;
        let url = self.cfg.tavily_url.clone();
        let response = self
            .http
            .post(&url)
            .set("Authorization", &format!("Bearer {key}"))
            .send_json(json!({"query": query, "max_results": 5, "search_depth": "advanced", "include_answer": true}));
        let value = read_json(response, &url)?;
        let mut text = String::new();
        if let Some(answer) = value["answer"].as_str() {
            text.push_str(&format!("Summary: {answer}\n"));
        }
        for result in value["results"].as_array().into_iter().flatten() {
            let content: String = result["content"].as_str().unwrap_or("").chars().take(600).collect();
            text.push_str(&format!(
                "- {} ({}): {}\n",
                result["title"].as_str().unwrap_or("untitled"),
                result["url"].as_str().unwrap_or(""),
                content
            ));
        }
        Ok(text)
    }
}

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn read_json(response: Result<ureq::Response, ureq::Error>, url: &str) -> Result<Value, RouterError> {
    match response {
        Ok(r) => r.into_json::<Value>().map_err(|e| RouterError::Response { url: url.to_string(), message: e.to_string() }),
        Err(ureq::Error::Status(status, r)) => Err(RouterError::Http {
            status,
            url: url.to_string(),
            body: r.into_string().unwrap_or_default().chars().take(500).collect(),
        }),
        Err(ureq::Error::Transport(t)) => Err(RouterError::Transport { url: url.to_string(), message: t.to_string() }),
    }
}

/// Model ids served at `{base_url}/models` (OpenAI-compatible `GET /models`, `data[].id`).
///
/// A cheap authenticated call, used to check that the provider's key and base URL work without
/// spending tokens (Nebius Token Factory, NVIDIA NIM and OpenRouter all answer `{data:[{id,…}]}`). A `base_url` that fails [`endpoint_problem`] is refused with
/// [`RouterError::Endpoint`] before any connection is made.
pub fn list_models(base_url: &str, api_key: &str, timeout: Duration) -> Result<Vec<String>, RouterError> {
    if let Some(problem) = endpoint_problem(base_url) {
        return Err(RouterError::Endpoint(problem));
    }
    let url = format!("{}/models", base_url.trim().trim_end_matches('/'));
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let response = agent.get(&url).set("Authorization", &format!("Bearer {api_key}")).call();
    let value = read_json(response, &url)?;
    let data = value["data"]
        .as_array()
        .ok_or_else(|| RouterError::Response { url: url.clone(), message: "no data[] array".into() })?;
    Ok(data.iter().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect())
}

fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(max)).collect()
}

fn search_query(failure: &BuildFailure) -> String {
    let first_error = failure
        .stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("error"))
        .unwrap_or_else(|| failure.stderr.lines().next().unwrap_or("build failure"));
    format!("{} {}", failure.command.first().map(String::as_str).unwrap_or(""), first_error).chars().take(300).collect()
}

/// Source files mentioned in compiler output (`path:line`), relative and inside the workspace.
fn referenced_files(root: &Path, stderr: &str) -> Vec<PathBuf> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"([A-Za-z0-9_./-]+\.(?:rs|toml|py|swift|ts|js|go|c|h|cedar|json|ya?ml|sh|txt)):\d+").expect("static regex")
    });
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for caps in re.captures_iter(stderr) {
        let rel = PathBuf::from(&caps[1]);
        if rel.is_absolute() || rel.components().any(|c| matches!(c, Component::ParentDir)) {
            continue;
        }
        if root.join(&rel).is_file() && seen.insert(rel.clone()) {
            files.push(rel);
        }
        if files.len() == MAX_CONTEXT_FILES {
            break;
        }
    }
    files
}

/// (system, user) messages for a repair request.
pub fn build_prompt(tier: Tier, root: &Path, failure: &BuildFailure, docs: Option<&str>) -> (String, String) {
    let thinking = if tier == Tier::TavilyNemotronUltra { "detailed thinking on" } else { "detailed thinking off" };
    let system = format!(
        "{thinking}\nYou repair failing builds in a sandboxed copy-on-write branch. Reply with exactly one \
         ```diff fenced block containing a minimal unified diff whose paths are relative to the workspace \
         root with a/ and b/ prefixes. Add a ```cedar block with the complete new policies/aegis_rules.cedar \
         only if the failure is caused by that policy file. Do not add prose outside the fenced blocks."
    );
    let mut user = format!(
        "Workspace: {}\nCommand: `{}`\nExit code: {}\n\nOutput (tail):\n```\n{}\n```\n",
        root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        failure.command.join(" "),
        failure.exit_code,
        tail_chars(&failure.stderr, MAX_STDERR_CHARS)
    );
    for rel in referenced_files(root, &failure.stderr) {
        if let Ok(content) = std::fs::read_to_string(root.join(&rel)) {
            let content: String = content.chars().take(MAX_FILE_CHARS).collect();
            user.push_str(&format!("\n### {}\n```\n{}\n```\n", rel.display(), content));
        }
    }
    if let Some(docs) = docs {
        user.push_str(&format!("\nDocumentation search results:\n{docs}\n"));
    }
    (system, user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/test-fixtures")
                .join(format!("nebius-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("app")).unwrap();
            std::fs::write(root.join("app/status.txt"), "broken\n").unwrap();
            Fixture { root }
        }

        fn live(&self) -> PathBuf {
            self.root.join("app")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn router(cfg: RouterConfig) -> RecoveryRouter {
        RecoveryRouter::new(
            cfg,
            Arc::new(AneScorer::open_default().expect("model")),
            Arc::new(GovernanceEngine::load_default().expect("policies")),
        )
    }

    fn offline_config() -> RouterConfig {
        RouterConfig { nebius_api_key: None, tavily_api_key: None, ..RouterConfig::from_env() }
    }

    fn check_command() -> Vec<String> {
        vec!["sh".into(), "-c".into(), "grep -q fixed status.txt || { echo 'error: status.txt:1 is broken' >&2; exit 1; }".into()]
    }

    /// One request served by [`serve_once`].
    struct Served {
        request: String,
        /// When the connection was accepted (the client is already inside its call).
        accepted: Instant,
        /// Just before the response was written (the client cannot have it earlier).
        responding: Instant,
    }

    /// Serves one HTTP request on 127.0.0.1 with `body`; returns the raw request and its timing.
    fn serve_once(body: String) -> (String, std::thread::JoinHandle<Served>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let accepted = Instant::now();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut payload = vec![0u8; length];
            reader.read_exact(&mut payload).unwrap();
            request.push_str(&String::from_utf8_lossy(&payload));
            let mut stream = stream;
            let responding = Instant::now();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            Served { request, accepted, responding }
        });
        (format!("http://{addr}/v1"), handle)
    }

    #[test]
    fn default_base_url_is_token_factory() {
        assert_eq!(DEFAULT_BASE_URL, "https://api.tokenfactory.nebius.com/v1");
        let cfg = RouterConfig { base_url: DEFAULT_BASE_URL.to_string(), ..RouterConfig::from_env() };
        assert!(cfg.base_url.starts_with("https://api.tokenfactory.nebius.com/"));
        assert!(!DEFAULT_BASE_URL.contains("studio.nebius.ai"));
    }

    #[test]
    fn list_models_reads_openai_compatible_listing() {
        let body = json!({"object": "list", "data": [{"id": TIER1_MODEL, "object": "model"}, {"id": TIER2_MODEL, "object": "model"}]}).to_string();
        let (base_url, server) = serve_once(body);
        let models = list_models(&base_url, "test-key", Duration::from_secs(10)).unwrap();
        let request = server.join().unwrap().request;
        assert!(request.starts_with("GET /v1/models "), "{request}");
        assert!(request.contains("Bearer test-key"));
        assert_eq!(models, vec![TIER1_MODEL.to_string(), TIER2_MODEL.to_string()]);
    }

    #[test]
    fn list_models_reads_openrouter_and_nim_shaped_listings() {
        // OpenRouter adds fields per model and top-level paging; NIM adds owned_by. Only data[].id counts.
        let openrouter = json!({"data": [{"id": "vendor/model-a:free", "name": "A", "pricing": {"prompt": "0"}, "context_length": 4096}], "total_count": 1, "links": {"next": null}});
        let nim = json!({"object": "list", "data": [{"id": "vendor/model-b", "object": "model", "created": 1, "owned_by": "vendor"}]});
        for (body, id) in [(openrouter, "vendor/model-a:free"), (nim, "vendor/model-b")] {
            let (base_url, server) = serve_once(body.to_string());
            let models = list_models(&format!("{base_url}/"), "test-key", Duration::from_secs(10)).unwrap();
            let request = server.join().unwrap().request;
            assert!(request.starts_with("GET /v1/models "), "{request}");
            assert_eq!(models, vec![id.to_string()]);
        }
    }

    #[test]
    fn disallowed_base_urls_are_refused_before_any_connection() {
        // A listener that must never be contacted: http:// to a non-loopback name resolves nowhere
        // useful, and the check happens before resolution. The loopback listener below proves the
        // refusal is about the rule, not reachability.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let key = "nb-unit-test-key-3c9d";
        for url in [
            format!("http://example.invalid:{port}/v1"),
            format!("ftp://127.0.0.1:{port}/v1"),
            format!("http://user@127.0.0.1:{port}/v1"),
            format!("https://x@127.0.0.1:{port}/v1"),
            format!("http://127.0.0.1:{port}\\@example.invalid/v1"),
            "api.tokenfactory.nebius.com/v1".to_string(),
            // Look-alikes of the allowlisted providers.
            "https://integrate.api.nvidia.com.evil.example/v1".to_string(),
            "http://integrate.api.nvidia.com/v1".to_string(),
            "https://x@openrouter.ai/api/v1".to_string(),
            "https://openrouter.ai/api/v2".to_string(),
            "https://proxy.example:8443/v1".to_string(),
        ] {
            let err = list_models(&url, key, Duration::from_secs(2)).unwrap_err();
            assert!(matches!(err, RouterError::Endpoint(_)), "{url}: {err}");
            let text = err.to_string();
            // (A value that is part of an allowed URL necessarily appears in the provider list.)
            let echoes = !aegis_config::NEMOTRON_PROVIDERS.iter().any(|p| p.base_url.contains(url.as_str())) && text.contains(&url);
            assert!(text.contains("NEBIUS_BASE_URL rejected") && !text.contains(key) && !echoes, "{text}");
            assert!(text.contains("https://integrate.api.nvidia.com/v1") && text.contains("https://openrouter.ai/api/v1"), "lists the providers: {text}");
            let cfg = RouterConfig { nebius_api_key: Some(key.into()), base_url: url.clone(), ..RouterConfig::from_env() };
            let reason = cfg.remote_unavailable().expect("remote tiers disabled");
            assert!(reason.contains("NEBIUS_BASE_URL rejected") && !reason.contains(key), "{reason}");
        }
        assert!(listener.accept().is_err(), "no connection was made to the loopback listener");
        for ok in [
            DEFAULT_BASE_URL.to_string(),
            "https://integrate.api.nvidia.com/v1".to_string(),
            "https://openrouter.ai/api/v1/".to_string(),
            format!("http://127.0.0.1:{port}/v1"),
            format!("http://localhost:{port}/v1"),
            format!("http://[::1]:{port}/v1"),
        ] {
            assert_eq!(endpoint_problem(&ok), None, "{ok}");
            let cfg = RouterConfig {
                nebius_api_key: Some(key.into()),
                base_url: ok.clone(),
                tier1_model: Some("test/tier1".into()),
                tier2_model: Some("test/tier2".into()),
                ..RouterConfig::from_env()
            };
            assert_eq!(cfg.remote_unavailable(), None, "{ok}");
        }
        let cfg = RouterConfig { nebius_api_key: None, base_url: DEFAULT_BASE_URL.to_string(), ..RouterConfig::from_env() };
        assert_eq!(cfg.remote_unavailable().as_deref(), Some("NEBIUS_API_KEY not set"));
    }

    #[test]
    fn refused_endpoint_gets_no_request_from_recover() {
        // With a key and a look-alike endpoint, recover() stops at the rule: no attempt, no call.
        let fx = Fixture::new("refused");
        let ws = Workspace::open(fx.live()).unwrap();
        let failure = BuildFailure::capture(&check_command(), &fx.live(), 0, Duration::from_secs(10)).unwrap().unwrap();
        let cfg = RouterConfig {
            nebius_api_key: Some("nb-unit-test-key-77aa".into()),
            base_url: "https://openrouter.ai.evil.example/api/v1".into(),
            tier1_model: Some("test/tier1".into()),
            tier2_model: Some("test/tier2".into()),
            tavily_api_key: None,
            ..RouterConfig::from_env()
        };
        let outcome = router(cfg).recover(&ws, &failure).unwrap();
        assert!(outcome.attempts.is_empty(), "{:?}", outcome.attempts);
        assert_eq!(outcome.unavailable.len(), 1);
        let reason = &outcome.unavailable[0];
        assert!(reason.contains("NEBIUS_BASE_URL rejected") && !reason.contains("evil.example") && !reason.contains("nb-unit-test-key"), "{reason}");
    }

    #[test]
    fn tier_models_default_per_provider_or_are_required() {
        let with = |base_url: &str, tier1: Option<&str>, tier2: Option<&str>| RouterConfig {
            nebius_api_key: Some("nb-unit-test-key-0b1c".into()),
            base_url: base_url.into(),
            tier1_model: tier1.map(str::to_string),
            tier2_model: tier2.map(str::to_string),
            ..RouterConfig::from_env()
        };
        let (nim1, nim2) = aegis_config::default_tier_models("https://integrate.api.nvidia.com/v1");
        assert_eq!(nim1, None);
        assert_eq!(nim2, Some("nvidia/llama-3.1-nemotron-ultra-253b-v1"));
        let reason = with("https://integrate.api.nvidia.com/v1", nim1, nim2).remote_unavailable().expect("tier 1 missing");
        assert!(reason.starts_with("NEBIUS_TIER1_MODEL not set") && reason.contains("NVIDIA NIM") && !reason.contains("TIER2"), "{reason}");
        let reason = with("https://openrouter.ai/api/v1", None, None).remote_unavailable().expect("both missing");
        assert!(reason.starts_with("NEBIUS_TIER1_MODEL / NEBIUS_TIER2_MODEL not set") && reason.contains("OpenRouter"), "{reason}");
        assert_eq!(with("https://openrouter.ai/api/v1", Some("vendor/a"), Some("vendor/b")).remote_unavailable(), None);
        assert_eq!(with(DEFAULT_BASE_URL, Some(TIER1_MODEL), Some(TIER2_MODEL)).remote_unavailable(), None);
        // A router without a tier model refuses the probe before any request.
        let r = router(with("https://openrouter.ai/api/v1", None, None));
        assert!(matches!(r.probe_model(Tier::NemotronSuper), Err(RouterError::NotConfigured("NEBIUS_TIER1_MODEL"))));
    }

    #[test]
    fn parse_reply_extracts_blocks_and_ignores_thinking() {
        let text = "<think>```diff\nnot this\n```</think>Here:\n```diff\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n```\n```cedar\npermit(principal, action, resource);\n```";
        let reply = parse_reply(text);
        assert_eq!(reply.patch.as_deref(), Some("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"));
        assert_eq!(reply.cedar_policy.as_deref(), Some("permit(principal, action, resource);\n"));
    }

    #[test]
    fn diagnostics_skip_cargo_summaries() {
        let out = "error[E0425]: cannot find value `x`\nwarning: unused import\nerror: aborting due to 1 previous error\nwarning: `demo` (lib) generated 1 warning\nerror: could not compile `demo`";
        assert_eq!(count_diagnostics(out), (1, 1));
        assert!((tests_failed_ratio("test result: FAILED. 3 passed; 1 failed;") - 0.25).abs() < 1e-9);
    }

    #[test]
    fn patch_with_parent_dir_is_rejected() {
        let fx = Fixture::new("traversal");
        let err = apply_patch(&fx.live(), "--- a/../escape\n+++ b/../escape\n@@ -0,0 +1 @@\n+x\n").unwrap_err();
        assert!(matches!(err, RouterError::Patch(_)));
    }

    #[test]
    fn unified_diff_applies() {
        let fx = Fixture::new("apply");
        let patch = "--- a/status.txt\n+++ b/status.txt\n@@ -1 +1 @@\n-broken\n+fixed\n";
        assert_eq!(apply_patch(&fx.live(), patch).unwrap(), vec![PathBuf::from("status.txt")]);
        assert_eq!(std::fs::read_to_string(fx.live().join("status.txt")).unwrap(), "fixed\n");
    }

    #[test]
    fn fuzzy_patch_leaves_no_backup_file() {
        let fx = Fixture::new("fuzz");
        std::fs::write(fx.live().join("report.rs"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        // The last context line does not match ("FOUR" vs "four"), so the hunk needs fuzz, which
        // makes patch's default --backup-if-mismatch write report.rs.orig.
        let patch = "--- a/report.rs\n+++ b/report.rs\n@@ -1,4 +1,4 @@\n one\n-two\n+TWO\n three\n FOUR\n";
        apply_patch(&fx.live(), patch).unwrap();
        assert_eq!(std::fs::read_to_string(fx.live().join("report.rs")).unwrap(), "one\nTWO\nthree\nfour\nfive\n");
        let mut names: Vec<String> =
            std::fs::read_dir(fx.live()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["report.rs".to_string(), "status.txt".to_string()]);
    }

    #[test]
    fn triage_is_local_and_routes() {
        let r = router(offline_config());
        let failure = BuildFailure {
            command: vec!["cargo".into(), "build".into()],
            exit_code: 101,
            stderr: "error[E0308]: mismatched types\n --> src/lib.rs:3:5\nerror: could not compile `demo`".into(),
            retry_count: 0,
        };
        let t = r.triage(&failure).unwrap();
        assert_eq!(t.compile_errors, 1);
        assert!((0.0..=100.0).contains(&t.risk_score));
        assert_eq!(t.start_tier == Tier::NemotronSuper, t.autonomous_commit_allowed);
    }

    #[test]
    fn without_key_remote_tiers_are_unavailable() {
        let fx = Fixture::new("nokey");
        let ws = Workspace::open(fx.live()).unwrap();
        let failure = BuildFailure::capture(&check_command(), &fx.live(), 0, Duration::from_secs(10)).unwrap().unwrap();
        let outcome = router(offline_config()).recover(&ws, &failure).unwrap();
        assert!(outcome.attempts.is_empty());
        assert!(!outcome.recovered);
        assert_eq!(outcome.unavailable, vec!["tiers 1-2: NEBIUS_API_KEY not set".to_string()]);
    }

    #[test]
    fn recovers_through_local_openai_compatible_endpoint() {
        let fx = Fixture::new("mock");
        let ws = Workspace::open(fx.live()).unwrap();
        let failure = BuildFailure::capture(&check_command(), &fx.live(), 0, Duration::from_secs(10)).unwrap().unwrap();
        let content = "```diff\n--- a/status.txt\n+++ b/status.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```";
        let body = json!({"choices": [{"message": {"role": "assistant", "content": content}}]}).to_string();
        let (base_url, server) = serve_once(body);
        let cfg = RouterConfig {
            nebius_api_key: Some("test-key".into()),
            base_url,
            tier1_model: Some(TIER1_MODEL.into()),
            tier2_model: Some(TIER2_MODEL.into()),
            tavily_api_key: None,
            ..RouterConfig::from_env()
        };
        let router = router(cfg);
        let outcome = router.recover(&ws, &failure).unwrap();
        let returned = Instant::now();
        let Served { request, accepted, responding } = server.join().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions"), "{request}");
        assert!(request.contains("Bearer test-key"));
        assert!(request.contains(TIER1_MODEL) || request.contains(TIER2_MODEL));
        let attempt = &outcome.attempts[0];
        assert!(attempt.patch_applied && attempt.build_passed, "{attempt:?}");
        assert!(attempt.committed, "low-risk verified fix should commit: {attempt:?}");
        assert!(outcome.recovered);
        assert_eq!(std::fs::read_to_string(fx.live().join("status.txt")).unwrap(), "fixed\n");

        // Timing is checked against causal order, not against a delay: the model timer runs from
        // before the connection until after the response, so it covers accept..responding.
        let (elapsed_ms, model_ms) = (attempt.elapsed_ms.unwrap(), attempt.model_ms.unwrap());
        assert!(model_ms >= millis(responding - accepted), "model_ms {model_ms} misses the served request");
        assert!(elapsed_ms >= model_ms, "elapsed_ms {elapsed_ms} must include the model call ({model_ms})");
        // The attempt ended before recover() returned, so if elapsed_ms covered only the
        // verification (which starts after the response arrived) it would be shorter than
        // returned - responding. Covering the model call makes it longer.
        assert!(
            elapsed_ms >= millis(returned - responding),
            "elapsed_ms {elapsed_ms} starts after the model answered (returned - responding = {})",
            millis(returned - responding)
        );
    }
}
