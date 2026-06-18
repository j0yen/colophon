//! AC7 (MUST): `colophon parse` piped to `head` does not panic (SIGPIPE reset).
//!
//! When stdout is closed mid-write, a process that hasn't reset SIGPIPE will
//! receive SIGPIPE (signal 13) and exit with code 141 on Linux. Without
//! `sigpipe::reset()`, Rust's default panic handler fires instead.
//! This test asserts the exit is either 0 or 141, never a panic (exit 101).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Stdio};

fn colophon_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("colophon")
}

#[test]
fn acceptance_ac7_sigpipe_no_panic() {
    let bin = colophon_bin();

    // Pipe colophon output through `head -1` which closes the read end early.
    // We use a shell pipeline so head closes stdin before colophon finishes writing.
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{} parse --from-string 'comm-chain:cat>zsh>claude;cwd:/tmp;pid:1;uid:0' --format json | head -1",
            bin.display()
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("sh -c pipeline failed to spawn");

    // Accept exit 0 (wrote everything before head closed) or 141 (SIGPIPE).
    // Reject 101 (Rust panic), 134 (SIGABRT), or any other crash code.
    let code = out.status.code().unwrap_or(0);
    assert!(
        code == 0 || code == 141,
        "expected exit 0 or 141 (SIGPIPE), got {code}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Stderr must not contain "panicked at" — confirms sigpipe::reset() worked.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "binary panicked on SIGPIPE:\n{stderr}"
    );
}
