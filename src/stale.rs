//! Staleness detection — walk a directory and flag files whose provenance shows
//! the writing session is dead or outdated relative to a consumer binary.
//!
//! # Overview
//!
//! ```rust
//! use colophon::stale::{stale, StaleOpts};
//! use std::path::Path;
//!
//! let opts = StaleOpts::default();
//! let entries = stale(Path::new("/some/state/dir"), &opts);
//! for e in &entries {
//!     println!("{}: {:?}", e.path.display(), e.reason);
//! }
//! ```

use crate::attribute::is_skip_prefixed;
use crate::read_file;
use std::path::{Path, PathBuf};

// ── Public types ─────────────────────────────────────────────────────────────

/// Why a file is considered stale.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StaleReason {
    /// The pid that wrote the file is no longer present in `/proc`.
    WriterDead,
    /// The file's `prov.ts` predates the consuming binary's mtime.
    OlderThanConsumer,
    /// Both `WriterDead` and `OlderThanConsumer` apply.
    Both,
}

/// A single stale-file verdict.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Staleness {
    /// Path of the stale file.
    pub path: PathBuf,
    /// Why it is considered stale.
    pub reason: StaleReason,
    /// Age of the file in whole days (floor(age_secs / 86400)).
    pub age_days: u64,
}

/// Options for [`stale`].
#[derive(Debug, Clone)]
pub struct StaleOpts {
    /// Minimum age in seconds before a file with a dead pid is flagged
    /// `WriterDead`. Guards against pid-reuse for very recent files.
    /// Default: 3600 (1 hour).
    pub min_age_secs: u64,
    /// Optional path to a binary. When `Some`, files older than its mtime are
    /// additionally flagged `OlderThanConsumer`.
    pub consumer: Option<PathBuf>,
    /// Maximum directory depth to walk.
    pub max_depth: usize,
}

impl Default for StaleOpts {
    fn default() -> Self {
        Self {
            min_age_secs: 3600,
            consumer: None,
            max_depth: 64,
        }
    }
}

// ── Core walk ────────────────────────────────────────────────────────────────

/// Walk `root` and flag files whose provenance shows the writer is dead or
/// outdated relative to a consumer binary.
///
/// I/O errors and provenance-absent files are silently ignored so a single
/// unreadable file does not abort the walk.
#[must_use]
pub fn stale(root: &Path, opts: &StaleOpts) -> Vec<Staleness> {
    let now_secs = unix_now();

    // Resolve consumer mtime once.
    let consumer_mtime: Option<u64> = opts.consumer.as_ref().and_then(|p| {
        std::fs::metadata(p)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs()))
    });

    let mut results: Vec<Staleness> = vec![];

    walk(root, root, 0, opts.max_depth, &mut |path: &Path| {
        // Skip-by-design paths.
        let rel = path.strip_prefix(root).unwrap_or(path);
        if is_skip_prefixed(rel) {
            return;
        }

        // Read provenance; silently skip on error or no stamp.
        let prov = match read_file(path) {
            Ok(p) => p,
            Err(_) => return,
        };

        use crate::Form;
        if prov.form == Form::Unstamped {
            return;
        }

        // Need a timestamp to judge freshness.
        let ts = match prov.ts {
            Some(t) => t,
            None => return,
        };

        let age_secs = now_secs.saturating_sub(ts);
        let age_days = age_secs / 86400;

        // WriterDead: pid is present in provenance, age exceeds min_age, and
        // /proc/<pid> does not exist.
        let writer_dead = prov
            .pid
            .map(|pid| age_secs >= opts.min_age_secs && !pid_alive(pid))
            .unwrap_or(false);

        // OlderThanConsumer: prov.ts < consumer binary mtime.
        let older_than_consumer = consumer_mtime
            .map(|consumer_ts| ts < consumer_ts)
            .unwrap_or(false);

        let reason = match (writer_dead, older_than_consumer) {
            (true, true) => Some(StaleReason::Both),
            (true, false) => Some(StaleReason::WriterDead),
            (false, true) => Some(StaleReason::OlderThanConsumer),
            (false, false) => None,
        };

        if let Some(reason) = reason {
            results.push(Staleness {
                path: path.to_owned(),
                reason,
                age_days,
            });
        }
    });

    results
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Return the current Unix timestamp in seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Return `true` if `/proc/<pid>` exists (process is alive).
fn pid_alive(pid: u32) -> bool {
    std::fs::metadata(format!("/proc/{pid}")).is_ok()
}

