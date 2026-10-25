# AegisFlow

Transactional copy-on-write workspaces, an on-device CoreML risk scorer and deterministic Cedar
governance for AI coding and procurement agents on Apple Silicon.

```
agent edit → clonefile branch → diff → 64 features → CoreML FP16 score → Cedar → RENAME_SWAP commit
                                                                                └→ or rollback (live tree untouched)
```

## Layout

| Path | What it is |
|---|---|
| `crates/aegis-storage` | `clonefile(CLONE_NOFOLLOW \| CLONE_NOOWNERCOPY)` branches, `renameatx_np(RENAME_SWAP)` commits, `Drop` rollback, version snapshots, tree diff |
| `crates/aegis-ane-scorer` | Swift `@_cdecl("aegis_ane_score_workflow")` bridge to CoreML (`.cpuAndNeuralEngine`), `MLComputePlan` placement report, feature extraction |
| `crates/aegis-governance` | `cedar-policy` 4.x with strict schema validation, `[REDACTED]` formatting, SHA-256 + Ed25519 `c2pa:ed25519:` receipts |
| `crates/adapter-nebius-nemotron` | Tier 0 local triage → Nemotron Super (Nebius) → Tavily + Nemotron Ultra; patches verified in CoW branches |
| `crates/adapter-paypal` | PayPal Sandbox Orders v2 middleware, Cedar budget rules, AG Grid / Bryntum rows |
| `crates/adapter-switch-mcp` | `aegis-mcp` MCP stdio server + Mattermost/Slack approval cards |
| `crates/aegis-bench` | Hardware benchmark → `artifacts/aegis_phase1_telemetry.json` |
| `models/` | Feature schema, trainer/exporter, `AegisWorkflowFP16{,Batch}.mlpackage/.mlmodelc`, parity vectors, model card |
| `policies/` | `aegis_rules.cedar`, `aegis.cedarschema` |
| `harness/` | `TASKS.json`, `STATE.json`, task runner, brain sync, test recorder, MCP smoke test |
| `brain/` | Obsidian vault: notes per node, `AegisFlow_Architecture.canvas`, colour-grouped graph |
| `.claude/` | Sub-agents and quality hooks (`verify_no_stubs.sh`, `post_task_brain_sync.sh`) |
| `.kiro/specs/aegisflow-phase1/` | Kiro requirements (EARS), design, generated task list |

## Run it

```bash
bash harness/build_model.sh                 # train + export + compile the CoreML models
cargo test --release --workspace
cargo run --release -p aegis-bench -- --out artifacts/aegis_phase1_telemetry.json
python3 harness/run_tasks.py                # execute TASKS.json and sync the brain
open -a Obsidian brain                      # vault is registered in Obsidian
```

Optional credentials (nothing remote is called without them): `NEBIUS_API_KEY`, `TAVILY_API_KEY`,
`PAYPAL_CLIENT_ID` / `PAYPAL_CLIENT_SECRET`, `AEGIS_SLACK_WEBHOOK_URL`, `AEGIS_MATTERMOST_WEBHOOK_URL`,
`AEGIS_APPROVAL_CALLBACK_URL`. The benchmark stays offline unless `AEGIS_BENCH_LIVE=1`.

## Known limits (measured, see the telemetry file for numbers)

- **Branch cost scales with tree entries.** `clonefile` copies metadata for every file and
  directory; only small trees branch in under 0.5 ms. The benchmark reports the per-entry cost.
- **Rollback = detach + reclaim.** Detaching (rename into trash) is constant time; deleting the
  detached tree scales with entries. Both are reported.
- **Neural Engine placement is CoreML's decision.** With `.cpuAndNeuralEngine`, `MLComputePlan`
  keeps the one-operation model on the CPU. The batch model (same weights, many operations per
  prediction) is placed on the Neural Engine. Both placements are recorded on every run.
- **The risk model is a demonstrator** trained on synthetic data labelled by the heuristic in
  `models/build_model.py`; it is not validated on real incidents.
- **`c2pa:ed25519:`** is a label for detached Ed25519 receipts, not a C2PA manifest.
- Direct edits to the live tree outside branches are not detected by commit conflict checks.
