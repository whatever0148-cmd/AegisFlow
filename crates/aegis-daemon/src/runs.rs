//! In-memory history of the most recent workspace runs (API.md `GET /api/runs`).
//!
//! Fed by [`crate::state::Events::send`] with every `run.started`, `run.stage` and `run.finished`
//! message, under the same lock as the broadcast, so the history holds each message exactly as it
//! was broadcast and in broadcast order. Each run's messages carry a per-run `seq` (1 for
//! `run.started`, then +1 per `run.stage` / `run.finished`, contiguous), which clients use to merge
//! this history with live WebSocket messages without duplicates.
//!
//! Records are ordered by run id (`r-000001`, …: request order; `run.started` of concurrent
//! requests can be broadcast in a different order). Eviction keeps [`RUNS_KEPT`] records: the
//! oldest run goes first, except that a run whose
//! latest outcome is `pending_approval` is skipped while any other run can go, so the post-approval
//! continuation (second `approval`, `policy`, `commit`/`rollback`, second `run.finished`) lands in a
//! record that still has the run's first half. Messages for a run that is no longer kept are
//! broadcast without `seq` and not recorded.

use std::collections::VecDeque;

use serde_json::{json, Map, Value};

/// Workspace runs returned by `GET /api/runs`.
pub const RUNS_KEPT: usize = 10;

/// Message types recorded here.
pub const RUN_MESSAGES: [&str; 3] = ["run.started", "run.stage", "run.finished"];

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunRecord {
    pub run_id: String,
    pub scenario: Value,
    pub title: Value,
    /// `ts` of the `run.started` message.
    pub started_at: Value,
    /// Latest `run.finished` outcome (`null` while the run is in progress).
    pub outcome: Value,
    /// Latest `run.finished` `total_ms`.
    pub total_ms: Value,
    /// The non-null `queued_ms` of a `run.finished` (only the first one carries it).
    pub queued_ms: Value,
    /// `run.stage` messages exactly as broadcast, in order.
    pub stages: Vec<Value>,
    /// `run.finished` messages exactly as broadcast, in order (two after an approval resolution).
    pub finished: Vec<Value>,
    /// Highest `seq` broadcast for this run.
    pub last_seq: u64,
}

#[derive(Debug, Default)]
pub struct RunHistory {
    /// Oldest run id first.
    runs: VecDeque<RunRecord>,
}

/// Ordering key of a run id: its number (`r-000042` → 42), so ids past six digits still sort.
fn run_number(run_id: &str) -> u64 {
    let digits: String = run_id.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().unwrap_or(0)
}

impl RunHistory {
    pub fn tracks(kind: &str) -> bool {
        RUN_MESSAGES.contains(&kind)
    }

    fn find(&mut self, run_id: &str) -> Option<&mut RunRecord> {
        self.runs.iter_mut().find(|r| r.run_id == run_id)
    }

    /// Assigns the message's `seq` (inserted into `msg`) and records a copy. `msg` is the complete
    /// message object (`type` and `ts` included) about to be broadcast.
    pub fn observe(&mut self, msg: &mut Map<String, Value>) {
        let kind = msg.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(run_id) = msg.get("run_id").and_then(Value::as_str).map(str::to_string) else { return };
        match kind.as_str() {
            "run.started" => {
                msg.insert("seq".into(), json!(1));
                self.runs.retain(|r| r.run_id != run_id);
                let key = run_number(&run_id);
                let at = self.runs.iter().position(|r| run_number(&r.run_id) > key).unwrap_or(self.runs.len());
                self.runs.insert(at, RunRecord {
                    run_id,
                    scenario: msg.get("scenario").cloned().unwrap_or(Value::Null),
                    title: msg.get("title").cloned().unwrap_or(Value::Null),
                    started_at: msg.get("ts").cloned().unwrap_or(Value::Null),
                    outcome: Value::Null,
                    total_ms: Value::Null,
                    queued_ms: Value::Null,
                    stages: Vec::new(),
                    finished: Vec::new(),
                    last_seq: 1,
                });
                self.evict();
            }
            "run.stage" => {
                let Some(record) = self.find(&run_id) else { return };
                record.last_seq += 1;
                msg.insert("seq".into(), json!(record.last_seq));
                record.stages.push(Value::Object(msg.clone()));
            }
            "run.finished" => {
                let Some(record) = self.find(&run_id) else { return };
                record.last_seq += 1;
                msg.insert("seq".into(), json!(record.last_seq));
                record.outcome = msg.get("outcome").cloned().unwrap_or(Value::Null);
                record.total_ms = msg.get("total_ms").cloned().unwrap_or(Value::Null);
                if let Some(queued) = msg.get("queued_ms").filter(|v| !v.is_null()) {
                    record.queued_ms = queued.clone();
                }
                record.finished.push(Value::Object(msg.clone()));
            }
            _ => {}
        }
    }

    fn evict(&mut self) {
        while self.runs.len() > RUNS_KEPT {
            let idx = self.runs.iter().position(|r| r.outcome != "pending_approval").unwrap_or(0);
            self.runs.remove(idx);
        }
    }

