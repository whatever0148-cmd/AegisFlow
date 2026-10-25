//! `aegis-daemon` — Aegis Console server and settings CLI.
//!
//! ```text
//! aegis-daemon [serve]            serve the console on http://127.0.0.1:4173 (AEGIS_CONSOLE_PORT);
//!                                 [::1]:4173 is bound too, so no other process can answer
//!                                 http://localhost:4173 over IPv6 while the daemon runs
//! aegis-daemon config status      print every known setting as set (source) / unset
//! aegis-daemon config keys        print every known setting: NAME secret|public
//! aegis-daemon config providers   print the allowlisted Nemotron providers (NEBIUS_BASE_URL values),
//!                                 tab-separated: selected(*|-) id base_url label signup_url
//!                                 tier1_model tier2_model (`-` where the provider has no default)
//! aegis-daemon config set NAME    read the value from stdin and write it to .env (0600)
//! aegis-daemon config unset NAME  remove NAME from .env
//! ```
//!
//! Values are never accepted as command-line arguments and never printed.
//!
//! SIGINT or SIGTERM stops the server gracefully (pending approval branches rolled back, pool
//! dropped, reaper drained for up to a few seconds); a second signal exits at once.

use std::io::Read;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use aegis_config::{Settings, KEYS};
use aegis_daemon::{router, spawn_background, workspace, AppState, DaemonOptions};
use tokio::signal::unix::{signal, SignalKind};

/// Upper bound for the workspace shutdown (pending rollbacks, pool drop, reaper drain).
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);

