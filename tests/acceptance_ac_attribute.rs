//! Acceptance tests for the `colophon attribute` PRD (PRD-colophon-attribute).
//!
//! ACs covered:
//!   AC2 — three distinct actor buckets with correct file_count / total_bytes
//!   AC3 — target/ subdir files counted in `skipped`, NOT unstamped
//!   AC4 — non-skipped file with no xattr lands in `unstamped`
//!   AC5 — --by skill groups dream/build correctly; --top 2 caps buckets
//!   AC6 — fixture tree is byte-for-byte unchanged after attribute() runs
//!   AC7 — live ~/wintermute/autobuilder integration (skipped when provfs absent)
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::indexing_slicing,
    clippy::doc_markdown,
    clippy::option_if_let_else
)]

use colophon::attribute::{attribute, AttributeOpts, GroupBy};
use std::fs;
use std::path::Path;
use std::process::Command;

// ── Helper: set a user.prov.session xattr on a file (requires setfattr) ──────

/// Set `user.prov.session` on `path`. Returns false if setfattr is unavailable
/// (skip the calling test gracefully).
fn set_prov(path: &Path, session: &str) -> bool {
    Command::new("setfattr")
        .arg("-n")
        .arg("user.prov.session")
        .arg("-v")
        .arg(session)
        .arg(path)
        .status()
        .is_ok_and(|s| s.success())
}

/// Check whether setfattr is available on this machine.
fn setfattr_available() -> bool {
    Command::new("setfattr").arg("--version").output().is_ok()
}

// ── AC2: Three actor buckets ──────────────────────────────────────────────────

#[test]
fn ac2_three_actor_buckets_correct_counts() {
    if !setfattr_available() {
        eprintln!("AC2 SKIP: setfattr not available");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // Actor 1: dream — 2 files, 10 + 20 = 30 bytes
    let f1 = root.join("dream_a.txt");
    let f2 = root.join("dream_b.txt");
    fs::write(&f1, b"aaaaaaaaaa").expect("write f1"); // 10 bytes
    fs::write(&f2, b"bbbbbbbbbbbbbbbbbbbb").expect("write f2"); // 20 bytes
    let dream_prov = "comm-chain:bash>claude-dream-helio;cwd:/x;pid:1;uid:1000";

    // Actor 2: build — 3 files, 5 + 5 + 5 = 15 bytes
    let f3 = root.join("build_a.txt");
    let f4 = root.join("build_b.txt");
    let f5 = root.join("build_c.txt");
    fs::write(&f3, b"xxxxx").expect("write f3");
    fs::write(&f4, b"yyyyy").expect("write f4");
    fs::write(&f5, b"zzzzz").expect("write f5");
    let build_prov = "comm-chain:bash>claude-build-main;cwd:/x;pid:2;uid:1000";

    // Actor 3: shell (no claude) — 1 file, 100 bytes
    let f6 = root.join("shell.txt");
    fs::write(&f6, b"x".repeat(100).as_slice()).expect("write f6");
    let shell_prov = "comm-chain:bash>zsh>xterm;cwd:/home/jsy;pid:3;uid:1000";

    // Set xattrs — if setfattr fails (no xattr support on this fs), skip.
    if !set_prov(&f1, dream_prov)
        || !set_prov(&f2, dream_prov)
        || !set_prov(&f3, build_prov)
        || !set_prov(&f4, build_prov)
        || !set_prov(&f5, build_prov)
        || !set_prov(&f6, shell_prov)
    {
        eprintln!("AC2 SKIP: setfattr failed (xattr not supported on this fs)");
        return;
    }

    let opts = AttributeOpts {
        max_depth: 4,
        top_n: 10,
        min_bytes: 0,
        group_by: GroupBy::Skill,
    };
    let report = attribute(root, &opts);

    // Should have exactly 3 actor buckets.
    assert_eq!(report.actors.len(), 3, "expected 3 actor buckets, got: {:?}", report.actors.iter().map(|b| &b.key).collect::<Vec<_>>());
    assert_eq!(report.unstamped, 0, "no unstamped expected");
    assert_eq!(report.skipped, 0, "no skipped expected");

    // Find each bucket by key.
    let dream_bucket = report.actors.iter().find(|b| b.key == "dream").expect("dream bucket");
    let build_bucket = report.actors.iter().find(|b| b.key == "build").expect("build bucket");
    // Shell files have no Claude link → fall back to innermost chain link "xterm".
    let shell_bucket = report.actors.iter().find(|b| b.key == "xterm").expect("xterm bucket");

    assert_eq!(dream_bucket.file_count, 2, "dream file_count");
    assert_eq!(dream_bucket.total_bytes, 30, "dream total_bytes");

    assert_eq!(build_bucket.file_count, 3, "build file_count");
    assert_eq!(build_bucket.total_bytes, 15, "build total_bytes");

    assert_eq!(shell_bucket.file_count, 1, "xterm file_count");
    assert_eq!(shell_bucket.total_bytes, 100, "xterm total_bytes");
}

// ── AC3: target/ subdir is counted in skipped, NOT unstamped ─────────────────

#[test]
fn ac3_target_subdir_counted_in_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // A target/ subdir with two files (no xattrs set — doesn't matter, they're skipped).
    let target_dir = root.join("target");
    fs::create_dir(&target_dir).expect("create target/");
    fs::write(target_dir.join("foo.rlib"), b"abc").expect("write foo.rlib");
    fs::write(target_dir.join("bar.d"), b"xyz").expect("write bar.d");

    // One normal attributed file (also no xattr — will be unstamped, but distinct from skipped).
    fs::write(root.join("normal.rs"), b"fn main() {}").expect("write normal.rs");

    let opts = AttributeOpts::default();
    let report = attribute(root, &opts);

    // target/ files → skipped = 2
    assert_eq!(report.skipped, 2, "expected 2 skipped entries for target/");
    // normal.rs with no xattr → unstamped = 1
    assert_eq!(report.unstamped, 1, "expected 1 unstamped entry");
    // No actor buckets.
    assert_eq!(report.actors.len(), 0, "expected no actor buckets");
}

