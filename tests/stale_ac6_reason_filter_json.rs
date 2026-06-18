//! Stale AC6: StaleReason serializes to JSON; judge_provenance returns typed reasons.
//!
//! The CLI --reason and --format json flags are exercised in integration tests;
//! here we verify the data model serializes correctly.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{judge_provenance, parse, StaleReason};

const NOW: u64 = 1_800_000_000;
const OLD_TS: u64 = NOW - 7200;
const DEAD_PID: u32 = 4_000_000;

#[test]
fn stale_reason_serializes_writer_dead() {
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{DEAD_PID};uid:1000"),
        Some(&OLD_TS.to_string()),
    );
    let (reason, _) = judge_provenance(&prov, NOW, None, 3600)
        .expect("should be WriterDead");
    assert_eq!(reason, StaleReason::WriterDead);
    let json = serde_json::to_string(&reason).expect("serialize");
    assert_eq!(json, "\"WriterDead\"");
}

#[test]
fn stale_reason_serializes_both() {
    let consumer_mtime = OLD_TS + 100;
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{DEAD_PID};uid:1000"),
        Some(&OLD_TS.to_string()),
    );
    let (reason, _) = judge_provenance(&prov, NOW, Some(consumer_mtime), 3600)
        .expect("should be Both");
    assert_eq!(reason, StaleReason::Both);
    let json = serde_json::to_string(&reason).expect("serialize");
    assert_eq!(json, "\"Both\"");
}
