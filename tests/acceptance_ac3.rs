//! AC3 (MUST): parse() decodes an env:-bearing sample.
//!
//! Input: `comm-chain:bash>claude;env:CLAUDE_TOOL=/build;cwd:/x;pid:1;uid:1000`
//! Expected: env["CLAUDE_TOOL"] == "/build"
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown, clippy::doc_link_with_quotes)]

use colophon::parse;

#[test]
fn acceptance_ac3_env_field() {
    let p = parse(
        "comm-chain:bash>claude;env:CLAUDE_TOOL=/build;cwd:/x;pid:1;uid:1000",
        None,
    );
    assert_eq!(
        p.env.get("CLAUDE_TOOL").map(String::as_str),
        Some("/build"),
        "env[CLAUDE_TOOL] should be /build"
    );
    assert_eq!(p.comm_chain, ["bash", "claude"]);
    assert_eq!(p.cwd.as_deref(), Some("/x"));
    assert_eq!(p.pid, Some(1));
    assert_eq!(p.uid, Some(1000));
}

#[test]
fn acceptance_ac3_env_value_with_equals() {
    // env value itself contains '=' — only split on first '='
    let p = parse(
        "comm-chain:bash>claude;env:PATH=/usr/bin=/sbin;pid:1;uid:0",
        None,
    );
    assert_eq!(
        p.env.get("PATH").map(String::as_str),
        Some("/usr/bin=/sbin"),
        "env value should preserve inner '='"
    );
}
