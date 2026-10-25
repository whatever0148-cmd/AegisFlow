---
tags:
  - core
status: verified
latency_ms: 0.00425
last_verified: 2026-10-25T18:15:32Z
git_commit: 3bfc312-dirty
crate: aegis_governance
---

# Core-Cedar-Rules

**Status:** 🟢 verified · **p50:** 0.0043 ms · **commit:** `3bfc312-dirty`

> Deterministic governance: `cedar-policy` 4.x evaluates policies/aegis_rules.cedar (schema-validated), `regex` redaction flags unformatted sensitive fields, `sha2` + `ed25519-dalek` produce `c2pa:ed25519:` receipts.

**Implemented in:** `crates/aegis-governance/src/lib.rs` · `policies/aegis_rules.cedar` · `policies/aegis.cedarschema`

**Upstream:** [[Agent-Systems-Architect]] (designs policy) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-PayPal-Commerce]] (budget rules) · [[Adapter-Switch-MCP]] (approval gate) · [[Adapter-Nebius-Nemotron]] (commit gate) · [[Bench-Cedar-Eval]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T18:15:35Z · bench 2026-10-25T18:15:32Z · tests 2026-10-25T08:10:13Z · commit `3bfc312-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | 20 passed, 0 failed (2 suites) |
| is_authorized | p50 4.250 µs · p95 4.709 µs · n=20000 |
| decision matrix | 10/10 as expected |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
