---
tags:
  - core
status: verified
latency_ms: 0.004208
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
crate: aegis_governance
---

# Core-Cedar-Rules

**Status:** 🟢 verified · **p50:** 0.0042 ms · **commit:** `c3ad550`

> Deterministic governance: `cedar-policy` 4.x evaluates policies/aegis_rules.cedar (schema-validated), `regex` redaction flags unformatted sensitive fields, `sha2` + `ed25519-dalek` produce `c2pa:ed25519:` receipts.

**Implemented in:** `crates/aegis-governance/src/lib.rs` · `policies/aegis_rules.cedar` · `policies/aegis.cedarschema`

**Upstream:** [[Agent-Systems-Architect]] (designs policy) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-PayPal-Commerce]] (budget rules) · [[Adapter-Switch-MCP]] (approval gate) · [[Adapter-Nebius-Nemotron]] (commit gate) · [[Bench-Cedar-Eval]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| cargo test --release | 20 passed, 0 failed (2 suites) |
| is_authorized | p50 4.208 µs · p95 4.667 µs · n=20000 |
| decision matrix | 10/10 as expected |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
