//! `aegis-mcp`: AegisFlow MCP server over stdio (one JSON-RPC message per line).
//!
//! Environment: `AEGIS_WORKSPACE` (optional workspace to open at start), `AEGIS_AGENT_ID`,
//! `AEGIS_MATTERMOST_WEBHOOK_URL`, `AEGIS_SLACK_WEBHOOK_URL`, `AEGIS_APPROVAL_CALLBACK_URL`.
//! Diagnostics go to stderr; stdout carries only protocol messages.

use std::io::{BufRead, Write};
use std::sync::Arc;

use adapter_switch_mcp::{ChatConfig, McpServer};
use aegis_ane_scorer::AneScorer;
use aegis_governance::GovernanceEngine;
use aegis_storage::Workspace;

fn main() {
    let scorer = AneScorer::open_default().unwrap_or_else(|e| {
        eprintln!("aegis-mcp: {e}");
        std::process::exit(1);
    });
    let gov = GovernanceEngine::load_default().unwrap_or_else(|e| {
        eprintln!("aegis-mcp: {e}");
        std::process::exit(1);
    });
    let mut server = McpServer::new(Arc::new(scorer), Arc::new(gov), ChatConfig::from_env());
    if let Some(path) = std::env::var_os("AEGIS_WORKSPACE") {
        match Workspace::open(&path) {
            Ok(ws) => server = server.with_workspace(ws),
            Err(e) => eprintln!("aegis-mcp: AEGIS_WORKSPACE not opened: {e}"),
        }
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            if writeln!(stdout, "{response}").and_then(|_| stdout.flush()).is_err() {
                break;
            }
        }
    }
}
