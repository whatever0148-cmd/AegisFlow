//! In-memory approval store (API.md `Approval`).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::util::now_iso;

/// Resolved approvals returned by `GET /api/approvals`.
pub const RESOLVED_KEPT: usize = 50;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Approval {
    pub id: String,
    pub run_id: Option<String>,
    pub kind: &'static str,
    pub title: String,
    pub summary: String,
    pub risk: f64,
    pub files_changed: usize,
    pub excerpt: String,
    pub determining_policies: Vec<String>,
    pub created_at: String,
    pub status: &'static str,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<String>,
    pub result: Option<Value>,
    pub cards: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub struct ApprovalStore {
    seq: u64,
    pending: BTreeMap<String, Approval>,
    resolved: Vec<Approval>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    Pending,
    Resolved(&'static str),
    Unknown,
}

impl ApprovalStore {
    pub fn next_id(&mut self) -> String {
        self.seq += 1;
        format!("a-{:06}", self.seq)
    }

    pub fn insert(&mut self, approval: Approval) {
        self.pending.insert(approval.id.clone(), approval);
    }

    pub fn update_cards(&mut self, id: &str, cards: BTreeMap<String, String>) -> Option<Approval> {
        let a = self.pending.get_mut(id)?;
        a.cards = cards;
        Some(a.clone())
    }

    /// An id this store issued that is neither pending nor in the kept resolved list was resolved
    /// earlier and dropped from the list (only the newest [`RESOLVED_KEPT`] are kept), so it is
    /// `Resolved("resolved")`, not `Unknown`.
    pub fn lookup(&self, id: &str) -> Lookup {
        if self.pending.contains_key(id) {
            Lookup::Pending
        } else if let Some(a) = self.resolved.iter().find(|a| a.id == id) {
            Lookup::Resolved(a.status)
        } else if self.issued(id) {
            Lookup::Resolved("resolved")
        } else {
            Lookup::Unknown
        }
    }

    /// True for `a-<n>` with `1 <= n <= seq` written the way [`ApprovalStore::next_id`] writes it.
    fn issued(&self, id: &str) -> bool {
        let Some(digits) = id.strip_prefix("a-") else { return false };
        match digits.parse::<u64>() {
            Ok(n) => n >= 1 && n <= self.seq && format!("a-{n:06}") == id,
            Err(_) => false,
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn pending_ids(&self) -> Vec<String> {
        self.pending.keys().cloned().collect()
    }

    /// Moves a pending approval to the resolved list.
    pub fn resolve(&mut self, id: &str, approved: bool, by: &str, result: Value) -> Option<Approval> {
        let mut a = self.pending.remove(id)?;
        a.status = if approved { "approved" } else { "rejected" };
        a.resolved_by = Some(by.to_string());
        a.resolved_at = Some(now_iso());
        a.result = Some(result);
        self.resolved.insert(0, a.clone());
        self.resolved.truncate(RESOLVED_KEPT);
        Some(a)
    }

    pub fn list_json(&self) -> Value {
        let mut pending: Vec<&Approval> = self.pending.values().collect();
        pending.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        json!({"pending": pending, "resolved": self.resolved})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(id: String) -> Approval {
        Approval {
            id,
            run_id: None,
            kind: "workspace",
            title: String::new(),
            summary: String::new(),
            risk: 0.0,
            files_changed: 0,
            excerpt: String::new(),
            determining_policies: Vec::new(),
            created_at: now_iso(),
            status: "pending",
            resolved_by: None,
            resolved_at: None,
            result: None,
            cards: BTreeMap::new(),
        }
    }

    #[test]
    fn resolved_ids_stay_resolved_after_leaving_the_list() {
        let mut store = ApprovalStore::default();
        let ids: Vec<String> = (0..RESOLVED_KEPT + 3).map(|_| store.next_id()).collect();
        for id in &ids {
            store.insert(approval(id.clone()));
            store.resolve(id, false, "test", Value::Null).unwrap();
        }
        let pending = store.next_id();
        store.insert(approval(pending.clone()));
        assert_eq!(store.list_json()["resolved"].as_array().unwrap().len(), RESOLVED_KEPT);
        assert_eq!(store.lookup(&ids[0]), Lookup::Resolved("resolved"), "evicted from the list, still resolved");
        assert_eq!(store.lookup(ids.last().unwrap()), Lookup::Resolved("rejected"));
        assert_eq!(store.lookup(&pending), Lookup::Pending);
        for unknown in ["a-999999", "a-000000", "a-1", "a-0000001", "b-000001", "a-", "a-x"] {
            assert_eq!(store.lookup(unknown), Lookup::Unknown, "{unknown}");
        }
    }
}
