//! Mattermost and Slack approval cards for a workspace branch.

use aegis_governance::{scanner, Decision};
use aegis_storage::{ChangeKind, ChangeSet};
use serde_json::{json, Value};

const EXCERPT_LINES_PER_FILE: usize = 12;
/// Slack section text is limited to 3000 characters; keep room for the code fence.
const MAX_EXCERPT_CHARS: usize = 2_600;

/// Webhook delivery failure. Incoming-webhook URLs are credentials (the token is in the path), so
/// neither variant stores the URL, and the texts it carries have the URL scrubbed out: `Display`
/// and `Debug` are safe to return to MCP clients or to log.
#[derive(Debug, thiserror::Error)]
pub enum CardError {
    #[error("webhook answered HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("webhook unreachable: {message}")]
    Transport { message: String },
}

/// Replaces every occurrence of `url`, its path/query part and its userinfo in `text`.
fn scrub_url(text: &str, url: &str) -> String {
    const MARK: &str = "[webhook]";
    let mut out = if url.is_empty() { text.to_string() } else { text.replace(url, MARK) };
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (authority, rest) = after_scheme.find(['/', '?', '#']).map_or((after_scheme, ""), |i| after_scheme.split_at(i));
    if rest.len() > 1 {
        out = out.replace(rest, MARK);
    }
    if let Some((userinfo, _)) = authority.rsplit_once('@') {
        if !userinfo.is_empty() {
            out = out.replace(userinfo, MARK);
        }
    }
    out
}

/// Kind and OS error of a transport failure. ureq's own `Display` starts with the URL, so it is
/// never used; the result is scrubbed as well in case a message quotes the URL.
fn transport_message(t: &ureq::Transport, url: &str) -> String {
    use std::error::Error as _;
    let mut message = t.kind().to_string();
    if let Some(source) = t.source() {
        message.push_str(&format!(": {source}"));
    }
    scrub_url(&message, url)
}

/// Webhook endpoints, all optional: nothing is posted to a channel that is not configured.
#[derive(Debug, Clone, Default)]
pub struct ChatConfig {
    pub mattermost_webhook: Option<String>,
    pub slack_webhook: Option<String>,
    /// Mattermost interactive-action callback (receives Approve/Reject clicks).
    pub callback_url: Option<String>,
}

