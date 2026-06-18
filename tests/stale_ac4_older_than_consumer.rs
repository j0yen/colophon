//! Stale AC4: prov.ts < consumer mtime → OlderThanConsumer; prov.ts >= mtime → not flagged.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{judge_provenance, parse, StaleReason};

const NOW: u64 = 1_800_000_000;
const OLD_TS: u64 = NOW - 7200; // file was written 2 h ago
const CONSUMER_MTIME: u64 = OLD_TS + 3600; // consumer was built 1 h after the file

#[test]
fn older_than_consumer_flagged() {
    let my_pid = std::process::id(); // live pid → no WriterDead
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{my_pid};uid:1000"),
        Some(&OLD_TS.to_string()),
    );
    let verdict = judge_provenance(&prov, NOW, Some(CONSUMER_MTIME), 3600);
    assert!(
        matches!(verdict, Some((StaleReason::OlderThanConsumer, _))),
        "expected OlderThanConsumer, got {verdict:?}"
    );
}

#[test]
fn newer_than_consumer_not_flagged() {
    let my_pid = std::process::id();
    // File ts is *after* the consumer mtime.
    let newer_ts = CONSUMER_MTIME + 100;
    let prov = parse(
        &format!("comm-chain:bash>zsh;cwd:/x;pid:{my_pid};uid:1000"),
        Some(&newer_ts.to_string()),
    );
    let verdict = judge_provenance(&prov, NOW, Some(CONSUMER_MTIME), 3600);
    assert!(
        verdict.is_none(),
        "file newer than consumer should not be flagged, got {verdict:?}"
    );
}
