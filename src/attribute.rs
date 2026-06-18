//! Tree-level provenance attribution — walk a directory, classify each file by
//! which session/skill wrote it, and return a ranked [`Attribution`] report.
//!
//! # Overview
//!
//! ```rust
//! use colophon::attribute::{attribute, AttributeOpts, GroupBy};
//! use std::path::Path;
//!
//! let opts = AttributeOpts {
//!     max_depth: 32,
//!     top_n: 10,
//!     min_bytes: 0,
//!     group_by: GroupBy::Skill,
//! };
//! let report = attribute(Path::new("/some/dir"), &opts);
//! assert!(report.total_files < u64::MAX);
//! ```
#![allow(clippy::print_stdout, clippy::print_stderr)]

use crate::{read_file, Provenance, Skill};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ── Public types ─────────────────────────────────────────────────────────────

/// How to group files when building actor buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GroupBy {
    /// Group by inferred wintermute skill (dream/build/self-review/claude).
    Skill,
    /// Group by the innermost comm-chain link (direct writer process name).
    Actor,
    /// Group by the writing cwd recorded in the provenance.
    Cwd,
}

/// One actor bucket in the [`Attribution`] report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActorBucket {
    /// Grouping key (skill name, actor name, or cwd).
    pub key: String,
    /// Number of files in this bucket.
    pub file_count: u64,
    /// Total size in bytes of all files in this bucket.
    pub total_bytes: u64,
    /// Oldest write timestamp seen in this bucket (Unix seconds), if any.
    pub oldest_ts: Option<u64>,
    /// Newest write timestamp seen in this bucket (Unix seconds), if any.
    pub newest_ts: Option<u64>,
    /// Up to 5 representative paths.
    pub sample_paths: Vec<PathBuf>,
}

/// Walk options controlling depth, filtering, and output.
#[derive(Debug, Clone)]
pub struct AttributeOpts {
    /// Maximum directory depth to descend (1 = files in `root` only).
    pub max_depth: usize,
    /// Emit only the top-N largest buckets (by `total_bytes`) in the ranked
    /// output. The headline total counts all files regardless of this filter.
    pub top_n: usize,
    /// Suppress buckets whose `total_bytes` is below this threshold.
    pub min_bytes: u64,
    /// Grouping key for actor buckets.
    pub group_by: GroupBy,
}

impl Default for AttributeOpts {
    fn default() -> Self {
        Self {
            max_depth: 64,
            top_n: 10,
            min_bytes: 0,
            group_by: GroupBy::Skill,
        }
    }
}

/// Full attribution report for a directory tree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Attribution {
    /// The root path that was walked.
    pub root: PathBuf,
    /// Total files visited (excludes skipped-by-design paths).
    pub total_files: u64,
    /// Total bytes of visited files (excludes skipped-by-design paths).
    pub total_bytes: u64,
    /// Ranked actor buckets (filtered/capped by `top_n` and `min_bytes`).
    pub actors: Vec<ActorBucket>,
    /// Count of skip-prefixed entries (target/, .git/, etc.) — not stamped, not unstamped.
    pub skipped: u64,
    /// Count of non-skipped files with no `user.prov.session` xattr.
    pub unstamped: u64,
}

// ── Skip-prefix logic ────────────────────────────────────────────────────────

/// Directory names that provfs intentionally never stamps.
/// Must mirror `provfs_lsm.c` skip list.
static SKIP_NAMES: &[&str] = &["target", ".git", "node_modules"];

/// Return true if any component of `path` is a known skip-prefix directory.
///
/// We check each component so that `some/deep/target/file` is also skipped.
#[must_use]
fn is_skip_prefixed(path: &Path) -> bool {
    path.components().any(|c| {
        if let std::path::Component::Normal(name) = c {
            SKIP_NAMES.iter().any(|s| name == std::ffi::OsStr::new(s))
        } else {
            false
        }
    })
}

// ── Grouping key extraction ───────────────────────────────────────────────────

