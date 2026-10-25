#!/usr/bin/env bash
# Smoke test for the aegis-mcp stdio server: initialize -> tools/list -> tools/call.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release -q -p adapter-switch-mcp --bin aegis-mcp
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"aegis_policy_evaluate","arguments":{"action":"paypal::create_order","ane_risk_score":20,"amount_cents":12000,"ed25519_receipt_verified":false}}}' \
  | target/release/aegis-mcp | python3 -c '
import json, sys
responses = [json.loads(line) for line in sys.stdin if line.strip()]
assert [r["id"] for r in responses] == [1, 2, 3], responses
assert responses[0]["result"]["protocolVersion"] == "2025-06-18"
tools = {t["name"] for t in responses[1]["result"]["tools"]}
assert {"aegis_branch_create", "aegis_branch_commit", "aegis_request_approval"} <= tools, tools
decision = responses[2]["result"]["structuredContent"]
assert decision["allowed"] is False, decision
assert decision["determining_policies"] == ["forbid-order-over-50-usd-without-receipt"], decision
print("mcp smoke: %d tools; $120 order without verified receipt -> Deny (%s)" % (len(tools), decision["determining_policies"][0]))
'
