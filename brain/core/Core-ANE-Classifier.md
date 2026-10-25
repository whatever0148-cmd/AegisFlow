---
tags:
  - core
status: pending
latency_ms: null
last_verified: null
git_commit: null
crate: aegis_ane_scorer
---

# Core-ANE-Classifier

**Status:** ⚪ pending · **p50:** — · **commit:** `uncommitted-dirty`

> CoreML FP16 MLP (64 -> 32 -> 16 -> 1, sigmoid x 100) loaded with `.cpuAndNeuralEngine` through the Swift `@_cdecl("aegis_ane_score_workflow")` bridge. Device placement is read back from `MLComputePlan`. Weights are trained on synthetic data labelled by the documented heuristic in models/build_model.py; this is not a validated production risk model.

**Implemented in:** `crates/aegis-ane-scorer/swift/AegisANEBridge.swift` · `crates/aegis-ane-scorer/src/lib.rs` · `models/build_model.py` · `models/feature_schema.json`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (tier-0 risk) · [[Adapter-PayPal-Commerce]] (order risk) · [[Bench-ANE-FP16]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T07:12:43Z · bench not run yet · tests not run yet · commit `uncommitted-dirty`_

| Metric | Measured |
|---|---|
| cargo test --release | no `cargo test --release` run recorded yet |
| inference · cpuAndNeuralEngine | — |
| parity max |Δ| vs NumPy FP32 | — |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
