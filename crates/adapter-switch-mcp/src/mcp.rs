//! Model Context Protocol server (JSON-RPC 2.0, one message per line on stdio).
//!
//! Supports `initialize`, `ping`, `tools/list` and `tools/call`; notifications get no response.
//! Tool failures are reported inside the result with `isError: true`, as the MCP spec requires.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aegis_ane_scorer::{workspace_features, AneScorer, ExecutionContext, Feature, FeatureVector, FEATURE_COUNT};
use aegis_governance::{risk_to_context, Action, AuthzRequest, Decision, GovernanceEngine, PolicyContext};
use aegis_storage::{Branch, ChangeSet, Workspace};
use serde_json::{json, Value};

use crate::cards::{mattermost_card, post_webhook, slack_card, ApprovalRequest, CardError, ChatConfig};

/// Newest first; the first entry is offered when a client asks for an unknown version.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Upper bound on the wait of `aegis_branch_rollback` with `wait_for_reclaim: true`.
const ROLLBACK_REAP_WAIT: Duration = Duration::from_secs(60);

struct Approval {
    approved: bool,
    approver: String,
}

/// A branch plus the workspace it was made from (whose reaper deletes it on rollback).
struct OpenBranch {
    branch: Branch,
    ws: Workspace,
}

pub struct McpServer {
    scorer: Arc<AneScorer>,
    gov: Arc<GovernanceEngine>,
    chat: ChatConfig,
    http: ureq::Agent,
    workspace: Option<Workspace>,
    /// Workspaces replaced by a later open; kept so [`McpServer::shutdown`] waits for their reapers.
    retired: Vec<Workspace>,
    branches: HashMap<String, OpenBranch>,
    approvals: HashMap<String, Approval>,
    agent_id: String,
}

/// What [`McpServer::shutdown`] did.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ShutdownReport {
    /// Branches still open at shutdown, now rolled back.
    pub rolled_back: usize,
    /// Rollbacks that failed (the tree stays in the store for `Workspace::reclaim_trash`).
    pub rollback_errors: Vec<String>,
    /// True when every workspace's reaper finished its queue within the timeout.
    pub reaped: bool,
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn decision_json(decision: &Decision) -> Value {
    json!({
        "allowed": decision.allowed,
        "determining_policies": decision.determining_policies,
        "errors": decision.errors,
        "eval_us": decision.eval_ns as f64 / 1000.0,
    })
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
    })
}

/// Tool catalogue returned by `tools/list`.
pub fn tool_definitions() -> Vec<Value> {
    let branch_id = json!({"branch_id": {"type": "string", "description": "id returned by aegis_branch_create"}});
    vec![
        tool("aegis_workspace_open", "Open an APFS workspace directory for copy-on-write branching.",
             json!({"path": {"type": "string"}}), &["path"]),
        tool("aegis_branch_create", "Clone the live workspace with clonefile(2); edit files under the returned path.",
             json!({"label": {"type": "string"}}), &[]),
        tool("aegis_branch_diff", "List changed files in a branch with the ANE risk score and the Cedar decision for committing it.",
             branch_id.clone(), &["branch_id"]),
        tool("aegis_score_features", "Score a raw 64-feature operation vector (order: models/feature_schema.json).",
             json!({"features": {"type": "array", "items": {"type": "number"}, "minItems": FEATURE_COUNT, "maxItems": FEATURE_COUNT}}),
             &["features"]),
        tool("aegis_policy_evaluate", "Evaluate the Cedar rules for an action and context.",
             json!({
                 "action": {"type": "string", "enum": ["workspace::branch", "workspace::rollback", "workspace::speculative_commit", "paypal::create_order"]},
                 "ane_risk_score": {"type": "integer", "minimum": 0, "maximum": 100},
                 "has_unformatted_sensitive_fields": {"type": "boolean"},
                 "team_approval_granted": {"type": "boolean"},
                 "ed25519_receipt_verified": {"type": "boolean"},
                 "amount_cents": {"type": "integer", "minimum": 0},
                 "principal": {"type": "string"},
                 "resource": {"type": "string"},
             }),
             &["action", "ane_risk_score"]),
        tool("aegis_branch_commit", "Commit a branch with renameatx_np(RENAME_SWAP) if Cedar allows it (recorded team approval counts).",
             branch_id.clone(), &["branch_id"]),
        tool("aegis_branch_rollback",
             "Discard a branch; the live workspace is untouched. detach_us is the O(1) rename into the store's trash. \
              Deletion is normally queued to a background reaper (reclaim_queued: true, reclaim_us: null, reap_ticket); \
              pass wait_for_reclaim: true to wait for it and get the measured reap.reap_us.",
             json!({
                 "branch_id": {"type": "string", "description": "id returned by aegis_branch_create"},
                 "wait_for_reclaim": {"type": "boolean", "description": "wait for the background deletion and report its duration"},
             }),
             &["branch_id"]),
        tool("aegis_request_approval", "Render the diff + approval card and post it to the configured Mattermost/Slack webhooks.",
             json!({"branch_id": {"type": "string"}, "channel": {"type": "string", "enum": ["mattermost", "slack", "both"]}}),
             &["branch_id"]),
        tool("aegis_record_approval", "Record a teammate's approve/reject decision for a branch (chat callback target).",
             json!({"branch_id": {"type": "string"}, "approved": {"type": "boolean"}, "approver": {"type": "string"}}),
             &["branch_id", "approved", "approver"]),
    ]
}