    /// `GET /api/runs`: newest run id first.
    pub fn list_json(&self) -> Value {
        let runs: Vec<&RunRecord> = self.runs.iter().rev().collect();
        json!({"runs": runs, "limit": RUNS_KEPT})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: &str, fields: Value) -> Map<String, Value> {
        let mut obj = fields.as_object().cloned().unwrap_or_default();
        obj.insert("type".into(), json!(kind));
        obj.insert("ts".into(), json!(crate::util::now_iso()));
        obj
    }

    fn observe(h: &mut RunHistory, kind: &str, fields: Value) -> Map<String, Value> {
        let mut msg = message(kind, fields);
        h.observe(&mut msg);
        msg
    }

    fn start(h: &mut RunHistory, id: &str) -> Map<String, Value> {
        observe(h, "run.started", json!({"run_id": id, "scenario": "secret_leak", "title": "t"}))
    }

    fn finish(h: &mut RunHistory, id: &str, outcome: &str, queued: Value) -> Map<String, Value> {
        observe(h, "run.finished", json!({"run_id": id, "outcome": outcome, "total_ms": null, "queued_ms": queued}))
    }

    fn ids(h: &RunHistory) -> Vec<String> {
        h.list_json()["runs"].as_array().unwrap().iter().map(|r| r["run_id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn seq_is_contiguous_per_run_and_messages_are_recorded_verbatim() {
        let mut h = RunHistory::default();
        assert_eq!(start(&mut h, "r-1")["seq"], 1);
        assert_eq!(start(&mut h, "r-2")["seq"], 1);
        let a = observe(&mut h, "run.stage", json!({"run_id": "r-1", "stage": "branch", "status": "ok", "ms": null, "detail": {}}));
        let b = observe(&mut h, "run.stage", json!({"run_id": "r-2", "stage": "branch", "status": "ok", "ms": null, "detail": {}}));
        let c = observe(&mut h, "run.stage", json!({"run_id": "r-1", "stage": "edit", "status": "ok", "ms": null, "detail": {}}));
        let queued = json!(crate::util::ms(std::time::Duration::from_micros(1500)));
        let f = finish(&mut h, "r-1", "rolled_back", queued.clone());
        assert_eq!((a["seq"].clone(), b["seq"].clone(), c["seq"].clone(), f["seq"].clone()), (json!(2), json!(2), json!(3), json!(4)));
        let list = h.list_json();
        assert_eq!(list["limit"], RUNS_KEPT);
        let r1 = list["runs"].as_array().unwrap().iter().find(|r| r["run_id"] == "r-1").unwrap();
        assert_eq!(r1["stages"], json!([Value::Object(a), Value::Object(c)]));
        assert_eq!(r1["finished"], json!([Value::Object(f)]));
        assert_eq!(r1["outcome"], "rolled_back");
        assert_eq!(r1["queued_ms"], queued);
        assert_eq!(r1["last_seq"], 4);
        assert_eq!(ids(&h), ["r-2", "r-1"], "newest first");
    }

    #[test]
    fn keeps_the_newest_runs_but_not_at_the_expense_of_a_pending_approval() {
        let mut h = RunHistory::default();
        start(&mut h, "r-0");
        finish(&mut h, "r-0", "pending_approval", json!(null));
        for i in 1..=RUNS_KEPT + 2 {
            let id = format!("r-{i}");
            start(&mut h, &id);
            finish(&mut h, &id, "rolled_back", json!(null));
        }
        let kept = ids(&h);
        assert_eq!(kept.len(), RUNS_KEPT);
        assert!(kept.contains(&"r-0".to_string()), "pending run evicted: {kept:?}");
        assert_eq!(kept[0], format!("r-{}", RUNS_KEPT + 2));
        assert!(!kept.contains(&"r-1".to_string()) && !kept.contains(&"r-2".to_string()) && !kept.contains(&"r-3".to_string()));

        // The continuation lands in the kept record; once resolved the run is evictable again.
        let resumed = observe(&mut h, "run.stage", json!({"run_id": "r-0", "stage": "approval", "status": "ok", "ms": null, "detail": {}}));
        assert_eq!(resumed["seq"], 3);
        finish(&mut h, "r-0", "committed", json!(null));
        start(&mut h, "r-99");
        assert!(!ids(&h).contains(&"r-0".to_string()));
    }

    #[test]
    fn ordered_by_run_id_not_by_broadcast_order() {
        let mut h = RunHistory::default();
        for id in ["r-000002", "r-000001", "r-1000000", "r-000003"] {
            start(&mut h, id);
        }
        assert_eq!(ids(&h), ["r-1000000", "r-000003", "r-000002", "r-000001"]);
    }

    #[test]
    fn messages_for_runs_not_kept_get_no_seq_and_are_not_recorded() {
        let mut h = RunHistory::default();
        let stray = observe(&mut h, "run.stage", json!({"run_id": "r-gone", "stage": "reap", "status": "ok", "ms": null, "detail": {}}));
        assert!(stray.get("seq").is_none());
        assert!(ids(&h).is_empty());
        let other = observe(&mut h, "log", json!({"level": "info", "message": "x"}));
        assert!(other.get("seq").is_none());
    }
}