fn usage() -> ExitCode {
    eprintln!("usage: aegis-daemon [serve] | config status | config keys | config providers | config set NAME (value on stdin) | config unset NAME");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["serve"] => serve(),
        ["config", rest @ ..] => config(rest),
        ["-h"] | ["--help"] | ["help"] => {
            usage();
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn config(args: &[&str]) -> ExitCode {
    let path = aegis_config::default_dotenv_path();
    match args {
        ["status"] => match Settings::load(&path) {
            Ok(settings) => {
                println!("settings file: {}", path.display());
                for status in settings.describe() {
                    match status.source {
                        Some(source) => println!("{}: set ({})", status.spec.name, source.name()),
                        None => println!("{}: unset", status.spec.name),
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("reading {}: {e}", path.display());
                ExitCode::FAILURE
            }
        },
        ["keys"] => {
            for spec in KEYS {
                println!("{} {}", spec.name, if spec.secret { "secret" } else { "public" });
            }
            ExitCode::SUCCESS
        }
        ["providers"] => {
            // Which provider is selected is not secret; the URL list is the allowlist itself.
            let selected = Settings::load(&path)
                .ok()
                .and_then(|s| s.get("NEBIUS_BASE_URL").map(str::to_string))
                .unwrap_or_else(|| aegis_config::DEFAULT_NEMOTRON_BASE_URL.to_string());
            let selected = aegis_config::nemotron_provider(&selected).map(|p| p.id);
            for p in aegis_config::NEMOTRON_PROVIDERS {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    if selected == Some(p.id) { "*" } else { "-" },
                    p.id,
                    p.base_url,
                    p.label,
                    p.signup_url,
                    p.tier1_model.unwrap_or("-"),
                    p.tier2_model.unwrap_or("-")
                );
            }
            ExitCode::SUCCESS
        }
        ["set", name] => {
            if aegis_config::spec(name).is_none() {
                eprintln!("unknown setting {name}");
                return ExitCode::from(2);
            }
            let mut value = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut value) {
                eprintln!("reading the value from stdin: {e}");
                return ExitCode::FAILURE;
            }
            let value = value.strip_suffix('\n').unwrap_or(&value);
            let value = value.strip_suffix('\r').unwrap_or(value);
            if value.trim().is_empty() {
                eprintln!("{name}: empty value on stdin; use `config unset {name}` to remove it");
                return ExitCode::from(2);
            }
            match aegis_config::write_dotenv(&path, &[(name, Some(value))]) {
                Ok(()) => {
                    println!("{name}: set (written to {}, mode 0600)", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{name}: not written: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ["unset", name] => match aegis_config::write_dotenv(&path, &[(name, None)]) {
            Ok(()) => {
                println!("{name}: unset in {}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{name}: not changed: {e}");
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}

/// Resolves on the first SIGINT or SIGTERM and returns its name; afterwards any further SIGINT or
/// SIGTERM exits the process immediately (a stuck shutdown can always be interrupted).
async fn first_signal(mut int: tokio::signal::unix::Signal, mut term: tokio::signal::unix::Signal) -> &'static str {
    let name = tokio::select! {
        _ = int.recv() => "SIGINT",
        _ = term.recv() => "SIGTERM",
    };
    tokio::spawn(async move {
        let second = tokio::select! {
            _ = int.recv() => "SIGINT",
            _ = term.recv() => "SIGTERM",
        };
        eprintln!("aegis-daemon: second signal ({second}); exiting now without waiting for the shutdown to finish");
        std::process::exit(130);
    });
    name
}

fn serve() -> ExitCode {
    let opts = DaemonOptions::from_env();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("aegis-daemon: tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(async move {
        // Installed before anything slow, so SIGTERM during startup is not the default (instant) exit.
        let (int, term) = match (signal(SignalKind::interrupt()), signal(SignalKind::terminate())) {
            (Ok(int), Ok(term)) => (int, term),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("aegis-daemon: installing signal handlers: {e}");
                return ExitCode::FAILURE;
            }
        };
        let port = opts.port;
        eprintln!("aegis-daemon: opening CoreML model, Cedar policies and the demo workspace…");
        let state = match tokio::task::spawn_blocking(move || AppState::open(opts)).await {
            Ok(Ok(state)) => state,
            Ok(Err(e)) => {
                eprintln!("aegis-daemon: {e}");
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("aegis-daemon: startup worker failed: {e}");
                return ExitCode::FAILURE;
            }
        };
        if state.workspace.startup_discarded > 0 {
            eprintln!(
                "aegis-daemon: discarded {} branch(es) left in the demo store by an earlier daemon (pending approvals do not survive a restart)",
                state.workspace.startup_discarded
            );
        }
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("aegis-daemon: binding {addr}: {e}");
                return ExitCode::FAILURE;
            }
        };
        // IPv6 loopback too: browsers try [::1] first for "localhost", and an unbound [::1]:<port>
        // could be taken by any local process.
        let addr6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
        let listener6 = match tokio::net::TcpListener::bind(addr6).await {
            Ok(l) => Some(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!(
                    "aegis-daemon: WARNING: another process is listening on {addr6}, so http://localhost:{port} may reach it instead of \
                     this daemon. Use http://127.0.0.1:{port}."
                );
                None
            }
            Err(e) => {
                eprintln!("aegis-daemon: not serving {addr6} ({e}); use http://127.0.0.1:{port}");
                None
            }
        };
        spawn_background(&state);
        let bound = if listener6.is_some() { format!("{addr} and {addr6}") } else { addr.to_string() };
        eprintln!("aegis-daemon: serving the Aegis Console on http://127.0.0.1:{port} (bound to {bound}); Ctrl-C stops it");
        let st = state.clone();
        tokio::spawn(async move {
            let name = first_signal(int, term).await;
            eprintln!("aegis-daemon: {name}: shutting down… (send it again to exit at once)");
            let _ = st.shutdown.send(true);
        });
        let stopped = |st: Arc<AppState>| async move {
            let mut rx = st.shutdown.subscribe();
            let _ = rx.wait_for(|stop| *stop).await;
        };
        let serve4 = axum::serve(listener, router(state.clone())).with_graceful_shutdown(stopped(state.clone()));
        let served = match listener6 {
            Some(listener6) => {
                let serve6 = axum::serve(listener6, router(state.clone())).with_graceful_shutdown(stopped(state.clone()));
                let (a, b) = tokio::join!(serve4, serve6);
                a.and(b)
            }
            None => serve4.await,
        };
        let _ = state.shutdown.send(true);
        state.paypal.set_stream(false, state.paypal.stream_state().interval_ms);
        let held = workspace::pending_branches(&state);
        let st = state.clone();
        let cleanup = tokio::task::spawn_blocking(move || workspace::shutdown(&st, SHUTDOWN_GRACE));
        match tokio::time::timeout(SHUTDOWN_DEADLINE, cleanup).await {
            Ok(_) => eprintln!("aegis-daemon: pool dropped, {held} pending approval branch(es) rolled back, reaper drained; bye"),
            Err(_) => eprintln!(
                "aegis-daemon: workspace shutdown did not finish within {} s; exiting (leftover branches are discarded at the next start)",
                SHUTDOWN_DEADLINE.as_secs()
            ),
        }
        drop(Arc::clone(&state));
        match served {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("aegis-daemon: server error: {e}");
                ExitCode::FAILURE
            }
        }
    });
    // Do not wait for blocking workers still inside a long call (a remote recovery, a rustc run):
    // the storage layer leaves their branches for the next start to discard.
    runtime.shutdown_timeout(Duration::from_secs(1));
    code
}