impl ChatConfig {
    /// `AEGIS_MATTERMOST_WEBHOOK_URL`, `AEGIS_SLACK_WEBHOOK_URL`, `AEGIS_APPROVAL_CALLBACK_URL`.
    pub fn from_env() -> Self {
        let var = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        Self {
            mattermost_webhook: var("AEGIS_MATTERMOST_WEBHOOK_URL"),
            slack_webhook: var("AEGIS_SLACK_WEBHOOK_URL"),
            callback_url: var("AEGIS_APPROVAL_CALLBACK_URL"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChangeLine {
    pub kind: &'static str,
    pub path: String,
    pub lines_added: u32,
    pub lines_removed: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApprovalRequest {
    pub branch_id: String,
    pub workspace: String,
    pub agent: String,
    pub risk_score: f32,
    pub decision_allowed: bool,
    pub determining_policies: Vec<String>,
    pub changes: Vec<ChangeLine>,
    /// Redacted +/- excerpt of the change set.
    pub excerpt: String,
}

fn kind_marker(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Deleted => "D",
    }
}

fn excerpt(changes: &ChangeSet) -> String {
    let mut out = String::new();
    for file in &changes.files {
        out.push_str(&format!("{} {} (+{}/-{})\n", kind_marker(file.kind), file.path.display(), file.lines_added, file.lines_removed));
        for line in file.removed_text.lines().take(EXCERPT_LINES_PER_FILE / 2) {
            out.push_str(&format!("- {line}\n"));
        }
        for line in file.added_text.lines().take(EXCERPT_LINES_PER_FILE) {
            out.push_str(&format!("+ {line}\n"));
        }
        if out.len() > MAX_EXCERPT_CHARS {
            let mut cut = MAX_EXCERPT_CHARS;
            while !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
            out.push_str("\n… (truncated)\n");
            break;
        }
    }
    scanner().redact(&out).0
}

impl ApprovalRequest {
    pub fn new(branch_id: &str, workspace: &str, agent: &str, risk_score: f32, decision: &Decision, changes: &ChangeSet) -> Self {
        Self {
            branch_id: branch_id.to_string(),
            workspace: workspace.to_string(),
            agent: agent.to_string(),
            risk_score,
            decision_allowed: decision.allowed,
            determining_policies: decision.determining_policies.clone(),
            changes: changes
                .files
                .iter()
                .map(|f| ChangeLine {
                    kind: kind_marker(f.kind),
                    path: f.path.display().to_string(),
                    lines_added: f.lines_added,
                    lines_removed: f.lines_removed,
                })
                .collect(),
            excerpt: excerpt(changes),
        }
    }

    fn decision_text(&self) -> String {
        match (self.decision_allowed, self.determining_policies.is_empty()) {
            (true, _) => format!("Allow ({})", self.determining_policies.join(", ")),
            (false, true) => "Deny (default: no permit matched)".to_string(),
            (false, false) => format!("Deny ({})", self.determining_policies.join(", ")),
        }
    }

    fn files_text(&self) -> String {
        self.changes
            .iter()
            .map(|c| format!("{} {} (+{}/-{})", c.kind, c.path, c.lines_added, c.lines_removed))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn color(&self) -> &'static str {
        if self.risk_score >= 35.0 {
            "#FF9800"
        } else {
            "#4CAF50"
        }
    }
}

/// Mattermost incoming-webhook payload. Approve/Reject buttons are included only with a callback
/// URL, since Mattermost needs an integration endpoint to deliver clicks.
pub fn mattermost_card(req: &ApprovalRequest, callback_url: Option<&str>) -> Value {
    let mut attachment = json!({
        "fallback": format!("AegisFlow approval needed for {} (risk {:.1})", req.branch_id, req.risk_score),
        "color": req.color(),
        "title": format!("Workspace diff · risk {:.1} / 100", req.risk_score),
        "text": format!("```diff\n{}```", req.excerpt),
        "fields": [
            {"short": true, "title": "Agent", "value": req.agent},
            {"short": true, "title": "Workspace", "value": req.workspace},
            {"short": false, "title": "Cedar", "value": req.decision_text()},
            {"short": false, "title": "Files", "value": req.files_text()},
        ],
    });
    if let Some(url) = callback_url {
        let action = |id: &str, name: &str, style: &str| {
            json!({
                "id": id, "name": name, "style": style,
                "integration": {"url": url, "context": {"action": id, "branch_id": req.branch_id}},
            })
        };
        attachment["actions"] = json!([action("approve", "Approve", "good"), action("reject", "Reject", "danger")]);
    }
    json!({
        "username": "AegisFlow",
        "text": format!("#### Team approval requested: `{}`", req.branch_id),
        "attachments": [attachment],
    })
}

/// Slack incoming-webhook payload (Block Kit).
pub fn slack_card(req: &ApprovalRequest) -> Value {
    let mrkdwn = |text: String| json!({"type": "mrkdwn", "text": text});
    let button = |text: &str, style: &str, action: &str| {
        json!({"type": "button", "text": {"type": "plain_text", "text": text}, "style": style, "action_id": action, "value": req.branch_id})
    };
    json!({
        "text": format!("AegisFlow approval needed for {} (risk {:.1})", req.branch_id, req.risk_score),
        "blocks": [
            {"type": "header", "text": {"type": "plain_text", "text": "AegisFlow · team approval requested"}},
            {"type": "section", "fields": [
                mrkdwn(format!("*Branch*\n`{}`", req.branch_id)),
                mrkdwn(format!("*Risk (ANE model)*\n{:.1} / 100", req.risk_score)),
                mrkdwn(format!("*Agent*\n{}", req.agent)),
                mrkdwn(format!("*Cedar*\n{}", req.decision_text())),
            ]},
            {"type": "section", "text": mrkdwn(format!("*Files*\n```{}```", req.files_text()))},
            {"type": "section", "text": mrkdwn(format!("```{}```", req.excerpt))},
            {"type": "actions", "elements": [
                button("Approve", "primary", "aegis_approve"),
                button("Reject", "danger", "aegis_reject"),
            ]},
        ],
    })
}

/// POSTs a card to a webhook and returns the HTTP status. Error texts never contain `url`.
pub fn post_webhook(agent: &ureq::Agent, url: &str, payload: &Value) -> Result<u16, CardError> {
    match agent.post(url).set("Content-Type", "application/json").send_json(payload.clone()) {
        Ok(response) => Ok(response.status()),
        Err(ureq::Error::Status(status, response)) => {
            // Some servers echo the request path ("Cannot POST /hooks/<token>"): scrub before cutting.
            let body = scrub_url(&response.into_string().unwrap_or_default(), url);
            Err(CardError::Http { status, body: body.chars().take(300).collect() })
        }
        Err(ureq::Error::Transport(t)) => Err(CardError::Transport { message: transport_message(&t, url) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis_storage::FileChange;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;

    fn request(risk: f32, added: &str) -> ApprovalRequest {
        let changes = ChangeSet {
            files: vec![FileChange {
                path: PathBuf::from("src/payments.rs"),
                kind: ChangeKind::Modified,
                old_len: 10,
                new_len: 50,
                is_symlink: false,
                mode_changed: false,
                binary: false,
                lines_added: added.lines().count() as u32,
                lines_removed: 1,
                added_text: added.to_string(),
                removed_text: "let old = 1;".to_string(),
            }],
            scanned_entries: 4,
        };
        let decision = Decision {
            allowed: false,
            determining_policies: vec!["forbid-high-impact-without-team-approval".into()],
            errors: vec![],
            eval_ns: 0,
        };
        ApprovalRequest::new("feature-1", "aegisflow", "core-builder", risk, &decision, &changes)
    }

    #[test]
    fn excerpt_is_redacted() {
        let key = format!("AKIA{}", "IOSFODNN7EXAMPLE");
        let req = request(61.0, &format!("const KEY: &str = \"{key}\";\nnotify(\"ops@example.com\");"));
        assert!(!req.excerpt.contains(&key) && !req.excerpt.contains("ops@example.com"), "{}", req.excerpt);
        assert!(req.excerpt.contains("[REDACTED]"));
        assert!(req.excerpt.starts_with("M src/payments.rs (+2/-1)"));
    }

    #[test]
    fn mattermost_actions_need_a_callback() {
        let req = request(61.0, "fn charge() {}");
        let plain = mattermost_card(&req, None);
        assert!(plain["attachments"][0].get("actions").is_none());
        let interactive = mattermost_card(&req, Some("https://hooks.example/aegis"));
        let actions = interactive["attachments"][0]["actions"].as_array().unwrap();
        assert_eq!(actions[0]["integration"]["context"]["branch_id"], "feature-1");
        assert_eq!(interactive["attachments"][0]["color"], "#FF9800");
    }

    #[test]
    fn slack_card_has_approve_and_reject() {
        let card = slack_card(&request(12.0, "fn charge() {}"));
        let blocks = card["blocks"].as_array().unwrap();
        let actions = blocks.iter().find(|b| b["type"] == "actions").unwrap();
        let ids: Vec<&str> = actions["elements"].as_array().unwrap().iter().map(|e| e["action_id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["aegis_approve", "aegis_reject"]);
        for block in blocks.iter().filter(|b| b["type"] == "section") {
            if let Some(text) = block["text"]["text"].as_str() {
                assert!(text.len() <= 3000);
            }
        }
    }

    /// Serves one request; `respond` maps the request line to (status line, body).
    fn serve_once(
        path: &str,
        respond: fn(&str) -> (&'static str, String),
    ) -> (String, std::thread::JoinHandle<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}{path}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut head, mut length) = (String::new(), 0usize);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let (status, reply) = respond(head.lines().next().unwrap_or_default());
            let mut stream = stream;
            write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            (head, String::from_utf8(body).unwrap())
        });
        (url, server)
    }

    /// URL of a local port with no listener (bound, then released), so connecting is refused.
    fn closed_port_url(path: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}{path}")
    }

    #[test]
    fn posts_to_a_local_webhook() {
        let (url, server) = serve_once("/hooks/abc", |_| ("200 OK", "ok".to_string()));
        let agent = ureq::AgentBuilder::new().build();
        let status = post_webhook(&agent, &url, &slack_card(&request(40.0, "fn charge() {}"))).unwrap();
        let (head, body) = server.join().unwrap();
        assert_eq!(status, 200);
        assert!(head.starts_with("POST /hooks/abc"));
        assert!(body.contains("aegis_approve"));
    }

    #[test]
    fn unreachable_webhook_error_hides_the_url() {
        let url = closed_port_url("/services/T000/B000/SECRETTOKEN123");
        let agent = ureq::AgentBuilder::new().build();
        let err = post_webhook(&agent, &url, &slack_card(&request(40.0, "fn charge() {}"))).unwrap_err();
        assert!(matches!(err, CardError::Transport { .. }), "{err:?}");
        for text in [err.to_string(), format!("{err:?}")] {
            assert!(!text.contains("SECRETTOKEN123") && !text.contains("/services/"), "{text}");
        }
        assert!(err.to_string().starts_with("webhook unreachable: Connection Failed"), "{err}");
    }

    #[test]
    fn http_error_body_that_echoes_the_path_is_scrubbed() {
        let (url, server) = serve_once("/hooks/SECRETTOKEN456", |request_line| {
            let path = request_line.split_whitespace().nth(1).unwrap_or_default().to_string();
            ("404 Not Found", format!("Cannot POST {path}"))
        });
        let agent = ureq::AgentBuilder::new().build();
        let err = post_webhook(&agent, &url, &slack_card(&request(40.0, "fn charge() {}"))).unwrap_err();
        server.join().unwrap();
        assert!(matches!(err, CardError::Http { status: 404, .. }), "{err:?}");
        for text in [err.to_string(), format!("{err:?}")] {
            assert!(!text.contains("SECRETTOKEN456"), "{text}");
        }
        assert_eq!(err.to_string(), "webhook answered HTTP 404: Cannot POST [webhook]");
    }

    #[test]
    fn scrub_url_covers_path_and_userinfo() {
        let url = "https://bot:hunter2@hooks.example.com/hooks/abc123?x=1";
        let text = format!("failed {url}; path /hooks/abc123?x=1; user bot:hunter2");
        let clean = scrub_url(&text, url);
        assert!(!clean.contains("abc123") && !clean.contains("hunter2"), "{clean}");
        assert_eq!(scrub_url("no url here", "http://127.0.0.1:9/"), "no url here");
    }
}
