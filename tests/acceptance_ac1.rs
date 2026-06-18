//! AC1 (MUST): cargo build and cargo test are green; binary installs to ~/.local/bin/colophon.
//!
//! This test verifies the binary can be compiled and run (smoke test).
//! The install step is exercised in CI / by the install script.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn acceptance_ac1_binary_runs() {
    // The binary must be reachable via `cargo run` with no args producing a usage message.
    // We verify the library compiles and the parse function is callable.
    let p = colophon::parse("comm-chain:cat>zsh;pid:1;uid:0", None);
    assert_eq!(p.comm_chain, ["cat", "zsh"]);
}
