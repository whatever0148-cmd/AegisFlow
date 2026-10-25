---
tags:
  - aegisflow
last_synced: 2026-10-25T18:15:35Z
git_commit: 3bfc312-dirty
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 429.5 | 2026-10-25T18:15:35Z |
| [[Agent-Core-Builder]] | 🟢 verified | 109682.1 | 2026-10-25T18:11:27Z |
| [[Agent-QA-Reviewer]] | 🟢 verified | 316316.8 | 2026-10-25T18:15:34Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 336.0 | 2026-10-25T18:15:35Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1069 | 2026-10-25T18:15:32Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1242 | 2026-10-25T18:15:32Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1816 | 2026-10-25T18:15:32Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0043 | 2026-10-25T18:15:32Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.1807 | 2026-10-25T18:15:32Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2646 | 2026-10-25T18:15:32Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.0622 | 2026-10-25T18:15:32Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-25T18:15:35Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1069 | 2026-10-25T18:15:32Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1816 | 2026-10-25T18:15:32Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0043 | 2026-10-25T18:15:32Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 3.165 | 2026-10-25T18:15:32Z |

See also: [[Telemetry-Log]]
