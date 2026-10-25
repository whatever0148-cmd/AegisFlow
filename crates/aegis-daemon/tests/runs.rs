//! `GET /api/runs`: the daemon keeps the newest workspace runs with their run messages exactly as
//! broadcast, so a reloaded console can render them again (API.md "Run history").

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use aegis_daemon::runs::RUNS_KEPT;
use axum::http::StatusCode;
use common::{is, stages, wait_for, Fixture};
use serde_json::{json, Value};

const RUN_TIMEOUT: Duration = Duration::from_secs(120);

async fn start(fx: &Fixture, scenario: &str) -> String {
    let (status, body) = fx.json(fx.post("/api/workspace/run", &json!({"scenario": scenario}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["run_id"].as_str().unwrap().to_string()
}

async fn runs(fx: &Fixture) -> Vec<Value> {
    let (status, body) = fx.json(fx.get("/api/runs")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["limit"], RUNS_KEPT);
    body["runs"].as_array().unwrap().clone()
}

fn record<'a>(list: &'a [Value], run_id: &str) -> &'a Value {
    list.iter().find(|r| r["run_id"] == run_id).unwrap_or_else(|| panic!("{run_id} not in /api/runs: {list:#?}"))
}

/// The history record of `run_id` holds exactly the broadcast messages seen for it, with a
/// contiguous per-run `seq` starting at `run.started` = 1.
fn assert_matches_broadcast(rec: &Value, seen: &[Value], run_id: &str) {
    let started = seen.iter().find(|v| is("run.started", run_id)(v)).unwrap();
    assert_eq!(started["seq"], 1, "{started}");
    assert_eq!(rec["scenario"], started["scenario"]);
    assert_eq!(rec["title"], started["title"]);
    assert_eq!(rec["started_at"], started["ts"]);
    let broadcast: Vec<Value> = stages(seen, run_id).into_iter().cloned().collect();
    assert_eq!(rec["stages"].as_array().unwrap(), &broadcast, "{run_id}: stages differ from the broadcast messages");
    let finished: Vec<Value> = seen.iter().filter(|v| is("run.finished", run_id)(v)).cloned().collect();
    assert_eq!(rec["finished"].as_array().unwrap(), &finished, "{run_id}: run.finished differs");
    let last = finished.last().unwrap();
    assert_eq!(rec["outcome"], last["outcome"]);
    assert_eq!(rec["total_ms"], last["total_ms"]);
    assert_eq!(rec["queued_ms"], finished[0]["queued_ms"], "queued_ms comes from the first run.finished");
    let mut seqs: Vec<u64> = std::iter::once(1)
        .chain(broadcast.iter().chain(finished.iter()).map(|m| m["seq"].as_u64().expect("every run message has seq")))
        .collect();
    seqs.sort_unstable();
    let expected: Vec<u64> = (1..=seqs.len() as u64).collect();
    assert_eq!(seqs, expected, "{run_id}: seq not contiguous");
    assert_eq!(rec["last_seq"], *seqs.last().unwrap());
    let in_order: Vec<u64> = rec["stages"].as_array().unwrap().iter().map(|s| s["seq"].as_u64().unwrap()).collect();
    assert!(in_order.windows(2).all(|w| w[0] < w[1]), "{run_id}: stages out of order {in_order:?}");
}

/// The run's `run.finished` that satisfies `pred`, from `seen` or (waiting) from `rx`.
async fn finished(
    rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<str>>,
    seen: &mut Vec<Value>,
    run_id: &str,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let matches = |v: &Value| is("run.finished", run_id)(v) && pred(v);
    if let Some(v) = seen.iter().find(|v| matches(v)) {
        return v.clone();
    }
    wait_for(rx, seen, RUN_TIMEOUT, matches).await
}

/// Waits until every run in `ids` has emitted its post-rollback `reap` stage (it arrives after
/// `run.finished`), so the broadcast and the history can be compared as complete.
async fn wait_for_reaps(rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<str>>, seen: &mut Vec<Value>, ids: &[String]) {
    for id in ids {
        if stages(seen, id).iter().any(|s| s["stage"] == "reap") {
            continue;
        }
        wait_for(rx, seen, Duration::from_secs(30), |v| v["type"] == "run.stage" && v["run_id"] == id.as_str() && v["stage"] == "reap").await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_history_keeps_the_newest_runs_newest_first_with_stages_in_broadcast_order() {
    let fx = Fixture::new("runs-keep", "").await;
    assert!(runs(&fx).await.is_empty());
    let mut rx = fx.subscribe();
    let mut seen = Vec::new();
    let total = RUNS_KEPT + 2;
    let mut ids = Vec::new();
    for _ in 0..total {
        ids.push(start(&fx, "secret_leak").await);
    }
    for id in &ids {
        let done = finished(&mut rx, &mut seen, id, |_| true).await;
        assert_eq!(done["outcome"], "rolled_back", "{done}");
    }
    wait_for_reaps(&mut rx, &mut seen, &ids).await;

    let list = runs(&fx).await;
    let listed: Vec<&str> = list.iter().map(|r| r["run_id"].as_str().unwrap()).collect();
    let mut newest: Vec<&str> = ids.iter().map(String::as_str).skip(total - RUNS_KEPT).collect();
    newest.reverse();
    assert_eq!(listed, newest, "the newest {RUNS_KEPT} runs, newest first");
    for id in &ids[total - RUNS_KEPT..] {
        let rec = record(&list, id);
        assert_matches_broadcast(rec, &seen, id);
        let names: Vec<&str> = rec["stages"].as_array().unwrap().iter().map(|s| s["stage"].as_str().unwrap()).collect();
        assert_eq!(names, ["branch", "edit", "diff", "features", "score", "policy", "verify", "rollback", "reap"], "{id}");
        assert!(rec["queued_ms"].as_f64().is_some(), "{rec}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_history_survives_approval_resolution() {
    let fx = Fixture::new("runs-approval", "").await;
    let mut rx = fx.subscribe();
    let mut seen = Vec::new();

    let held = start(&fx, "mass_delete").await;
    let pending = finished(&mut rx, &mut seen, &held, |_| true).await;
    let score = stages(&seen, &held).into_iter().find(|s| s["stage"] == "score").cloned().unwrap_or(Value::Null);
    assert_eq!(
        pending["outcome"], "pending_approval",
        "mass_delete must reach team approval (risk {} vs threshold {})",
        score["detail"]["risk"],
        aegis_governance::HIGH_IMPACT_THRESHOLD
    );
    let created = seen.iter().find(|v| v["type"] == "approval.created").cloned().expect("approval.created before run.finished");
    let id = created["approval"]["id"].as_str().unwrap().to_string();

    // Enough newer runs to push the pending one past the limit: it stays (its continuation is due).
    let mut others = Vec::new();
    for _ in 0..RUNS_KEPT {
        others.push(start(&fx, "secret_leak").await);
    }
    for run in &others {
        finished(&mut rx, &mut seen, run, |_| true).await;
    }
    let list = runs(&fx).await;
    assert_eq!(list.len(), RUNS_KEPT);
    assert_eq!(list[0]["run_id"], others.last().unwrap().as_str(), "newest first");
    assert_eq!(record(&list, &held)["outcome"], "pending_approval");
    assert!(list.iter().all(|r| r["run_id"] != others[0].as_str()), "the oldest finished run is the one evicted");

    let (status, resolved) = fx.json(fx.post(&format!("/api/approvals/{id}/approve"), &json!({"by": "test-lead"}))).await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    let done = finished(&mut rx, &mut seen, &held, |v| v["outcome"] != "pending_approval").await;
    let expected_outcome = if resolved["result"]["action"] == "commit" { "committed" } else { "rolled_back" };
    assert_eq!(done["outcome"], expected_outcome, "{resolved}");
    if expected_outcome == "rolled_back" {
        wait_for_reaps(&mut rx, &mut seen, std::slice::from_ref(&held)).await;
    }

    let list = runs(&fx).await;
    let rec = record(&list, &held);
    assert_matches_broadcast(rec, &seen, &held);
    let both = rec["finished"].as_array().unwrap();
    assert_eq!(both.len(), 2);
    assert_eq!(both[0]["outcome"], "pending_approval");
    assert!(both[1]["queued_ms"].is_null());
    assert!(rec["queued_ms"].as_f64().is_some(), "the first run.finished's queued_ms is kept: {rec}");
    let names: Vec<&str> = rec["stages"].as_array().unwrap().iter().map(|s| s["stage"].as_str().unwrap()).collect();
    let resumed = names.iter().position(|n| *n == "approval").unwrap();
    assert_eq!(&names[..resumed], ["branch", "edit", "diff", "features", "score", "policy", "verify"]);
    let statuses: BTreeMap<usize, &str> = rec["stages"].as_array().unwrap().iter().enumerate().filter(|(_, s)| s["stage"] == "approval").map(|(i, s)| (i, s["status"].as_str().unwrap())).collect();
    assert_eq!(statuses.values().copied().collect::<Vec<_>>(), ["pending", "ok"], "approval pending, then approved");
    let after: Vec<&str> = names[resumed + 2..].to_vec();
    assert_eq!(after[0], "policy", "{names:?}");
    assert!(["commit", "rollback"].contains(&after[1]), "{names:?}");
    let resumed_policy = &rec["stages"][resumed + 2];
    assert_eq!(resumed_policy["detail"]["team_approval_granted"], true);
}