/// Derive the grouping key from a `Provenance` and the chosen `GroupBy`.
/// Returns `None` for `Unstamped` provenance (caller puts it in unstamped bucket).
#[must_use]
fn bucket_key(prov: &Provenance, group_by: GroupBy) -> Option<String> {
    use crate::Form;
    if prov.form == Form::Unstamped {
        return None;
    }
    match group_by {
        GroupBy::Skill => {
            // Map the optional Skill to a canonical string; fall back to the
            // innermost comm-chain link (or cwd when chain is empty).
            match prov.originating_skill() {
                Some(Skill::Dream) => Some("dream".to_owned()),
                Some(Skill::Build) => Some("build".to_owned()),
                Some(Skill::SelfReview) => Some("self-review".to_owned()),
                Some(Skill::Claude) => Some("claude".to_owned()),
                None => {
                    // No Claude link — use innermost comm-chain link or cwd.
                    prov.comm_chain
                        .last()
                        .cloned()
                        .or_else(|| prov.cwd.clone())
                        .or_else(|| Some(prov.raw.chars().take(32).collect()))
                }
            }
        }
        GroupBy::Actor => {
            // Use the innermost comm-chain link (the direct writing process).
            prov.comm_chain
                .last()
                .cloned()
                .or_else(|| prov.cwd.clone())
                .or_else(|| Some(prov.raw.chars().take(32).collect()))
        }
        GroupBy::Cwd => {
            prov.cwd
                .clone()
                .or_else(|| prov.comm_chain.last().cloned())
                .or_else(|| Some(prov.raw.chars().take(32).collect()))
        }
    }
}

// ── Core walk ────────────────────────────────────────────────────────────────

/// Accumulator passed through the walk closure.
struct WalkState {
    actor_map: BTreeMap<String, ActorBucket>,
    skipped: u64,
    unstamped: u64,
    total_files: u64,
    total_bytes: u64,
}

/// Classify one file into the walk state.
///
/// `rel_path` is the path relative to the walk root (used for skip-prefix
/// checks). `abs_path` is the absolute path used for `getfattr` I/O.
fn classify_file(
    rel_path: &Path,
    abs_path: &Path,
    file_bytes: u64,
    group_by: GroupBy,
    state: &mut WalkState,
) {
    // Skip-prefixed paths: count and skip entirely.
    if is_skip_prefixed(rel_path) {
        state.skipped = state.skipped.saturating_add(1);
        return;
    }

    state.total_files = state.total_files.saturating_add(1);
    state.total_bytes = state.total_bytes.saturating_add(file_bytes);

    // Read provenance from xattrs; treat I/O error as unstamped.
    let Ok(prov) = read_file(abs_path) else {
        state.unstamped = state.unstamped.saturating_add(1);
        return;
    };

    match bucket_key(&prov, group_by) {
        None => {
            // Unstamped (Form::Unstamped).
            state.unstamped = state.unstamped.saturating_add(1);
        }
        Some(key) => {
            let bucket = state
                .actor_map
                .entry(key.clone())
                .or_insert_with(|| ActorBucket {
                    key,
                    file_count: 0,
                    total_bytes: 0,
                    oldest_ts: None,
                    newest_ts: None,
                    sample_paths: Vec::new(),
                });
            bucket.file_count = bucket.file_count.saturating_add(1);
            bucket.total_bytes = bucket.total_bytes.saturating_add(file_bytes);
            if let Some(file_ts) = prov.ts {
                bucket.oldest_ts =
                    Some(bucket.oldest_ts.map_or(file_ts, |existing| existing.min(file_ts)));
                bucket.newest_ts =
                    Some(bucket.newest_ts.map_or(file_ts, |existing| existing.max(file_ts)));
            }
            if bucket.sample_paths.len() < 5 {
                bucket.sample_paths.push(abs_path.to_path_buf());
            }
        }
    }
}

/// Walk `root` (no symlink-follow, bounded by `opts.max_depth`) and aggregate
/// file provenance into an [`Attribution`] report.
///
/// I/O errors on individual files are silently treated as `unstamped` so a
/// single unreadable file does not abort the entire walk.
#[must_use]
pub fn attribute(root: &Path, opts: &AttributeOpts) -> Attribution {
    let mut state = WalkState {
        actor_map: BTreeMap::new(),
        skipped: 0,
        unstamped: 0,
        total_files: 0,
        total_bytes: 0,
    };

    walk(root, root, 0, opts, &mut state);

    // Rank buckets by total_bytes descending, then apply top_n / min_bytes.
    let mut actors: Vec<ActorBucket> = state.actor_map.into_values().collect();
    actors.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));
    actors.retain(|b| b.total_bytes >= opts.min_bytes);
    actors.truncate(opts.top_n);

    Attribution {
        root: root.to_path_buf(),
        total_files: state.total_files,
        total_bytes: state.total_bytes,
        actors,
        skipped: state.skipped,
        unstamped: state.unstamped,
    }
}

