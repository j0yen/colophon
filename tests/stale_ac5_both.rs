//! Stale AC5: file matching both WriterDead and OlderThanConsumer → reported once as Both.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{judge_provenance, parse, StaleReason};

const NOW: u64 = 1_800_000_000;
const OLD_TS: u64 = NOW - 7200;
const DEAD_PID: u32 = 4_000_000;
const MIN_AGE: u64 = 3600;

#[test]
fn both_conditions_reported_as_both() {
    let consumer_mtime = OLD_TS + 100; // consumer is newer
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{DEAD_PID};uid:1000"),
        Some(&OLD_TS.to_string()),
    );
    let verdict = judge_provenance(&prov, NOW, Some(consumer_mtime), MIN_AGE);
    assert!(
        matches!(verdict, Some((StaleReason::Both, _))),
        "expected Both, got {verdict:?}"
    );
}
