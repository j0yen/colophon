//! Stale AC3: dead pid but ts newer than min_age → NOT flagged WriterDead.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{judge_provenance, parse};

const NOW: u64 = 1_800_000_000;
const MIN_AGE: u64 = 3600; // 1 h
const RECENT_TS: u64 = NOW - 60; // only 1 min old
const DEAD_PID: u32 = 4_000_000;

#[test]
fn pid_reuse_guard_recent_file_not_flagged() {
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{DEAD_PID};uid:1000"),
        Some(&RECENT_TS.to_string()),
    );
    // Even though the pid is dead, the ts is too recent → guard fires
    let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
    assert!(
        verdict.is_none(),
        "recent ts must suppress WriterDead even for a dead pid, got {verdict:?}"
    );
}