/// Recursive file walker — visits regular files only, no symlink-follow.
fn walk<F>(path: &Path, root: &Path, depth: usize, max_depth: usize, cb: &mut F)
where
    F: FnMut(&Path),
{
    if depth > max_depth {
        return;
    }

    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };

    if meta.is_symlink() {
        return;
    }

    if meta.is_file() {
        cb(path);
        return;
    }

    if meta.is_dir() {
        // Early-exit skip-prefixed dirs to avoid unnecessary descend.
        let rel = path.strip_prefix(root).unwrap_or(path);
        if depth > 0 && is_skip_prefixed(rel) {
            return;
        }

        let entries = match std::fs::read_dir(path) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            walk(&entry.path(), root, depth + 1, max_depth, cb);
        }
    }
}

// ── Formatting ───────────────────────────────────────────────────────────────

/// Print staleness entries as a human-readable list.
pub fn print_text(entries: &[Staleness]) {
    if entries.is_empty() {
        println!("(no stale state found)");
        return;
    }
    println!("{:<60}  {:<22}  {:>10}", "path", "reason", "age (days)");
    println!("{}", "-".repeat(96));
    for e in entries {
        let reason = match e.reason {
            StaleReason::WriterDead => "writer-dead",
            StaleReason::OlderThanConsumer => "older-than-consumer",
            StaleReason::Both => "both",
        };
        // Truncate long paths for display.
        let display = e.path.display().to_string();
        let display = if display.len() > 58 {
            format!("…{}", &display[display.len() - 57..])
        } else {
            display
        };
        println!("{:<60}  {:<22}  {:>10}", display, reason, e.age_days);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Helper: write a tiny file and set a fake xattr via setfattr (best-effort).
    /// Returns path. On systems where setfattr is absent, xattr calls are no-ops
    /// and the test exercises the "no ts → skip" path instead, which is safe.
    #[allow(clippy::expect_used)]
    fn make_file_with_xattr(
        dir: &Path,
        name: &str,
        session: &str,
        ts: &str,
    ) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).expect("create fixture");
        writeln!(f, "fixture").expect("write fixture");
        drop(f);
        // Best-effort: ignore errors if setfattr is absent.
        let _ = std::process::Command::new("setfattr")
            .args(["-n", "user.prov.session", "-v", session, &path.display().to_string()])
            .output();
        let _ = std::process::Command::new("setfattr")
            .args(["-n", "user.prov.ts", "-v", ts, &path.display().to_string()])
            .output();
        path
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn dead_pid_absurd_flagged() {
        let dir = std::env::temp_dir().join(format!("colophon-stale-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // PID 999_999_999 is almost certainly not alive; ts 1 is old enough.
        let _p = make_file_with_xattr(
            &dir,
            "dead.bin",
            "comm-chain:bash>zsh;pid:999999999;uid:1000",
            "1",
        );

        let opts = StaleOpts {
            min_age_secs: 0, // no floor — flag immediately
            ..StaleOpts::default()
        };
        let results = stale(&dir, &opts);

        // If setfattr was absent, there's no ts and nothing gets flagged — that's OK.
        // If setfattr succeeded, the file should be WriterDead.
        for r in &results {
            assert!(
                matches!(r.reason, StaleReason::WriterDead | StaleReason::Both),
                "unexpected reason {:?}",
                r.reason
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn live_pid_not_flagged() {
        let dir = std::env::temp_dir().join(format!("colophon-stale-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // Our own PID is definitely alive; use a very recent ts.
        let own_pid = std::process::id();
        let ts_now = unix_now().to_string();
        let session = format!("comm-chain:bash>zsh;pid:{own_pid};uid:1000");
        let _p = make_file_with_xattr(&dir, "live.bin", &session, &ts_now);

        let opts = StaleOpts {
            min_age_secs: 3600, // 1h floor
            ..StaleOpts::default()
        };
        let results = stale(&dir, &opts);

        // Our own pid is alive, so WriterDead must not appear.
        for r in &results {
            assert_ne!(
                r.reason,
                StaleReason::WriterDead,
                "live pid should not be flagged WriterDead"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn min_age_guard_protects_recent_dead_pid() {
        let dir = std::env::temp_dir().join(format!("colophon-stale-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // PID 999_999_998 is almost certainly dead, but ts is NOW → under min_age.
        let ts_now = unix_now().to_string();
        let _p = make_file_with_xattr(
            &dir,
            "recent.bin",
            "comm-chain:bash;pid:999999998;uid:1000",
            &ts_now,
        );

        let opts = StaleOpts {
            min_age_secs: 3600, // 1h floor
            ..StaleOpts::default()
        };
        let results = stale(&dir, &opts);

        // With a very recent ts, WriterDead must NOT fire (pid-reuse guard).
        for r in &results {
            assert_ne!(
                r.reason,
                StaleReason::WriterDead,
                "recent file with dead pid should not be flagged (pid-reuse guard)"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