impl McpServer {
    pub fn new(scorer: Arc<AneScorer>, gov: Arc<GovernanceEngine>, chat: ChatConfig) -> Self {
        Self {
            scorer,
            gov,
            chat,
            http: ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(15)).build(),
            workspace: None,
            retired: Vec::new(),
            branches: HashMap::new(),
            approvals: HashMap::new(),
            agent_id: std::env::var("AEGIS_AGENT_ID").unwrap_or_else(|_| "mcp-client".to_string()),
        }
    }

    pub fn with_workspace(mut self, workspace: Workspace) -> Self {
        self.set_workspace(workspace);
        self
    }

    fn set_workspace(&mut self, workspace: Workspace) {
        if let Some(previous) = self.workspace.replace(workspace) {
            self.retired.push(previous);
        }
    }

    /// Rolls back every open branch, then waits (at most `timeout` in total) until the background
    /// reaper of every workspace opened by this server has deleted what it was handed. Call it
    /// before the process exits: the reaper thread is detached, so deletions still queued when
    /// `main` returns are lost and their trees stay in `<store>/trash` until the next
    /// `aegis_workspace_open` reclaims them.
    pub fn shutdown(&mut self, timeout: Duration) -> ShutdownReport {
        let deadline = Instant::now() + timeout;
        let mut report = ShutdownReport { rolled_back: 0, rollback_errors: Vec::new(), reaped: true };
        for (id, open) in self.branches.drain() {
            match open.branch.rollback() {
                Ok(_) => report.rolled_back += 1,
                Err(e) => report.rollback_errors.push(format!("{id}: {e}")),
            }
        }
        self.approvals.clear();
        for ws in self.retired.iter().chain(self.workspace.iter()) {
            let left = deadline.saturating_duration_since(Instant::now());
            report.reaped &= ws.wait_reaped(left);
        }
        report
    }

    /// Handles one line of input; returns the line to write back, if any.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle(&message).map(|v| v.to_string()),
            Err(e) => Some(error_response(Value::Null, -32700, &format!("parse error: {e}")).to_string()),
        }
    }

    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned()?;
        let method = message["method"].as_str().unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => self.initialize(&params),
            "ping" => json!({}),
            "tools/list" => json!({"tools": tool_definitions()}),
            "tools/call" => self.call(&params),
            other => return Some(error_response(id, -32601, &format!("method not found: {other}"))),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSIONS[0]);
        let version = PROTOCOL_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "aegis-switch-mcp", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Open a workspace, create a branch, edit files under the branch path, then diff, request approval and commit or roll back. Commits are gated by Cedar using the on-device ANE risk score.",
        })
    }

    fn call(&mut self, params: &Value) -> Value {
        let name = params["name"].as_str().unwrap_or_default();
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        match self.dispatch(name, &args) {
            Ok(value) => json!({"content": [{"type": "text", "text": value.to_string()}], "structuredContent": value, "isError": false}),
            Err(message) => json!({"content": [{"type": "text", "text": message}], "isError": true}),
        }
    }

    fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "aegis_workspace_open" => {
                let path = args["path"].as_str().ok_or("path is required")?;
                let ws = Workspace::open(path).map_err(|e| e.to_string())?;
                // Trees a previous session left in trash (it exited before its reaper finished,
                // or crashed) are deleted now; nothing of this new handle is queued yet. A failure
                // here does not block the open; it is reported instead.
                let mut out = json!({"live": ws.live(), "store": ws.store(), "filesystem": "apfs"});
                match ws.reclaim_trash() {
                    Ok(n) => out["trash_reclaimed"] = json!(n),
                    Err(e) => out["trash_reclaim_error"] = json!(e.to_string()),
                }
                self.set_workspace(ws);
                Ok(out)
            }
            "aegis_branch_create" => {
                let ws = self.workspace.as_ref().ok_or("no workspace open (call aegis_workspace_open)")?;
                let branch = ws.branch(args["label"].as_str().unwrap_or("mcp")).map_err(|e| e.to_string())?;
                let out = json!({
                    "branch_id": branch.id(),
                    "path": branch.path(),
                    "clonefile_us": us(branch.clone_duration()),
                });
                self.branches.insert(branch.id().to_string(), OpenBranch { branch, ws: ws.clone() });
                Ok(out)
            }
            "aegis_branch_diff" => {
                let id = branch_id(args)?;
                let (changes, features, risk, decision) = self.assess(&id)?;
                Ok(json!({
                    "branch_id": id,
                    "files": changes.files.iter().map(|f| json!({
                        "path": f.path, "kind": f.kind, "lines_added": f.lines_added, "lines_removed": f.lines_removed,
                    })).collect::<Vec<_>>(),
                    "risk_score": risk,
                    "decision": decision_json(&decision),
                    "active_features": features.active().into_iter().map(|(n, v)| json!({"name": n, "value": v})).collect::<Vec<_>>(),
                }))
            }
            "aegis_score_features" => {
                let values: Vec<f32> = args["features"]
                    .as_array()
                    .ok_or("features must be an array")?
                    .iter()
                    .map(|v| v.as_f64().map(|x| x as f32).ok_or("features must be numbers"))
                    .collect::<Result<_, _>>()?;
                let risk = self.scorer.score_slice(&values).map_err(|e| e.to_string())?;
                Ok(json!({"risk_score": risk, "high_impact": risk >= 35.0}))
            }
            "aegis_policy_evaluate" => {
                let action_name = args["action"].as_str().ok_or("action is required")?;
                let action = Action::parse(action_name).ok_or_else(|| format!("unknown action {action_name}"))?;
                let flag = |k: &str| args[k].as_bool().unwrap_or(false);
                let context = PolicyContext {
                    ane_risk_score: args["ane_risk_score"].as_i64().ok_or("ane_risk_score is required")?,
                    has_unformatted_sensitive_fields: flag("has_unformatted_sensitive_fields"),
                    team_approval_granted: flag("team_approval_granted"),
                    ed25519_receipt_verified: flag("ed25519_receipt_verified"),
                    amount_cents: args["amount_cents"].as_i64().unwrap_or(0),
                };
                let decision = self
                    .gov
                    .authorize(&AuthzRequest {
                        principal: args["principal"].as_str().unwrap_or(&self.agent_id).to_string(),
                        action,
                        resource: args["resource"].as_str().unwrap_or("default").to_string(),
                        context,
                    })
                    .map_err(|e| e.to_string())?;
                Ok(decision_json(&decision))
            }
            "aegis_branch_commit" => {
                let id = branch_id(args)?;
                let (_, _, risk, decision) = self.assess(&id)?;
                if !decision.allowed {
                    return Ok(json!({"branch_id": id, "committed": false, "risk_score": risk, "decision": decision_json(&decision)}));
                }
                let open = self.branches.remove(&id).ok_or("unknown branch")?;
                let receipt = open.branch.commit().map_err(|e| e.to_string())?;
                let approved_by = self.approvals.remove(&id).filter(|a| a.approved).map(|a| a.approver);
                Ok(json!({
                    "branch_id": id, "committed": true, "risk_score": risk, "decision": decision_json(&decision),
                    "approved_by": approved_by, "version": receipt.version, "swap_us": us(receipt.swap),
                }))
            }
            "aegis_branch_rollback" => {
                let id = branch_id(args)?;
                let wait = args["wait_for_reclaim"].as_bool().unwrap_or(false);
                let OpenBranch { branch, ws } = self.branches.remove(&id).ok_or_else(|| format!("unknown branch {id}"))?;
                let receipt = branch.rollback().map_err(|e| e.to_string())?;
                self.approvals.remove(&id);
                let mut out = json!({
                    "branch_id": id, "rolled_back": true,
                    "detach_us": us(receipt.detach),
                    // Measured only when deletion ran on this thread; null when it was queued.
                    "reclaim_us": receipt.reclaim_inline().map(us),
                    "reclaim_queued": receipt.queued,
                    "reap_ticket": receipt.reap_ticket,
                });
                if let (true, Some(ticket)) = (wait, receipt.reap_ticket) {
                    out["reap"] = reap_report(&ws, ticket, ROLLBACK_REAP_WAIT);
                }
                Ok(out)
            }
            "aegis_request_approval" => {
                let id = branch_id(args)?;
                let channel = args["channel"].as_str().unwrap_or("both");
                let (changes, _, risk, decision) = self.assess(&id)?;
                let workspace = self.workspace.as_ref().map(|w| display_name(w.live())).unwrap_or_default();
                let request = ApprovalRequest::new(&id, &workspace, &self.agent_id, risk, &decision, &changes);
                let mattermost = mattermost_card(&request, self.chat.callback_url.as_deref());
                let slack = slack_card(&request);
                let mut posted = json!({});
                for (name, url, card) in [
                    ("mattermost", self.chat.mattermost_webhook.clone(), &mattermost),
                    ("slack", self.chat.slack_webhook.clone(), &slack),
                ] {
                    if channel != "both" && channel != name {
                        continue;
                    }
                    posted[name] = match url {
                        Some(url) => match post_webhook(&self.http, &url, card) {
                            Ok(status) => json!({"posted": true, "status": status}),
                            // CardError texts never contain the webhook URL (a secret).
                            Err(e @ CardError::Http { status, .. }) => json!({"posted": false, "status": status, "error": e.to_string()}),
                            Err(e @ CardError::Transport { .. }) => json!({"posted": false, "error": e.to_string()}),
                        },
                        None => json!({"posted": false, "reason": format!("AEGIS_{}_WEBHOOK_URL not set", name.to_uppercase())}),
                    };
                }
                Ok(json!({"branch_id": id, "risk_score": risk, "posted": posted, "cards": {"mattermost": mattermost, "slack": slack}}))
            }
            "aegis_record_approval" => {
                let id = branch_id(args)?;
                if !self.branches.contains_key(&id) {
                    return Err(format!("unknown branch {id}"));
                }
                let approved = args["approved"].as_bool().ok_or("approved must be a boolean")?;
                let approver = args["approver"].as_str().ok_or("approver is required")?.to_string();
                self.approvals.insert(id.clone(), Approval { approved, approver: approver.clone() });
                Ok(json!({"branch_id": id, "approved": approved, "approver": approver}))
            }
            other => Err(format!("unknown tool {other}")),
        }
    }

    /// Diff → features → ANE score → Cedar decision for committing `id`.
    fn assess(&self, id: &str) -> Result<(ChangeSet, FeatureVector, f32, Decision), String> {
        let branch = &self.branches.get(id).ok_or_else(|| format!("unknown branch {id}"))?.branch;
        let changes = branch.changes().map_err(|e| e.to_string())?;
        let exec = ExecutionContext {
            branch_age_minutes: branch.age().as_secs_f64() / 60.0,
            concurrent_branches: self.branches.len() as u32,
            ..Default::default()
        };
        let features = workspace_features(&changes, &exec);
        let risk = self.scorer.score(&features).map_err(|e| e.to_string())?;
        let approval = self.approvals.get(id);
        let decision = self
            .gov
            .authorize(&AuthzRequest {
                principal: self.agent_id.clone(),
                action: Action::SpeculativeCommit,
                resource: self.workspace.as_ref().map(|w| display_name(w.live())).unwrap_or_else(|| "workspace".into()),
                context: PolicyContext {
                    ane_risk_score: risk_to_context(risk),
                    has_unformatted_sensitive_fields: features.get(Feature::UnformattedSensitiveFields) > 0.5,
                    team_approval_granted: approval.is_some_and(|a| a.approved),
                    ..Default::default()
                },
            })
            .map_err(|e| e.to_string())?;
        Ok((changes, features, risk, decision))
    }
}

