# Design Document — AegisFlow Phase 1

## Overview

```
agent edit ──► aegis-storage::Workspace::branch()      clonefile(CLONE_NOFOLLOW|CLONE_NOOWNERCOPY)
                 │  agent writes inside the branch
                 ▼
             Branch::changes()  ──► features (64) ──► aegis-ane-scorer (CoreML FP16, Swift @_cdecl)
                                                       │ risk 0..100
                                                       ▼
                                   aegis-governance (cedar-policy 4.x + schema, redaction, ed25519)
                                                       │ Allow / Deny + determining policies
                         ┌─────────────────────────────┴──────────────────────────┐
                         ▼                                                        ▼
        Branch::commit()  renameatx_np(RENAME_SWAP)               Branch dropped → detach + reclaim
        previous tree kept as version snapshot                    live tree untouched
```

Adapters reuse the same pipeline: `adapter-nebius-nemotron` (build recovery), `adapter-paypal`
(Orders v2 middleware), `adapter-switch-mcp` (MCP tools + chat approval cards).

## Crates

| Crate | Responsibility | Key dependencies |
|---|---|---|
| aegis-storage | branches, atomic commit, versions, tree diff | libc (statfs), raw `clonefile` / `renameatx_np` FFI |
| aegis-governance | Cedar authorizer, redaction, receipts | cedar-policy 4, regex, sha2, ed25519-dalek 2 |
| aegis-ane-scorer | CoreML bridge, MLComputePlan report, features | swiftc-built static lib, CoreML.framework |
| adapter-nebius-nemotron | tiered recovery, OpenAI-compatible client, Tavily | ureq |
| adapter-paypal | Orders v2 body, OAuth2 client credentials, dashboard rows | ureq, base64 |
| adapter-switch-mcp | MCP stdio server, Mattermost/Slack payloads | ureq |
| aegis-bench | hardware benchmark → telemetry JSON | all of the above |

## Key decisions and limits

- **Branch cost is O(tree entries).** `clonefile` on a directory clones every entry's metadata
  (data blocks are shared). Sub-millisecond branching holds for small trees; the benchmark reports the
  per-entry cost so larger workspaces can be sized honestly.
- **Rollback is detach-then-reclaim.** The branch directory is renamed into a trash area (O(1)), then
  deleted. Both times are measured separately.
- **Optimistic concurrency.** A branch records the live directory's inode; after a commit the live
  path points at a different inode, so a stale branch's commit is rejected. Commits are serialized with
  an `flock` on the store lock file. Direct edits to the live tree outside branches are not detected.
- **Neural Engine placement is verified, not assumed.** `MLComputePlan` (macOS 14.4+) reports the
  preferred device per MIL operation; the telemetry stores that report.
- **The model is a demonstrator.** It is trained on synthetic samples labelled by the documented
  heuristic in `models/build_model.py`. It shows the pipeline and latency; it is not a validated
  production risk model.
- **`c2pa:ed25519:` is a label.** Receipts are detached Ed25519 signatures over SHA-256 of canonical
  JSON. They are not C2PA manifests.
- **External services are opt-in.** Nebius, Tavily, PayPal Sandbox and chat webhooks are called only
  when their credentials are in the environment; tests use local listeners.

## Policies

`policies/aegis_rules.cedar` (validated against `policies/aegis.cedarschema`):

1. `permit-branch-and-rollback` — isolated operations are always allowed for agents.
2. `permit-low-risk` — commits and orders with `ane_risk_score < 35` and no unformatted sensitive fields.
3. `permit-approved-high-impact-commit` — commits with risk ≥ 35 that carry team approval.
4. `forbid-order-over-50-usd-without-receipt` — `amount_cents > 5000` unless the receipt verified.
5. `forbid-high-impact-without-team-approval` — commits with risk ≥ 35 unless approved.

Cedar semantics: any matching `forbid` wins; with no matching `permit` the decision is deny.

## Telemetry flow

`aegis-bench` → `artifacts/aegis_phase1_telemetry.json` → `harness/record_test_results.py`
(+ `cargo test`) → `artifacts/aegis_telemetry.json` → `harness/sync_obsidian_brain.py` → `brain/`.
