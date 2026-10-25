---
tags:
  - core
status: pending
latency_ms: null
last_verified: null
git_commit: null
crate: aegis_governance
---

# Core-Cedar-Rules

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> Deterministic governance: `cedar-policy` 4.x evaluates policies/aegis_rules.cedar (schema-validated), `regex` redaction flags unformatted sensitive fields, `sha2` + `ed25519-dalek` produce `c2pa:ed25519:` receipts.

**Implemented in:** `crates/aegis-governance/src/lib.rs` · `policies/aegis_rules.cedar` · `policies/aegis.cedarschema`

**Upstream:** [[Agent-Systems-Architect]] (designs policy) · [[Agent-Core-Builder]] (builds)
**Downstream:** [[Adapter-PayPal-Commerce]] (budget rules) · [[Adapter-Switch-MCP]] (approval gate) · [[Adapter-Nebius-Nemotron]] (commit gate) · [[Bench-Cedar-Eval]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | no `cargo test --release` run recorded yet |
| is_authorized | — |
| decision matrix | —/— as expected |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
