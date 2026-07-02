//! Process-level resilience tests for the stdio JSON-RPC transport.
//!
//! These exercise the actual compiled binary as a subprocess, since the
//! bugs being guarded against (blank-line-treated-as-EOF, a single bad
//! request killing the whole server) only manifest at the process/pipe
//! boundary — a unit test against `Stdio` in isolation wouldn't prove the
//! main loop behaves correctly end-to-end.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

fn binary_path() -> std::path::PathBuf {
    // Cargo places the test binary's sibling build artifacts here.
    let mut path = std::env::current_exe().expect("current_exe");
    path.pop(); // test binary name
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("mcp-for-page-builders");
    assert!(
        path.exists(),
        "expected binary at {} — run `cargo build` first",
        path.display()
    );
    path
}

/// Blank lines between JSON-RPC messages must not be treated as EOF.
/// Regression test for a bug where `read_request` returned `Ok(None)` on
/// an empty line, identical to true EOF, silently killing the server on
/// any stray newline — which the MCP client observed as "connection closed".
#[test]
fn survives_blank_lines_and_malformed_json() {
    let mut child = Command::new(binary_path())
        .env("MCP_LOG_DIR", isolated_log_dir("survives_blank_lines"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Write init request, a blank line, a malformed line, another blank
    // line, then a second valid request — all interleaved as a real
    // client's writer might emit them.
    let payload = format!(
        "{}\n\n{}\n\n{}\n",
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "not valid json at all",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    );
    stdin.write_all(payload.as_bytes()).expect("write to child stdin");
    stdin.flush().expect("flush child stdin");

    // Read exactly two non-blank response lines. `read_line` is a blocking
    // call with no built-in timeout, so a hung/deadlocked child could hang
    // the test indefinitely if we called it directly on this thread. Do
    // the reading on a background thread and join it with a timeout via a
    // channel, so a hang fails fast instead of blocking the test suite/CI.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut responses: Vec<String> = Vec::new();
        while responses.len() < 2 {
            let mut line = String::new();
            let Ok(n) = reader.read_line(&mut line) else { break };
            if n == 0 {
                break; // EOF
            }
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                responses.push(trimmed.to_string());
            }
        }
        let _ = tx.send(responses);
    });

    let responses = rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    assert_eq!(
        responses.len(),
        2,
        "expected 2 valid responses despite blank/malformed lines, got: {responses:?}"
    );

    let r0: Value = serde_json::from_str(&responses[0]).expect("response 0 is valid JSON");
    let r1: Value = serde_json::from_str(&responses[1]).expect("response 1 is valid JSON");
    assert_eq!(r0["id"], 1);
    assert_eq!(r1["id"], 2);
}

/// Unique per-test log directory under the target dir, so concurrently
/// running tests never share (and race on) the same log files.
fn isolated_log_dir(test_name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("mcp-for-page-builders-test-logs")
        .join(format!("{test_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The log directory should contain a durable record of the malformed
/// line, independent of stdout/stderr, so failures are diagnosable even
/// after the client's pipe is gone.
#[test]
fn malformed_line_is_logged_to_file() {
    let log_dir = isolated_log_dir("malformed_line_logged");

    let mut child = Command::new(binary_path())
        .env("MCP_LOG_DIR", &log_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");

    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "definitely {{ not json").expect("write to child stdin");
    stdin.flush().expect("flush child stdin");

    // Poll for the log file to appear and contain the expected line,
    // instead of a fixed sleep — a fixed delay is flaky under load
    // (background file-writer thread scheduling, disk contention from
    // other concurrently running builds/tests).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut found = false;
    while std::time::Instant::now() < deadline {
        if let Ok(entries) = std::fs::read_dir(&log_dir) {
            for entry in entries.filter_map(std::result::Result::ok) {
                if std::fs::read_to_string(entry.path())
                    .unwrap_or_default()
                    .contains("Failed to parse incoming JSON-RPC line")
                {
                    found = true;
                    break;
                }
            }
        }
        if found {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    assert!(found, "expected the malformed-line error to be recorded in the log file within 10s");
}
