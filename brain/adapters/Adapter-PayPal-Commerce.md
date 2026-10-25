---
tags:
  - adapter
status: pending
latency_ms: null
last_verified: null
git_commit: null
crate: adapter_paypal
---

# Adapter-PayPal-Commerce

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> PayPal Sandbox `/v2/checkout/orders` middleware: `[REDACTED]` formatting of sensitive strings, ed25519 receipt (`c2pa:ed25519:...`) in `custom_id`, Cedar budget rules, AG Grid + Bryntum telemetry rows.

**Implemented in:** `crates/adapter-paypal/src/lib.rs`

**Upstream:** [[Agent-Telemetry-UI]] (dashboard rows) · [[Core-ANE-Classifier]] (order risk) · [[Core-Cedar-Rules]] (budget rules)
**Downstream:** [[Bench-EndToEnd-Rollback]] (receipt signing)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | no `cargo test --release` run recorded yet |
| prepare order (redact+sign+verify+Cedar) | — |
| ed25519 sign | — |
| Sandbox submission | — |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
