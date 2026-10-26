---
tags:
  - aegisflow
last_synced: 2026-10-26T06:51:41Z
git_commit: 7cb1a10-dirty
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 237.7 | 2026-10-26T06:51:41Z |
| [[Agent-Core-Builder]] | 🟢 verified | 118405.7 | 2026-10-26T06:46:27Z |
| [[Agent-QA-Reviewer]] | 🟢 verified | 410172.0 | 2026-10-26T06:51:41Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 322.5 | 2026-10-26T06:51:41Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1060 | 2026-10-26T06:51:38Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1281 | 2026-10-26T06:51:38Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1938 | 2026-10-26T06:51:38Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0042 | 2026-10-26T06:51:38Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.1998 | 2026-10-26T06:51:38Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2735 | 2026-10-26T06:51:38Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.1330 | 2026-10-26T06:51:38Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-26T06:51:41Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1060 | 2026-10-26T06:51:38Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1938 | 2026-10-26T06:51:38Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0042 | 2026-10-26T06:51:38Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 3.253 | 2026-10-26T06:51:38Z |

See also: [[Telemetry-Log]]
