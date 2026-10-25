# Requirements Document — AegisFlow Phase 1

## Introduction

AegisFlow lets autonomous coding and procurement agents act on a Mac without risking the live
workspace or the budget: every change happens in an APFS copy-on-write branch, is scored by an
on-device CoreML model, and is committed or rolled back according to deterministic Cedar policies.
Criteria ids (R<n>.<m>) are referenced by `harness/TASKS.json`.

## Requirements

### Requirement 1 — Copy-on-write workspace branches
**User Story:** As an AI coding agent, I want an isolated copy of the workspace almost instantly, so that I can edit files without touching the live tree.

#### Acceptance Criteria
1. (R1.1) WHEN an agent requests a branch THE SYSTEM SHALL create it with `clonefile(2)` and `CLONE_NOFOLLOW | CLONE_NOOWNERCOPY` on the same APFS volume as the live workspace.
2. (R1.2) IF the workspace is not on APFS THEN THE SYSTEM SHALL refuse to branch with an explicit error instead of falling back to a byte copy.

### Requirement 2 — Atomic commit and automatic rollback
**User Story:** As a team lead, I want agent changes to land atomically or not at all, so that the live workspace is never half-written.

#### Acceptance Criteria
1. (R2.1) WHEN a branch is committed THE SYSTEM SHALL exchange branch and live trees with `renameatx_np(RENAME_SWAP)` in a single syscall.
2. (R2.2) WHEN a branch is dropped without a commit THE SYSTEM SHALL detach and delete it, leaving the live tree byte-identical.
3. (R2.3) WHEN a commit succeeds THE SYSTEM SHALL keep the previous live tree as a restorable version snapshot, and IF another commit happened since the branch was created THEN THE SYSTEM SHALL reject the commit as a conflict.

### Requirement 3 — On-device risk scoring
**User Story:** As a governance engineer, I want each operation scored locally in well under a millisecond, so that policy decisions do not wait on the network.

#### Acceptance Criteria
1. (R3.1) THE SYSTEM SHALL ship an FP16 CoreML ML Program that maps a 64-feature operation vector to a 0–100 risk score.
2. (R3.2) WHEN scoring THE SYSTEM SHALL load the compiled model with `.cpuAndNeuralEngine` through a Swift `@_cdecl("aegis_ane_score_workflow")` bridge and SHALL match a NumPy FP32 reference within 1.0 score point.
3. (R3.3) THE SYSTEM SHALL report per-operation device placement from `MLComputePlan` rather than assuming Neural Engine execution.

### Requirement 4 — Deterministic governance
**User Story:** As a compliance owner, I want explicit, auditable rules for commits and payments, so that agents cannot exceed their authority.

#### Acceptance Criteria
1. (R4.1) WHILE `ane_risk_score < 35` and `has_unformatted_sensitive_fields == false` THE SYSTEM SHALL permit speculative workspace commits and standard orders.
2. (R4.2) WHEN a `paypal::create_order` exceeds 5000 cents THE SYSTEM SHALL forbid it unless `ed25519_receipt_verified == true`.
3. (R4.3) WHEN a workspace operation has `ane_risk_score >= 35` THE SYSTEM SHALL forbid it unless `team_approval_granted == true`.
4. (R4.4) THE SYSTEM SHALL sign outbound payloads with Ed25519 over their SHA-256 digest and SHALL reject receipts whose payload was altered.

### Requirement 5 — Build recovery router
**User Story:** As a developer, I want failed builds repaired automatically when it is safe, so that agents spend less time blocked.

#### Acceptance Criteria
1. (R5.1) WHEN a build fails THE SYSTEM SHALL route it with a local tier (ANE score + Cedar) before calling any remote model.
2. (R5.2) WHEN a remote model returns a patch THE SYSTEM SHALL apply and verify it in a fresh CoW branch and SHALL roll back on failure.
3. (R5.3) IF `NEBIUS_API_KEY` (tiers 1–2) or `TAVILY_API_KEY` (tier 2 search) is absent THEN THE SYSTEM SHALL report the tier as unavailable instead of calling the service.

### Requirement 6 — Agent payments middleware
**User Story:** As a finance owner, I want agent purchases redacted, signed and budget-checked, so that procurement is safe to automate.

#### Acceptance Criteria
1. (R6.1) WHEN preparing a PayPal Orders v2 request THE SYSTEM SHALL replace sensitive customer strings with `[REDACTED]`.
2. (R6.2) THE SYSTEM SHALL attach a verifiable `c2pa:ed25519:` signature and evaluate Cedar budget rules before any submission; submission happens only with Sandbox credentials present.
3. (R6.3) THE SYSTEM SHALL emit one telemetry row per order in AG Grid and Bryntum store formats.

### Requirement 7 — MCP and chat approvals
**User Story:** As a team member, I want to see an agent's diff and approve it from chat, so that high-impact changes get a human decision.

#### Acceptance Criteria
1. (R7.1) THE SYSTEM SHALL expose branch, score, policy, commit, rollback and approval tools over MCP (JSON-RPC 2.0 on stdio).
2. (R7.2) WHEN approval is requested THE SYSTEM SHALL render a workspace-diff card for Mattermost and Slack and SHALL post it only to webhooks configured in the environment.

### Requirement 8 — Evidence, not claims
**User Story:** As the founder, I want every status and number to come from a real run, so that I can trust the dashboard.

#### Acceptance Criteria
1. (R8.1) THE SYSTEM SHALL write benchmark results with sample counts and percentiles to `artifacts/aegis_phase1_telemetry.json`, including targets that were missed.
2. (R8.2) WHEN telemetry changes THE SYSTEM SHALL update the Obsidian vault's frontmatter (`status`, `latency_ms`, `last_verified`, `git_commit`) and canvas.
3. (R8.3) THE SYSTEM SHALL block edits containing stub markers or hand-typed metrics and SHALL run the release test suite after each agent task.
4. (R8.4) THE SYSTEM SHALL keep this spec's task list in sync with harness verification results.
