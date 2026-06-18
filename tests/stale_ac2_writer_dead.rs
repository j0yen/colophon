//! Stale AC2: dead pid + old ts → WriterDead; live pid + old ts → not flagged.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{judge_provenance, parse, StaleReason};

const NOW: u64 = 1_800_000_000;
const OLD_TS: u64 = NOW - 7200; // 2 h — older than 1 h min_age
const MIN_AGE: u64 = 3600;
/// A pid that is definitely not alive (above Linux pid_max of ~4M).
const DEAD_PID: u32 = 4_000_000;

#[test]
fn writer_dead_dead_pid_old_ts_flagged() {
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{DEAD_PID};uid:1000"),
        Some(&OLD_TS.to_string()),
    );
    let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
    assert!(
        matches!(verdict, Some((StaleReason::WriterDead, _))),
        "dead pid + old ts must be WriterDead, got {verdict:?}"
    );
}

#[test]
fn writer_dead_live_pid_not_flagged() {
    let my_pid = std::process::id();
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{my_pid};uid:1000"),
        Some(&OLD_TS.to_string()),
    );
    let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
    assert!(
        verdict.is_none(),
        "live pid must not be flagged WriterDead, got {verdict:?}"
    );
}