/// Recursive directory walker (no symlink-follow).
fn walk(path: &Path, root: &Path, depth: usize, opts: &AttributeOpts, state: &mut WalkState) {
    if depth > opts.max_depth {
        return;
    }

    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };

    if meta.is_symlink() {
        // Never follow symlinks.
        return;
    }

    if meta.is_file() {
        let rel = path.strip_prefix(root).unwrap_or(path);
        classify_file(rel, path, meta.len(), opts.group_by, state);
        return;
    }

    if meta.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            walk(&entry.path(), root, depth + 1, opts, state);
        }
    }
}

// ── Formatting ───────────────────────────────────────────────────────────────

/// Format a byte count as a human-readable string (B / KiB / MiB / GiB).
#[must_use]
pub fn fmt_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    #[allow(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        clippy::float_arithmetic
    )]
    if bytes >= GIB {
        format!("{:.1}G", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1}M", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1}K", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes}B")
    }
}

/// Print the [`Attribution`] report as a human-readable ranked table.
pub fn print_text(report: &Attribution, top_n: usize) {
    // Headline.
    println!(
        "{} across {} files in {}",
        fmt_bytes(report.total_bytes),
        report.total_files,
        report.root.display()
    );
    if report.skipped > 0 {
        println!("  skipped (target/.git/…): {} entries", report.skipped);
    }
    if report.unstamped > 0 {
        println!("  unstamped (no xattr):    {} files", report.unstamped);
    }
    if report.actors.is_empty() {
        println!("  (no attributed files)");
        return;
    }
    println!();
    println!("{:<30} {:>10}  {:>8}  sample", "actor", "bytes", "files");
    println!("{}", "-".repeat(72));
    for bucket in report.actors.iter().take(top_n) {
        let sample = bucket
            .sample_paths
            .first()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        println!(
            "{:<30} {:>10}  {:>8}  {}",
            bucket.key,
            fmt_bytes(bucket.total_bytes),
            bucket.file_count,
            sample
        );
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_skip_prefixed_target() {
        assert!(is_skip_prefixed(Path::new("target/debug/foo")));
        assert!(is_skip_prefixed(Path::new("some/nested/target/file")));
        assert!(is_skip_prefixed(Path::new(".git/config")));
        assert!(!is_skip_prefixed(Path::new("src/main.rs")));
    }

    #[test]
    fn bucket_key_skill_dream() {
        let prov = crate::parse(
            "comm-chain:Bun Pool 3>claude-dream-helio;cwd:/x;pid:1;uid:0",
            None,
        );
        assert_eq!(
            bucket_key(&prov, GroupBy::Skill),
            Some("dream".to_owned())
        );
    }

    #[test]
    fn bucket_key_skill_build() {
        let prov = crate::parse("comm-chain:bash>claude-build-main;cwd:/x;pid:1;uid:0", None);
        assert_eq!(
            bucket_key(&prov, GroupBy::Skill),
            Some("build".to_owned())
        );
    }

    #[test]
    fn bucket_key_no_skill_falls_back_to_actor() {
        let prov =
            crate::parse("comm-chain:bash>zsh>xterm;cwd:/home/jsy;pid:1;uid:0", None);
        // No Claude link — falls back to innermost chain link.
        assert_eq!(
            bucket_key(&prov, GroupBy::Skill),
            Some("xterm".to_owned())
        );
    }

    #[test]
    fn bucket_key_unstamped_is_none() {
        let prov = crate::parse("", None);
        assert_eq!(bucket_key(&prov, GroupBy::Skill), None);
    }

    #[test]
    fn fmt_bytes_ranges() {
        assert_eq!(fmt_bytes(0), "0B");
        assert_eq!(fmt_bytes(1023), "1023B");
        assert_eq!(fmt_bytes(1024), "1.0K");
        assert_eq!(fmt_bytes(1024 * 1024), "1.0M");
        assert_eq!(fmt_bytes(1024 * 1024 * 1024), "1.0G");
    }
}
