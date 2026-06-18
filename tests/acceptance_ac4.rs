//! AC4 (MUST): AgentId / Unstamped form detection + fuzz-style bad-input table.
//!
//! - 32-hex-char non-zero → Form::AgentId with agent_session set
//! - all-zero 32-hex-char → Form::Unstamped
//! - empty string → Form::Unstamped
//! - No input causes a panic (proven by the table below)
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{parse, Form};

#[test]
fn acceptance_ac4_agent_id_nonzero() {
    let id = "0102030405060708090a0b0c0d0e0f10";
    let p = parse(id, None);
    assert_eq!(p.form, Form::AgentId, "non-zero 32-hex should be AgentId");
    assert_eq!(
        p.agent_session.as_deref(),
        Some(id),
        "agent_session should hold the hex id"
    );
    assert!(p.comm_chain.is_empty());
}

#[test]
fn acceptance_ac4_all_zero_is_unstamped() {
    let p = parse("00000000000000000000000000000000", None);
    assert_eq!(p.form, Form::Unstamped, "all-zero should be Unstamped");
    assert!(p.agent_session.is_none(), "agent_session should be None for Unstamped");
}

#[test]
fn acceptance_ac4_empty_is_unstamped() {
    let p = parse("", None);
    assert_eq!(p.form, Form::Unstamped, "empty should be Unstamped");
}

#[test]
fn acceptance_ac4_fuzz_table_no_panic() {
    // Table of malformed / adversarial / edge-case inputs. parse() must return
    // without panicking for all of them.
    // Pre-bind heap-allocated strings so they live for the duration of the slice.
    let long_input = "a".repeat(10_000);
    let bad_inputs: &[&str] = &[
        // Truncated forms
        "comm-chain:",
        "comm-chain:>",
        "comm-chain:>>",
        "comm-chain:a>>b",
        "comm-chain:foo;",
        "comm-chain:foo;cwd:",
        "comm-chain:foo;pid:",
        "comm-chain:foo;uid:",
        "comm-chain:foo;env:",
        "comm-chain:foo;env:=",
        "comm-chain:foo;env:KEY=",
        // Reordered fields
        "pid:1;uid:0;comm-chain:a>b;cwd:/x",
        "cwd:/tmp;pid:99;comm-chain:sh;uid:1000",
        // Missing fields
        "uid:0",
        "pid:123",
        "cwd:/",
        // Garbage
        ";;;;;",
        ">>>>>>",
        "=====",
        "null",
        "\x00\x01\x02",
        &long_input,
        // Almost-AgentId (31 chars, 33 chars)
        "0102030405060708090a0b0c0d0e0f1",
        "0102030405060708090a0b0c0d0e0f100",
        // Mixed hex/non-hex (31 valid + 1 'g')
        "0102030405060708090a0b0c0d0e0g10",
        // Whitespace
        "   ",
        "\t",
        "\n",
    ];

    for input in bad_inputs {
        let p = parse(input, None);
        // The only thing we assert: it returned, and `raw` equals `input`.
        assert_eq!(&p.raw, input, "raw should preserve input for: {input:?}");
    }
}

#[test]
fn acceptance_ac4_ts_parsing() {
    let p = parse("comm-chain:bash;pid:1;uid:0", Some("1781768028"));
    assert_eq!(p.ts, Some(1_781_768_028_u64));
}

#[test]
fn acceptance_ac4_malformed_ts_is_none() {
    let p = parse("comm-chain:bash;pid:1;uid:0", Some("not-a-number"));
    assert_eq!(p.ts, None);
}
