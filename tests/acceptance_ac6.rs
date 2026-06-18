//! AC6 (MUST): CLI `colophon parse --from-string` and real-file round-trip.
//!
//! --from-string path: always exercised.
//! real-file path: skipped with eprintln! when getfattr reports no user.prov.session.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::print_stderr,
    clippy::unnecessary_map_or
)]

use std::process::Command;

fn colophon_bin() -> std::path::PathBuf {
    // Use the binary built by cargo test infrastructure.
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop(); // strip test binary name
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("colophon")
}

#[test]
fn acceptance_ac6_from_string_json() {
    let bin = colophon_bin();
    let session = "comm-chain:cat>zsh>claude;cwd:/tmp;pid:1;uid:0";

    let out = Command::new(&bin)
        .args(["parse", "--from-string", session, "--format", "json"])
        .output()
        .expect("failed to run colophon");

    assert!(
        out.status.success(),
        "exit non-zero: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let text = String::from_utf8_lossy(&out.stdout);
    let val: serde_json::Value = serde_json::from_str(&text).expect("invalid JSON output");

    assert_eq!(val["form"], "CommChain");
    let chain = val["comm_chain"].as_array().expect("comm_chain array");
    assert_eq!(
        chain.iter().map(|v| v.as_str().unwrap_or("")).collect::<Vec<_>>(),
        ["cat", "zsh", "claude"]
    );
    assert_eq!(val["pid"], 1);
    assert_eq!(val["uid"], 0);
}

#[test]
fn acceptance_ac6_from_string_text_format() {
    let bin = colophon_bin();
    let session = "comm-chain:bash>claude;cwd:/home;pid:99;uid:1000";

    let out = Command::new(&bin)
        .args(["parse", "--from-string", session, "--format", "text"])
        .output()
        .expect("failed to run colophon");

    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("CommChain"), "text output should contain CommChain");
    assert!(text.contains("bash"), "text output should contain bash");
}

#[test]
fn acceptance_ac6_real_file_integration() {
    // Check if provfs is present by running getfattr on a known file.
    let gossip = std::path::Path::new("/home/jsy/wintermute/autobuilder/notes/gossip.md");
    if !gossip.exists() {
        eprintln!("AC6 real-file: gossip.md not present, skipping provfs integration test");
        return;
    }

    let getfattr_check = Command::new("getfattr")
        .args(["-d", "--absolute-names", gossip.to_str().unwrap_or("")])
        .output();

    let has_provfs = getfattr_check.map_or(false, |out| {
        String::from_utf8_lossy(&out.stdout).contains("user.prov.session")
    });

    if !has_provfs {
        eprintln!("AC6 real-file: no user.prov.session on gossip.md, skipping provfs integration test");
        return;
    }

    // provfs is live — run colophon against the real file.
    let bin = colophon_bin();
    let out = Command::new(&bin)
        .args(["parse", "--format", "json", gossip.to_str().unwrap_or("")])
        .output()
        .expect("failed to run colophon");

    assert!(
        out.status.success(),
        "colophon parse on real provfs file failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let text = String::from_utf8_lossy(&out.stdout);
    let val: serde_json::Value = serde_json::from_str(&text).expect("invalid JSON");
    // form must be CommChain or AgentId (not Unstamped) since xattr was present.
    let form = val["form"].as_str().unwrap_or("");
    assert!(
        form == "CommChain" || form == "AgentId",
        "expected CommChain or AgentId, got {form}"
    );
}