// ── AC4: Non-skipped file with no xattr → unstamped ──────────────────────────

#[test]
fn ac4_unstamped_file_no_xattr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // Write a file with no xattr (written before provfs booted, or on ext4 without xattr).
    fs::write(root.join("old.txt"), b"hello").expect("write old.txt");

    let opts = AttributeOpts::default();
    let report = attribute(root, &opts);

    assert_eq!(report.unstamped, 1, "expected 1 unstamped");
    assert_eq!(report.skipped, 0, "expected 0 skipped");
    assert_eq!(report.actors.len(), 0, "expected 0 actor buckets");
    assert_eq!(report.total_files, 1, "expected 1 total file");
}

// ── AC5: --by skill groups correctly; --top N caps buckets ───────────────────

#[test]
fn ac5_top_n_caps_buckets_headline_counts_all() {
    if !setfattr_available() {
        eprintln!("AC5 SKIP: setfattr not available");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // 3 actors, each 1 file.
    let actors = [
        ("dream.txt", "comm-chain:bash>claude-dream-helio;cwd:/x;pid:1;uid:0", 100u64),
        ("build.txt", "comm-chain:bash>claude-build-main;cwd:/x;pid:2;uid:0", 50u64),
        ("shell.txt", "comm-chain:zsh>xterm;cwd:/x;pid:3;uid:0", 10u64),
    ];

    for (name, prov, size) in &actors {
        let path = root.join(name);
        let data = vec![b'x'; usize::try_from(*size).unwrap_or(0)];
        fs::write(&path, &data).expect("write");
        if !set_prov(&path, prov) {
            eprintln!("AC5 SKIP: setfattr failed");
            return;
        }
    }

    // --top 2 should return only 2 actor buckets.
    let opts = AttributeOpts {
        max_depth: 4,
        top_n: 2,
        min_bytes: 0,
        group_by: GroupBy::Skill,
    };
    let report = attribute(root, &opts);

    // Headline total covers all 3 files.
    assert_eq!(report.total_files, 3, "headline total_files");
    assert_eq!(report.total_bytes, 160, "headline total_bytes");

    // Only top 2 buckets in output.
    assert_eq!(report.actors.len(), 2, "top_n capped at 2");
    // Top bucket by bytes should be dream (100).
    assert_eq!(report.actors[0].key, "dream", "top bucket = dream");
    assert_eq!(report.actors[1].key, "build", "second bucket = build");
}

#[test]
fn ac5_by_cwd_groups_by_cwd() {
    if !setfattr_available() {
        eprintln!("AC5 by_cwd SKIP: setfattr not available");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // Two files written from the same cwd — should merge into one bucket.
    let f1 = root.join("a.txt");
    let f2 = root.join("b.txt");
    fs::write(&f1, b"aaa").expect("write f1");
    fs::write(&f2, b"bbb").expect("write f2");
    let prov1 = "comm-chain:bash>claude-dream-x;cwd:/home/jsy/proj;pid:1;uid:0";
    let prov2 = "comm-chain:bash>claude-build-y;cwd:/home/jsy/proj;pid:2;uid:0";

    if !set_prov(&f1, prov1) || !set_prov(&f2, prov2) {
        eprintln!("AC5 by_cwd SKIP: setfattr failed");
        return;
    }

    let opts = AttributeOpts {
        max_depth: 4,
        top_n: 10,
        min_bytes: 0,
        group_by: GroupBy::Cwd,
    };
    let report = attribute(root, &opts);

    // Both files share cwd=/home/jsy/proj → one bucket.
    assert_eq!(report.actors.len(), 1, "should group by cwd");
    assert_eq!(report.actors[0].key, "/home/jsy/proj");
    assert_eq!(report.actors[0].file_count, 2);
}

// ── AC6: Fixture tree is byte-for-byte unchanged after attribute() ────────────

#[test]
fn ac6_attribute_is_readonly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // Write several files.
    let paths_and_data: Vec<(&str, &[u8])> = vec![
        ("alpha.txt", b"hello"),
        ("beta.txt", b"world"),
        ("gamma.rs", b"fn main() {}"),
    ];
    for (name, data) in &paths_and_data {
        fs::write(root.join(name), data).expect("write");
    }

    let opts = AttributeOpts::default();
    let _ = attribute(root, &opts);

    // Verify every file's contents are unchanged.
    for (name, expected) in &paths_and_data {
        let actual = fs::read(root.join(name)).expect("read back");
        assert_eq!(&actual, expected, "file {name} was mutated by attribute()!");
    }
}

// ── AC7: Live integration test (skipped if provfs absent) ────────────────────

#[test]
fn ac7_live_autobuilder_dir() {
    let dir = std::path::Path::new("/home/jsy/wintermute/autobuilder");
    if !dir.exists() {
        eprintln!("AC7 SKIP: ~/wintermute/autobuilder does not exist");
        return;
    }

    // Quick probe: check if any file has a prov xattr. If not, skip.
    let probe = std::process::Command::new("getfattr")
        .args(["-d", "--absolute-names"])
        .arg(dir.join("PRD-colophon-parse.md"))
        .output();

    let has_prov = match probe {
        Ok(out) => String::from_utf8_lossy(&out.stdout).contains("user.prov.session"),
        Err(_) => false,
    };

    if !has_prov {
        eprintln!("AC7 SKIP: provfs xattrs absent on this machine");
        return;
    }

    let opts = AttributeOpts {
        max_depth: 2,
        top_n: 10,
        min_bytes: 0,
        group_by: GroupBy::Skill,
    };
    let report = attribute(dir, &opts);

    // Non-empty report: must have at least 1 actor bucket or stamped files.
    let total_attributed: u64 = report.actors.iter().map(|b| b.file_count).sum();
    assert!(
        total_attributed > 0 || report.unstamped > 0,
        "AC7 FAIL: live attribution produced empty report (total_files={})",
        report.total_files
    );
    eprintln!(
        "AC7 PASS: {} files, {} actors, {} unstamped",
        report.total_files,
        report.actors.len(),
        report.unstamped
    );
}
