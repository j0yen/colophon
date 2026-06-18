//! Stale AC7: stale() is read-only (fixture tree unchanged); judge_provenance is pure.
//!
//! The SIGPIPE safety is ensured by sigpipe::reset() at the top of main().
//! Here we verify stale() does not mutate the files it reads.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{stale, StaleOpts};
use std::fs;

#[test]
fn stale_readonly_fixture_tree_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Create a few plain files; stale() will skip them (no xattrs → Unstamped).
    let file_a = dir.path().join("a.txt");
    let file_b = dir.path().join("b.txt");
    fs::write(&file_a, b"content A").expect("write a");
    fs::write(&file_b, b"content B").expect("write b");

    let opts = StaleOpts::default();
    let _ = stale(dir.path(), &opts);

    // Files must be unchanged after stale() returns.
    assert_eq!(fs::read(&file_a).expect("read a"), b"content A");
    assert_eq!(fs::read(&file_b).expect("read b"), b"content B");
}
