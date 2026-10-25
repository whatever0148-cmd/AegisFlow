//! The real binary: SIGTERM takes the graceful shutdown path (it used to kill the process at once,
//! leaking pending-approval branches), and the startup message points at 127.0.0.1.
//!
//! The daemon gets a scratch demo directory and `.env`, the keep-warm off, and none of the known
//! credential variables, so it never calls an external service.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

#[test]
fn sigterm_runs_the_graceful_shutdown() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-fixtures")
        .join(format!("daemon-sigterm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".env"), "").unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aegis-daemon"));
    cmd.arg("serve")
        .env("AEGIS_CONSOLE_PORT", port.to_string())
        .env("AEGIS_CONSOLE_DEMO_DIR", root.join("demo"))
        .env("AEGIS_DOTENV", root.join(".env"))
        .env("AEGIS_ANE_KEEPWARM_MS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for spec in aegis_config::KEYS {
        cmd.env_remove(spec.name);
    }
    let mut child = cmd.spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    let mut log = Vec::new();
    let serving = loop {
        match lines.recv_timeout(STARTUP_TIMEOUT) {
            Ok(line) => {
                log.push(line.clone());
                if line.contains("serving the Aegis Console") {
                    break line;
                }
            }
            Err(e) => {
                let _ = child.kill();
                panic!("daemon did not start ({e}):\n{}", log.join("\n"));
            }
        }
    };
    assert!(serving.contains(&format!("http://127.0.0.1:{port}")), "{serving}");

    let status = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
    assert!(status.success());
    // The reader ends when the daemon closes stderr at exit.
    while let Ok(line) = lines.recv_timeout(STARTUP_TIMEOUT) {
        log.push(line);
    }
    let exit = child.wait().unwrap();
    let text = log.join("\n");
    assert!(exit.success(), "exit {exit:?}:\n{text}");
    assert!(text.contains("SIGTERM: shutting down"), "{text}");
    assert!(text.contains("pending approval branch(es) rolled back"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}
