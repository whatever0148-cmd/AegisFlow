---
tags:
  - aegisflow
last_synced: 2026-10-25T07:12:43Z
git_commit: uncommitted-dirty
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | ⚪ pending | — | — |
| [[Agent-Core-Builder]] | ⚪ pending | — | — |
| [[Agent-QA-Reviewer]] | ⚪ pending | — | — |
| [[Agent-Telemetry-UI]] | ⚪ pending | — | — |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | ⚪ pending | — | — |
| [[Core-Memory-Snapshot]] | ⚪ pending | — | — |
| [[Core-ANE-Classifier]] | ⚪ pending | — | — |
| [[Core-Cedar-Rules]] | ⚪ pending | — | — |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | ⚪ pending | — | — |
| [[Adapter-PayPal-Commerce]] | ⚪ pending | — | — |
| [[Adapter-Switch-MCP]] | ⚪ pending | — | — |
| [[Adapter-Kiro-Specs]] | ⚪ pending | — | — |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | ⚪ pending | — | — |
| [[Bench-ANE-FP16]] | ⚪ pending | — | — |
| [[Bench-Cedar-Eval]] | ⚪ pending | — | — |
| [[Bench-EndToEnd-Rollback]] | ⚪ pending | — | — |

See also: [[Telemetry-Log]]
