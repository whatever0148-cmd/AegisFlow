//! End-to-end workspace scenarios against a temporary demo workspace, asserting the live tree.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::{is, stage, stages, wait_for, Fixture};
use serde_json::{json, Value};

const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// A quoted 32-character alphanumeric value after `api_key` (what `demo::secret_leak` writes).
fn regex_free_secret(text: &str) -> Option<String> {
    text.lines().find(|l| {
        l.split('"').any(|part| part.len() == 32 && part.chars().all(|c| c.is_ascii_alphanumeric())) && l.contains("api_key")
    }).map(str::to_string)
}

fn read(live: &Path, rel: &str) -> String {
    std::fs::read_to_string(live.join(rel)).unwrap_or_default()
}

async fn start(fx: &Fixture, scenario: &str) -> String {
    let (status, body) = fx.json(fx.post("/api/workspace/run", &json!({"scenario": scenario}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["run_id"].as_str().unwrap().to_string()
}

fn print_run(seen: &[Value], run_id: &str) {
    for s in stages(seen, run_id) {
        eprintln!("  {run_id} {:<9} {:<7} ms={:<12} {}", s["stage"].as_str().unwrap(), s["status"].as_str().unwrap(), s["ms"], s["detail"]);
    }
}

/// Reads one HTTP request (head and body) from `stream`.
fn read_request(stream: &TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let (mut request, mut length) = (String::new(), 0usize);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
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
    request
}

/// Patch that undoes build_break's edit, with the line number computed from the demo template.
fn build_break_fix() -> String {
    let root = Fixture::scratch(&format!("recover-template-{}", std::thread::current().name().unwrap_or("t").replace("::", "-")));
    aegis_daemon::demo::create(&root).unwrap();
    let report = std::fs::read_to_string(root.join("src/report.rs")).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let lines: Vec<&str> = report.lines().collect();
    let idx = lines.iter().position(|l| l.contains("format_cents(order_amount(lines))")).unwrap();
    let broken = lines[idx].replace("format_cents(order_amount(lines))", "format_cents(order_amount(lines), CURRENCY)");
    format!(
        "--- a/src/report.rs\n+++ b/src/report.rs\n@@ -{start},3 +{start},3 @@\n {before}\n-{broken}\n+{fixed}\n {after}\n",
        start = idx,
        before = lines[idx - 1],
        broken = broken,
        fixed = lines[idx],
        after = lines[idx + 1],
    )
}

/// OpenAI-compatible endpoint whose first `/chat/completions` reply is held until `release`
/// receives a message; `arrived` gets one message per request as it comes in. Later requests are
/// answered at once. Stops when `stop` is set and a connection wakes it.
fn gated_nebius(diff: String, stop: Arc<AtomicBool>) -> (String, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (arrived_tx, arrived) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut first = true;
        for stream in listener.incoming() {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            let mut stream = stream.unwrap();
            let request = read_request(&stream);
            let _ = arrived_tx.send(());
            if first {
                first = false;
                let _ = release_rx.recv();
            }
            let body = if request.starts_with("POST /v1/chat/completions") {
                json!({"choices": [{"message": {"role": "assistant", "content": format!("```diff\n{diff}```")}}]}).to_string()
            } else {
                json!({"data": []}).to_string()
            };
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    (format!("http://{addr}/v1"), arrived, release)
}

/// Serves an OpenAI-compatible `/chat/completions` reply carrying `diff` to every request until
/// `stop` is set; returns the raw requests.
fn mock_nebius(diff: String, stop: Arc<AtomicBool>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        listener.set_nonblocking(true).unwrap();
        while !stop.load(Ordering::SeqCst) {
            let stream = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => panic!("accept: {e}"),
            };
            stream.set_nonblocking(false).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut request, mut length) = (String::new(), 0usize);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut payload = vec![0u8; length];
            reader.read_exact(&mut payload).unwrap();
            request.push_str(&String::from_utf8_lossy(&payload));
            let content = format!("```diff\n{diff}```");
            let body = json!({"choices": [{"message": {"role": "assistant", "content": content}}]}).to_string();
            let mut stream = stream;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            requests.push(request);
        }
        requests
    });
    (format!("http://{addr}/v1"), handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn workspace_scenarios_commit_rollback_and_approvals() {
    let fx = Fixture::new("e2e", "").await;
    let live = fx.live();
    let mut rx = fx.subscribe();
    let mut seen = Vec::new();
    assert!(read(&live, "src/pricing.rs").contains("pub fn compute_total("));

    // safe_refactor -> commit
    let run = start(&fx, "safe_refactor").await;
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &run)).await;
    print_run(&seen, &run);
    assert_eq!(finished["outcome"], "committed", "{:#?}", stages(&seen, &run));
    assert!(stage(&seen, &run, "branch").unwrap()["detail"]["source"].is_string());
    assert_eq!(stage(&seen, &run, "policy").unwrap()["detail"]["allowed"], true);
    assert_eq!(stage(&seen, &run, "verify").unwrap()["status"], "ok");
    let commit = stage(&seen, &run, "commit").unwrap();
    assert!(commit["ms"].as_f64().unwrap() > 0.0);
    for rel in ["src/pricing.rs", "src/inventory.rs", "src/orders.rs", "src/report.rs", "src/lib.rs"] {
        let text = read(&live, rel);
        assert!(text.contains("order_total") && !text.contains("compute_total"), "{rel} not refactored:\n{text}");
    }
    let edit = stage(&seen, &run, "edit").unwrap();
    assert_eq!(edit["detail"]["files_modified"], 5);
    // The diff stage carries the branch's unified diff for the console's diff viewer.
    let view = &stage(&seen, &run, "diff").unwrap()["detail"]["unified_diff"];
    let text = view["text"].as_str().unwrap();
    assert_eq!(view["format"], "unified");
    assert!(text.contains("diff --git a/src/pricing.rs b/src/pricing.rs\n--- a/src/pricing.rs\n+++ b/src/pricing.rs\n@@ -"), "{text}");
    assert!(text.lines().any(|l| l.starts_with('-') && l.contains("compute_total")), "{text}");
    assert!(text.lines().any(|l| l.starts_with('+') && l.contains("order_total")), "{text}");
    assert_eq!(view["files"], 5, "{view}");
    assert_eq!(view["files_total"], 5);
    assert_eq!(view["bytes"].as_u64().unwrap() as usize, text.len());
    assert!(text.len() <= view["cap_bytes"].as_u64().unwrap() as usize);
    assert_eq!(view["truncated"], false);
    let (_, history) = fx.json(fx.get("/api/runs")).await;
    let record = history["runs"].as_array().unwrap().iter().find(|r| r["run_id"] == run.as_str()).unwrap();
    let kept = record["stages"].as_array().unwrap().iter().find(|s| s["stage"] == "diff").unwrap();
    assert_eq!(kept["detail"]["unified_diff"], *view, "GET /api/runs keeps the diff as broadcast");
    let score = stage(&seen, &run, "score").unwrap();
    assert!(score["detail"]["risk"].as_f64().unwrap() < 35.0);
    assert_eq!(score["detail"]["units"], "cpuAndNeuralEngine");

    // secret_leak -> Cedar deny -> rollback
    let config_before = read(&live, "config/app.toml");
    let run = start(&fx, "secret_leak").await;
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &run)).await;
    print_run(&seen, &run);
    assert_eq!(finished["outcome"], "rolled_back");
    let features = stage(&seen, &run, "features").unwrap();
    assert_eq!(features["detail"]["unformatted_sensitive_fields"], true);
    let policy = stage(&seen, &run, "policy").unwrap();
    assert_eq!(policy["detail"]["allowed"], false);
    assert_eq!(policy["detail"]["has_unformatted_sensitive_fields"], true);
    assert_eq!(stage(&seen, &run, "rollback").unwrap()["status"], "ok");
    // The leaked value never reaches the diff viewer: the same redaction as the approval excerpt.
    let view = &stage(&seen, &run, "diff").unwrap()["detail"]["unified_diff"];
    let text = view["text"].as_str().unwrap();
    assert!(text.contains("+++ b/config/app.toml"), "{text}");
    assert!(text.contains("[REDACTED]"), "{text}");
    assert!(view["redactions"].as_u64().unwrap() >= 1, "{view}");
    let leaked = regex_free_secret(text);
    assert!(leaked.is_none(), "unredacted api_key in the diff: {leaked:?}");
    assert_eq!(read(&live, "config/app.toml"), config_before, "live config changed by a rolled-back branch");
    assert!(!read(&live, "config/app.toml").contains("api_key"));
    // The detached branch is deleted by the background reaper and reported as a reap stage.
    let reap = wait_for(&mut rx, &mut seen, Duration::from_secs(20), |v| v["type"] == "run.stage" && v["run_id"] == run.as_str() && v["stage"] == "reap").await;
    assert_eq!(reap["status"], "ok");
    assert!(reap["ms"].as_f64().unwrap() > 0.0);

    // mass_delete + reject -> rollback, files stay
    let fixtures_before = std::fs::read_dir(live.join("data/fixtures")).unwrap().count();
    assert!(fixtures_before >= 32);
    let run = start(&fx, "mass_delete").await;
    let created = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, |v| v["type"] == "approval.created").await;
    let pending = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &run)).await;
    print_run(&seen, &run);
    assert_eq!(pending["outcome"], "pending_approval");
    // The disclosed comparison: the same change scored as one event, measured in this run.
    let score = stage(&seen, &run, "score").unwrap();
    assert!(score["detail"]["single_event_risk"].as_f64().is_some(), "{score}");
    assert!(score["detail"]["window_events"].as_u64().unwrap() > 1, "{score}");
    assert!(pending["queued_ms"].as_f64().is_some(), "{pending}");
    let approval = &created["approval"];
    assert_eq!(approval["run_id"], run.as_str());
    assert_eq!(approval["status"], "pending");
    // Cedar sees the risk rounded to an integer (risk_to_context): high impact means >= 34.5.
    assert!(approval["risk"].as_f64().unwrap() >= 34.5, "{approval}");
    assert_eq!(approval["determining_policies"], json!(["forbid-high-impact-without-team-approval"]));
    assert_eq!(approval["cards"], json!({"mattermost": "disabled", "slack": "disabled"}));
    let id = approval["id"].as_str().unwrap().to_string();
    let (_, list) = fx.json(fx.get("/api/approvals")).await;
    assert_eq!(list["pending"][0]["id"], id.as_str());
    let (status, resolved) = fx.json(fx.post(&format!("/api/approvals/{id}/reject"), &json!({"by": "test-reviewer"}))).await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["status"], "rejected");
    assert_eq!(resolved["resolved_by"], "test-reviewer");
    assert_eq!(resolved["result"]["action"], "rollback");
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, |v| v["type"] == "run.finished" && v["run_id"] == run.as_str() && v["outcome"] != "pending_approval").await;
    assert_eq!(finished["outcome"], "rolled_back");
    assert_eq!(std::fs::read_dir(live.join("data/fixtures")).unwrap().count(), fixtures_before);
    assert!(live.join("docs/architecture.md").is_file());
    let (status, again) = fx.json(fx.post(&format!("/api/approvals/{id}/approve"), &json!({"by": "late"}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");
    let (status, _) = fx.json(fx.post("/api/approvals/a-999999/approve", &json!({"by": "x"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // mass_delete + approve -> Cedar re-evaluated with team approval -> commit
    let run = start(&fx, "mass_delete").await;
    let created = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, |v| v["type"] == "approval.created").await;
    wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &run)).await;
    let id = created["approval"]["id"].as_str().unwrap().to_string();
    let (status, resolved) = fx.json(fx.post(&format!("/api/approvals/{id}/approve"), &json!({"by": "test-lead"}))).await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["status"], "approved");
    assert_eq!(resolved["result"]["action"], "commit", "{resolved}");
    assert_eq!(resolved["result"]["detail"]["determining_policies"], json!(["permit-approved-high-impact-commit"]));
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, |v| v["type"] == "run.finished" && v["run_id"] == run.as_str() && v["outcome"] != "pending_approval").await;
    print_run(&seen, &run);
    assert_eq!(finished["outcome"], "committed");
    let policies: Vec<&Value> = stages(&seen, &run).into_iter().filter(|s| s["stage"] == "policy").collect();
    assert_eq!(policies.last().unwrap()["detail"]["team_approval_granted"], true);
    assert!(!live.join("data").exists(), "data/ still in live after the approved mass delete");
    assert!(!live.join("docs").exists());
    assert!(read(&live, "README.md").contains("removed by the cleanup agent"));
    assert!(read(&live, "src/pricing.rs").contains("order_total"), "earlier commit lost");

    // build_break without keys: tier 0 runs, tiers 1-2 are reported disabled, rollback.
    let report_before = read(&live, "src/report.rs");
    let run = start(&fx, "build_break").await;
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &run)).await;
    print_run(&seen, &run);
    assert_eq!(finished["outcome"], "rolled_back");
    let verify = stage(&seen, &run, "verify").unwrap();
    assert_eq!(verify["status"], "fail");
    assert!(verify["detail"]["errors"].as_u64().unwrap() >= 1);
    let recovers: Vec<&Value> = stages(&seen, &run).into_iter().filter(|s| s["stage"] == "recover").collect();
    assert_eq!(recovers.len(), 3, "{recovers:#?}");
    assert_eq!((recovers[0]["detail"]["tier"].clone(), recovers[0]["status"].clone()), (json!(0), json!("ok")));
    assert!(recovers[0]["ms"].as_f64().unwrap() > 0.0);
    for (r, tier) in recovers[1..].iter().zip([1, 2]) {
        assert_eq!(r["detail"]["tier"], tier);
        assert_eq!(r["status"], "skip");
        assert_eq!(r["detail"]["enabled"], false);
        assert_eq!(r["detail"]["reason"], "NEBIUS_API_KEY not set");
    }
    assert_eq!(recovers[2]["detail"]["tavily"]["reason"], "TAVILY_API_KEY not set");
    assert!(recovers.iter().all(|r| r["detail"].get("patch_diff").is_none()), "no patch without a remote tier");
    let view = &stage(&seen, &run, "diff").unwrap()["detail"]["unified_diff"];
    assert!(view["text"].as_str().unwrap().contains("+++ b/src/report.rs"), "{view}");
    assert_eq!(read(&live, "src/report.rs"), report_before);

    // Counters and live stats come from these runs.
    let live_stats = fx.state.live_json();
    assert_eq!(live_stats["counters"]["runs"], 5);
    assert_eq!(live_stats["counters"]["commits"], 2);
    assert_eq!(live_stats["counters"]["rollbacks"], 3);
    assert!(live_stats["samples"]["ane_score_ms"]["n"].as_u64().unwrap() >= 5);
    assert_eq!(live_stats["samples"]["commit_swap_ms"]["n"], 2);

    // Reset rebuilds the demo crate.
    let (status, ws) = fx.json(fx.post("/api/workspace/reset", &json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{ws}");
    assert!(live.join("data/fixtures").is_dir());
    assert!(read(&live, "src/pricing.rs").contains("pub fn compute_total("));
    assert_eq!(ws["versions"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn build_break_recovers_through_local_openai_compatible_endpoint() {
    let diff = build_break_fix();
    let stop = Arc::new(AtomicBool::new(false));
    let (base_url, server) = mock_nebius(diff, stop.clone());
    let fx = Fixture::new("recover", &format!("NEBIUS_API_KEY=test-key-local-only\nNEBIUS_BASE_URL={base_url}\n")).await;
    let live = fx.live();
    let report_before = read(&live, "src/report.rs");
    let mut rx = fx.subscribe();
    let mut seen = Vec::new();
    let run = start(&fx, "build_break").await;
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &run)).await;
    stop.store(true, Ordering::SeqCst);
    let requests = server.join().unwrap();
    print_run(&seen, &run);
    let recovers: Vec<&Value> = stages(&seen, &run).into_iter().filter(|s| s["stage"] == "recover").collect();
    assert_eq!(recovers[0]["detail"]["tier"], 0);
    let remote: Vec<&&Value> = recovers.iter().filter(|r| r["detail"]["enabled"] == true && r["detail"]["tier"] != 0 && r["status"] != "skip").collect();
    assert!(!remote.is_empty(), "{recovers:#?}");
    assert_eq!(remote[0]["detail"]["patch_applied"], true, "{:#?}", remote[0]);
    // The committed tier carries the repaired-file diff: the branch before the patch against after.
    let committed = recovers.iter().find(|r| r["detail"]["committed_to_branch"] == true).expect("a tier committed its patch");
    let patch = &committed["detail"]["patch_diff"];
    let text = patch["text"].as_str().unwrap_or_else(|| panic!("{committed:#}"));
    assert!(text.contains("--- a/src/report.rs\n+++ b/src/report.rs\n@@ -"), "{text}");
    assert!(text.lines().any(|l| l.starts_with('-') && l.contains("format_cents(order_amount(lines), CURRENCY)")), "{text}");
    assert!(text.lines().any(|l| l.starts_with('+') && l.contains("format_cents(order_amount(lines))")), "{text}");
    assert_eq!(patch["files"], 1, "{patch}");
    assert!(!text.contains("test-key-local-only"));
    assert!(recovers.iter().filter(|r| r["detail"].get("patch_diff").is_some()).count() == 1);
    assert_eq!(finished["outcome"], "recovered", "{:#?}", stages(&seen, &run));
    let verifies: Vec<&Value> = stages(&seen, &run).into_iter().filter(|s| s["stage"] == "verify").collect();
    assert_eq!(verifies.len(), 2);
    assert_eq!((verifies[0]["status"].clone(), verifies[1]["status"].clone()), (json!("fail"), json!("ok")));
    assert_eq!(read(&live, "src/report.rs"), report_before, "the recovered commit should leave report.rs compiling as before");
    assert!(requests[0].starts_with("POST /v1/chat/completions"), "{}", requests[0]);
    assert!(requests[0].contains("Bearer test-key-local-only"));
    assert!(!fx.root.join("demo/.recovery").join(&run).exists(), "recovery store left behind");
}

/// A commit whose branch is no longer based on the live generation goes through the normal
/// rollback path: `commit` fail (conflict), then `rollback` with its detach time, then `reap`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approved_branch_that_conflicts_is_rolled_back_with_a_measured_detach() {
    let fx = Fixture::new("conflict", "").await;
    let live = fx.live();
    let mut rx = fx.subscribe();
    let mut seen = Vec::new();
    let held = start(&fx, "mass_delete").await;
    let created = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, |v| v["type"] == "approval.created").await;
    wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &held)).await;
    let id = created["approval"]["id"].as_str().unwrap().to_string();
    let run = start(&fx, "safe_refactor").await;
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &run)).await;
    assert_eq!(finished["outcome"], "committed");

    let (status, resolved) = fx.json(fx.post(&format!("/api/approvals/{id}/approve"), &json!({"by": "test-lead"}))).await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["status"], "approved");
    assert_eq!(resolved["result"]["action"], "rollback", "{resolved}");
    assert_eq!(resolved["result"]["detail"]["conflict"], true, "{resolved}");
    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, |v| v["type"] == "run.finished" && v["run_id"] == held.as_str() && v["outcome"] != "pending_approval").await;
    assert_eq!(finished["outcome"], "rolled_back");
    let reap = wait_for(&mut rx, &mut seen, Duration::from_secs(20), |v| v["type"] == "run.stage" && v["run_id"] == held.as_str() && v["stage"] == "reap").await;
    print_run(&seen, &held);
    assert_eq!(reap["status"], "ok");
    let after: Vec<&Value> = stages(&seen, &held).into_iter().skip_while(|s| s["stage"] != "approval" || s["status"] == "pending").collect();
    let names: Vec<&str> = after.iter().map(|s| s["stage"].as_str().unwrap()).collect();
    assert_eq!(names, ["approval", "policy", "commit", "rollback", "reap"], "{after:#?}");
    assert_eq!((after[2]["status"].clone(), after[2]["detail"]["conflict"].clone()), (json!("fail"), json!(true)));
    assert_eq!(after[3]["status"], "ok");
    assert!(after[3]["ms"].as_f64().unwrap() > 0.0, "rollback reports its detach time");
    let live_stats = fx.state.live_json();
    assert_eq!(live_stats["counters"]["rollbacks"], 1);
    assert_eq!(live_stats["samples"]["rollback_detach_ms"]["n"], 1);
    // The mass delete never reached live; the earlier refactor did.
    assert!(live.join("data/fixtures").is_dir());
    assert!(read(&live, "src/pricing.rs").contains("order_total"));
    let (status, _) = fx.json(fx.post(&format!("/api/approvals/{id}/approve"), &json!({"by": "late"}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

/// While a run holds the run queue inside a (held) remote recovery: an approval resolves at once,
/// and a run queued behind it reports the wait as `queued_ms`, not inside `total_ms`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approvals_do_not_wait_for_a_running_scenario_and_queue_time_is_separate() {
    let stop = Arc::new(AtomicBool::new(false));
    let (base_url, arrived, release) = gated_nebius(build_break_fix(), stop.clone());
    let fx = Fixture::new("queue", &format!("NEBIUS_API_KEY=test-key-local-only\nNEBIUS_BASE_URL={base_url}\n")).await;
    let live = fx.live();
    let mut rx = fx.subscribe();
    let mut seen = Vec::new();

    let held = start(&fx, "mass_delete").await;
    let created = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, |v| v["type"] == "approval.created").await;
    wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &held)).await;
    let id = created["approval"]["id"].as_str().unwrap().to_string();

    let breaking = start(&fx, "build_break").await;
    let arrived = tokio::task::spawn_blocking(move || arrived.recv_timeout(RUN_TIMEOUT).map(|()| arrived)).await.unwrap().expect("recovery reached the endpoint");
    let queued = start(&fx, "safe_refactor").await;
    wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.started", &queued)).await;
    let t0 = Instant::now();

    // build_break is now inside the held remote call, with the run queue held.
    let approve = fx.post(&format!("/api/approvals/{id}/approve"), &json!({"by": "test-lead"}));
    let (status, resolved) = tokio::time::timeout(Duration::from_secs(60), fx.json(approve)).await.expect("approval waited for the running scenario");
    assert_eq!(status, StatusCode::OK, "{resolved}");
    assert_eq!(resolved["result"]["action"], "commit", "{resolved}");
    assert!(!live.join("data").exists(), "the approved mass delete is live");
    let t1 = Instant::now();
    release.send(()).unwrap();

    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &breaking)).await;
    print_run(&seen, &breaking);
    // Its branch predates the approved commit, so it cannot commit any more.
    assert_eq!(finished["outcome"], "rolled_back", "{:#?}", stages(&seen, &breaking));
    assert!(stages(&seen, &breaking).iter().all(|s| !(s["stage"] == "commit" && s["status"] == "ok")));

    let finished = wait_for(&mut rx, &mut seen, RUN_TIMEOUT, is("run.finished", &queued)).await;
    let finished_at = Instant::now();
    print_run(&seen, &queued);
    assert_eq!(finished["outcome"], "committed", "{:#?}", stages(&seen, &queued));
    let (queued_ms, total_ms) = (finished["queued_ms"].as_f64().unwrap(), finished["total_ms"].as_f64().unwrap());
    let held_ms = (t1 - t0).as_secs_f64() * 1e3;
    let after_release_ms = (finished_at - t1).as_secs_f64() * 1e3;
    // The run could not take the queue before t1 and was requested before t0 ...
    assert!(queued_ms >= held_ms, "queued_ms {queued_ms} < time the queue was certainly held {held_ms}");
    // ... and total_ms starts after it took the queue (after t1) and ends before we saw run.finished.
    assert!(total_ms < after_release_ms, "total_ms {total_ms} includes queue time (finished {after_release_ms} ms after release)");
    let stage_sum: f64 = stages(&seen, &queued).iter().filter_map(|s| s["ms"].as_f64()).sum();
    assert!(total_ms >= stage_sum, "total_ms {total_ms} < sum of its stages {stage_sum}");

    stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(base_url.trim_start_matches("http://").trim_end_matches("/v1"));
    drop(arrived);
}

/// Branches an earlier daemon left in the store (a pending approval when it was killed) are
/// discarded at startup, and a second daemon cannot open the same demo directory.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn startup_discards_leftover_branches_and_locks_the_demo_directory() {
    let root = Fixture::scratch("orphans");
    std::fs::write(root.join(".env"), "").unwrap();
    let demo = root.join("demo");
    aegis_daemon::demo::create(&demo.join("live")).unwrap();
    let orphan = demo.join(".aegis-live/branches/mass_delete-99999-0-deadbeef");
    aegis_daemon::demo::create(&orphan).unwrap();
    let fx = Fixture::with_options(root.clone(), Fixture::options(&root)).await;
    assert_eq!(fx.state.workspace.startup_discarded, 1);
    assert!(!orphan.exists(), "leftover branch still in branches/");
    let second = aegis_daemon::workspace::WorkspaceEngine::open(&demo, 1);
    let err = second.err().expect("a second engine on the same demo directory must be refused");
    assert!(err.contains("another aegis-daemon"), "{err}");
}