/// Waits up to `timeout` for the reaper of `ws`, then returns the measured deletion of `ticket`.
fn reap_report(ws: &Workspace, ticket: u64, timeout: Duration) -> Value {
    let idle = ws.wait_reaped(timeout);
    match ws.drain_reap_records().into_iter().find(|r| r.ticket == ticket) {
        Some(r) => json!({"finished": true, "ok": r.ok, "reap_us": us(r.duration), "queue_wait_us": us(r.waited)}),
        None if idle => json!({"finished": false, "reason": "no record for this ticket"}),
        None => json!({"finished": false, "reason": "reaper still busy when the wait limit expired"}),
    }
}

fn branch_id(args: &Value) -> Result<String, String> {
    args["branch_id"].as_str().map(str::to_owned).ok_or_else(|| "branch_id is required".to_string())
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn server() -> McpServer {
        McpServer::new(
            Arc::new(AneScorer::open_default().unwrap()),
            Arc::new(GovernanceEngine::load_default().unwrap()),
            ChatConfig::default(),
        )
    }

    fn call(s: &mut McpServer, id: u64, tool: &str, args: Value) -> Value {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": args}});
        let response = s.handle(&msg).unwrap();
        assert_eq!(response["result"]["isError"], false, "{tool}: {response}");
        response["result"]["structuredContent"].clone()
    }

    struct Fixture(PathBuf);

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `<target>/test-fixtures/mcp-<name>-<pid>/live/docs/guide.md`; the store is `.aegis-live`.
    fn fixture(name: &str) -> (PathBuf, Fixture) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-fixtures/mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("live/docs")).unwrap();
        std::fs::write(root.join("live/docs/guide.md"), "# Guide\n").unwrap();
        (root.clone(), Fixture(root))
    }

    fn entries(dir: &Path) -> usize {
        std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
    }

    #[test]
    fn initialize_negotiates_version() {
        let mut s = server();
        let r = s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        let r = s.handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}})).unwrap();
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        assert!(s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
    }

    #[test]
    fn errors_follow_json_rpc() {
        let mut s = server();
        let r: Value = serde_json::from_str(&s.handle_line("{not json").unwrap()).unwrap();
        assert_eq!(r["error"]["code"], -32700);
        let r = s.handle(&json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"})).unwrap();
        assert_eq!(r["error"]["code"], -32601);
        let r = s.handle(&json!({"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {"name": "nope"}})).unwrap();
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn lists_tools_and_evaluates_policy() {
        let mut s = server();
        let r = s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 9);
        let d = call(&mut s, 2, "aegis_policy_evaluate", json!({"action": "paypal::create_order", "ane_risk_score": 20, "amount_cents": 12000}));
        assert_eq!(d["allowed"], false);
        assert_eq!(d["determining_policies"], json!(["forbid-order-over-50-usd-without-receipt"]));
        let scored = call(&mut s, 3, "aegis_score_features", json!({"features": vec![0.0; FEATURE_COUNT]}));
        assert!(scored["risk_score"].as_f64().unwrap() < 35.0);
    }

    #[test]
    fn branch_diff_approve_commit_and_rollback() {
        let (root, _cleanup) = fixture("flow");
        let mut s = server();
        call(&mut s, 1, "aegis_workspace_open", json!({"path": root.join("live")}));

        let b = call(&mut s, 2, "aegis_branch_create", json!({"label": "docs"}));
        let id = b["branch_id"].as_str().unwrap().to_string();
        std::fs::write(Path::new(b["path"].as_str().unwrap()).join("docs/guide.md"), "# Guide\n\nMore detail.\n").unwrap();
        let diff = call(&mut s, 3, "aegis_branch_diff", json!({"branch_id": id}));
        assert_eq!(diff["files"][0]["path"], "docs/guide.md");
        let card = call(&mut s, 4, "aegis_request_approval", json!({"branch_id": id}));
        assert_eq!(card["posted"]["slack"]["posted"], false, "no webhook configured, nothing may be posted");
        call(&mut s, 5, "aegis_record_approval", json!({"branch_id": id, "approved": true, "approver": "lead"}));
        let commit = call(&mut s, 6, "aegis_branch_commit", json!({"branch_id": id}));
        assert_eq!(commit["committed"], true, "{commit}");
        assert!(std::fs::read_to_string(root.join("live/docs/guide.md")).unwrap().contains("More detail."));

        let b2 = call(&mut s, 7, "aegis_branch_create", json!({}));
        let id2 = b2["branch_id"].as_str().unwrap().to_string();
        std::fs::write(Path::new(b2["path"].as_str().unwrap()).join("docs/guide.md"), "deleted everything").unwrap();
        let rb = call(&mut s, 8, "aegis_branch_rollback", json!({"branch_id": id2}));
        assert_eq!(rb["rolled_back"], true);
        assert!(rb["detach_us"].is_f64(), "{rb}");
        // Background reclaim is the default: the deletion is queued, so no reclaim time exists yet.
        assert_eq!(rb["reclaim_queued"], true, "{rb}");
        assert!(rb["reclaim_us"].is_null(), "a queued deletion has no measured reclaim time: {rb}");
        assert!(rb["reap_ticket"].is_u64(), "{rb}");
        assert!(rb.get("reap").is_none(), "{rb}");
        assert!(std::fs::read_to_string(root.join("live/docs/guide.md")).unwrap().contains("More detail."));

        let b3 = call(&mut s, 9, "aegis_branch_create", json!({}));
        let id3 = b3["branch_id"].as_str().unwrap().to_string();
        let rb = call(&mut s, 10, "aegis_branch_rollback", json!({"branch_id": id3, "wait_for_reclaim": true}));
        assert_eq!(rb["reclaim_queued"], true, "{rb}");
        assert!(rb["reclaim_us"].is_null(), "{rb}");
        assert_eq!(rb["reap"]["finished"], true, "{rb}");
        assert_eq!(rb["reap"]["ok"], true, "{rb}");
        assert!(rb["reap"]["reap_us"].is_f64(), "{rb}");
        assert!(!root.join(".aegis-live/trash").join(&id3).exists(), "waited-for tree must be gone");

        let report = s.shutdown(Duration::from_secs(60));
        assert!(report.reaped && report.rollback_errors.is_empty(), "{report:?}");
        assert_eq!(entries(&root.join(".aegis-live/trash")), 0, "every queued deletion finished before shutdown returned");
    }

    #[test]
    fn synchronous_reclaim_reports_measured_time() {
        let (root, _cleanup) = fixture("sync");
        let ws = Workspace::open(root.join("live")).unwrap();
        ws.set_background_reclaim(false);
        let mut s = server().with_workspace(ws);
        let b = call(&mut s, 1, "aegis_branch_create", json!({}));
        let rb = call(&mut s, 2, "aegis_branch_rollback", json!({"branch_id": b["branch_id"], "wait_for_reclaim": true}));
        assert_eq!(rb["reclaim_queued"], false, "{rb}");
        assert!(rb["reclaim_us"].is_f64(), "{rb}");
        assert!(rb["reap_ticket"].is_null() && rb.get("reap").is_none(), "{rb}");
        assert_eq!(entries(&root.join(".aegis-live/trash")), 0);
    }

    #[test]
    fn shutdown_rolls_back_open_branches_and_drains_the_reaper() {
        let (root, _cleanup) = fixture("shutdown");
        let mut s = server();
        call(&mut s, 1, "aegis_workspace_open", json!({"path": root.join("live")}));
        call(&mut s, 2, "aegis_branch_create", json!({"label": "a"}));
        call(&mut s, 3, "aegis_branch_create", json!({"label": "b"}));
        // A second open retires the first workspace handle; its branches and reaper stay tracked.
        call(&mut s, 4, "aegis_workspace_open", json!({"path": root.join("live")}));
        call(&mut s, 5, "aegis_branch_create", json!({"label": "c"}));
        let report = s.shutdown(Duration::from_secs(60));
        assert_eq!(report.rolled_back, 3, "{report:?}");
        assert!(report.reaped && report.rollback_errors.is_empty(), "{report:?}");
        assert_eq!(entries(&root.join(".aegis-live/branches")), 0);
        assert_eq!(entries(&root.join(".aegis-live/trash")), 0);
    }

    #[test]
    fn workspace_open_reclaims_trash_left_by_an_earlier_session() {
        let (root, _cleanup) = fixture("leftover");
        let leftover = root.join(".aegis-live/trash/rb-earlier-session");
        std::fs::create_dir_all(leftover.join("src")).unwrap();
        std::fs::write(leftover.join("src/main.rs"), "fn main() {}\n").unwrap();
        let mut s = server();
        let opened = call(&mut s, 1, "aegis_workspace_open", json!({"path": root.join("live")}));
        assert_eq!(opened["trash_reclaimed"], 1, "{opened}");
        assert!(!leftover.exists());
    }

    /// URL of a local port with no listener (bound, then released), so connecting is refused.
    fn closed_port_url(path: &str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}{path}")
    }

    #[test]
    fn approval_error_never_returns_the_webhook_url() {
        let (root, _cleanup) = fixture("secret");
        let secret = "SECRETTOKEN123";
        let url = closed_port_url(&format!("/services/T000/B000/{secret}"));
        let chat = ChatConfig { slack_webhook: Some(url.clone()), ..ChatConfig::default() };
        let mut s = McpServer::new(
            Arc::new(AneScorer::open_default().unwrap()),
            Arc::new(GovernanceEngine::load_default().unwrap()),
            chat,
        );
        call(&mut s, 1, "aegis_workspace_open", json!({"path": root.join("live")}));
        let b = call(&mut s, 2, "aegis_branch_create", json!({}));
        let msg = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                         "params": {"name": "aegis_request_approval", "arguments": {"branch_id": b["branch_id"], "channel": "slack"}}});
        let response = s.handle(&msg).unwrap();
        let slack = &response["result"]["structuredContent"]["posted"]["slack"];
        assert_eq!(slack["posted"], false, "{response}");
        assert!(slack["error"].as_str().unwrap().starts_with("webhook unreachable"), "{response}");
        let text = response.to_string();
        assert!(!text.contains(secret) && !text.contains(&url) && !text.contains("/services/"), "{text}");
    }
}
