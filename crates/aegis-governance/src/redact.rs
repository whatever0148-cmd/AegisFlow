//! Detection and `[REDACTED]` formatting of sensitive strings.
//!
//! Detectors run in a fixed order (secrets, card numbers, SSNs, e-mail addresses, phone numbers)
//! and each match is masked before the next detector runs, so a value is counted once. Card
//! numbers must pass the Luhn check.

use std::sync::OnceLock;

use regex::{Captures, Regex};
use serde_json::Value;

pub const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScanReport {
    pub secrets: usize,
    pub card_numbers: usize,
    pub ssns: usize,
    pub emails: usize,
    pub phones: usize,
}

impl ScanReport {
    pub fn total(&self) -> usize {
        self.secrets + self.card_numbers + self.ssns + self.emails + self.phones
    }

    pub fn any(&self) -> bool {
        self.total() > 0
    }

    pub fn merge(&mut self, other: ScanReport) {
        self.secrets += other.secrets;
        self.card_numbers += other.card_numbers;
        self.ssns += other.ssns;
        self.emails += other.emails;
        self.phones += other.phones;
    }
}

pub struct SensitiveScanner {
    secret: Regex,
    card: Regex,
    ssn: Regex,
    email: Regex,
    phone: Regex,
}

/// Process-wide scanner (regexes are compiled once).
pub fn scanner() -> &'static SensitiveScanner {
    static SCANNER: OnceLock<SensitiveScanner> = OnceLock::new();
    SCANNER.get_or_init(SensitiveScanner::new)
}

impl Default for SensitiveScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl SensitiveScanner {
    pub fn new() -> Self {
        let re = |pattern: &str| Regex::new(pattern).expect("built-in detector pattern compiles");
        Self {
            secret: re(concat!(
                r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
                r"|\bAKIA[0-9A-Z]{16}\b",
                r"|\bgh[pousr]_[A-Za-z0-9]{36,}\b",
                r"|\bxox[abprs]-[A-Za-z0-9-]{10,}",
                r"|\bsk_(?:live|test)_[A-Za-z0-9]{16,}\b",
                r#"|(?i:\b(?:api[_-]?key|secret|token|passw(?:or)?d)\b)\s*[:=]\s*['"]?[A-Za-z0-9_\-./+]{8,}"#,
            )),
            card: re(r"\b(?:\d[ -]?){12,18}\d\b"),
            ssn: re(r"\b\d{3}-\d{2}-\d{4}\b"),
            email: re(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
            phone: re(r"(?:\+\d{1,3}[ .-]?)?(?:\(\d{3}\)|\b\d{3})[ .-]?\d{3}[ .-]?\d{4}\b"),
        }
    }

    /// Counts sensitive values without changing the text.
    pub fn scan(&self, text: &str) -> ScanReport {
        self.process(text).1
    }

    /// Returns the text with every sensitive value replaced by `[REDACTED]`.
    pub fn redact(&self, text: &str) -> (String, ScanReport) {
        self.process(text)
    }

    fn process(&self, text: &str) -> (String, ScanReport) {
        let mut report = ScanReport::default();
        let text = mask(&self.secret, text, |_| true, &mut report.secrets);
        let text = mask(&self.card, &text, luhn_valid, &mut report.card_numbers);
        let text = mask(&self.ssn, &text, |_| true, &mut report.ssns);
        let text = mask(&self.email, &text, |_| true, &mut report.emails);
        let text = mask(&self.phone, &text, |_| true, &mut report.phones);
        (text, report)
    }

    /// Scans every string (keys excluded) inside a JSON document.
    pub fn scan_json(&self, value: &Value) -> ScanReport {
        let mut report = ScanReport::default();
        visit(value, &mut |s| report.merge(self.scan(s)));
        report
    }

    /// Redacts every string inside a JSON document in place.
    pub fn redact_json(&self, value: &mut Value) -> ScanReport {
        let mut report = ScanReport::default();
        visit_mut(value, &mut |s| {
            let (clean, found) = self.redact(s);
            if found.any() {
                *s = clean;
                report.merge(found);
            }
        });
        report
    }
}

fn mask(re: &Regex, text: &str, accept: impl Fn(&str) -> bool, count: &mut usize) -> String {
    re.replace_all(text, |caps: &Captures| {
        let found = &caps[0];
        if accept(found) {
            *count += 1;
            REDACTED.to_string()
        } else {
            found.to_string()
        }
    })
    .into_owned()
}

fn luhn_valid(candidate: &str) -> bool {
    let digits: Vec<u32> = candidate.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                d
            }
        })
        .sum();
    sum % 10 == 0
}

fn visit(value: &Value, f: &mut impl FnMut(&str)) {
    match value {
        Value::String(s) => f(s),
        Value::Array(items) => items.iter().for_each(|v| visit(v, f)),
        Value::Object(map) => map.values().for_each(|v| visit(v, f)),
        _ => {}
    }
}

fn visit_mut(value: &mut Value, f: &mut impl FnMut(&mut String)) {
    match value {
        Value::String(s) => f(s),
        Value::Array(items) => items.iter_mut().for_each(|v| visit_mut(v, f)),
        Value::Object(map) => map.values_mut().for_each(|v| visit_mut(v, f)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_each_kind_once() {
        let text = "Ship to jane.doe@example.com, card 4111 1111 1111 1111, SSN 123-45-6789, call +1 (415) 555-0132";
        let (clean, report) = scanner().redact(text);
        assert_eq!(report, ScanReport { secrets: 0, card_numbers: 1, ssns: 1, emails: 1, phones: 1 });
        assert_eq!(clean.matches(REDACTED).count(), 4);
        assert!(!scanner().scan(&clean).any(), "redacted text must scan clean: {clean}");
    }

    #[test]
    fn non_luhn_digit_runs_are_kept() {
        let (clean, report) = scanner().redact("order ref 1234 5678 9012 3456");
        assert_eq!(report.card_numbers, 0);
        assert!(clean.contains("1234 5678 9012 3456"));
    }

    #[test]
    fn detects_secrets() {
        let key_id = format!("AKIA{}", "IOSFODNN7EXAMPLE");
        let report = scanner().scan(&format!("aws_key={key_id}\napi_key = \"abcd1234efgh5678\""));
        assert_eq!(report.secrets, 2);
    }

    #[test]
    fn json_redaction_reaches_nested_strings() {
        let mut doc = json!({"note": "email me at a.b@corp.io", "items": [{"name": "SSN 078-05-1120"}], "qty": 2});
        let report = scanner().redact_json(&mut doc);
        assert_eq!(report.total(), 2);
        assert_eq!(doc["items"][0]["name"], "SSN [REDACTED]");
        assert!(!scanner().scan_json(&doc).any());
    }
}
