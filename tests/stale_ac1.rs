//! Stale AC1: cargo build/test green; colophon parse and colophon stale subcommands compile.
//!
//! Smoke-tests the stale API surface.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{stale, StaleOpts};
use std::path::Path;

#[test]
fn stale_ac1_api_compiles_and_empty_dir_returns_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let opts = StaleOpts::default();
    let verdicts = stale(dir.path(), &opts);
    assert!(verdicts.is_empty(), "empty dir should yield no verdicts");
}

fn tempdir_with_empty_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("unstamped.txt");
    std::fs::write(&path, b"hello").expect("write");
    (dir, path)
}

#[test]
fn stale_ac1_unstamped_file_not_flagged() {
    // An ordinary file with no xattrs must not appear in the stale list.
    let (_dir, _path) = tempdir_with_empty_file();
    // We cannot actually set xattrs in the test, but we can verify that
    // files that `read_file` returns as Unstamped are not flagged.
    // Directly exercise judge_provenance with Unstamped provenance.
    let prov = colophon::parse("", None);
    assert_eq!(prov.form, colophon::Form::Unstamped);
    let verdict = colophon::judge_provenance(&prov, 1_800_000_000, None, 3600);
    assert!(verdict.is_none(), "unstamped must not produce a verdict");
}

/// Ensure Path is at least importable (compile-time check).
#[test]
fn stale_ac1_stale_accepts_path_reference() {
    let opts = StaleOpts { min_age_secs: 60, ..StaleOpts::default() };
    let path = Path::new("/tmp");
    let _result: Vec<colophon::Staleness> = stale(path, &opts);
}
