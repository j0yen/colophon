//! `colophon` — canonical decoder for the wintermute kernel's provfs provenance xattrs, with
//! tree-level attribution reporting.
//!
//! The [`attribute`] module adds `fn attribute(root, opts) -> Attribution` and the
//! `colophon attribute <dir>` subcommand for cruft-origin analysis.
//!
//! The [`stale`] module adds `fn stale(root, opts) -> Vec<Staleness>` and the
//! `colophon stale <dir>` subcommand for dead-session detection.
//!
//! The [`digest`] module adds `fn digest(opts) -> Digest` and the
//! `colophon digest` subcommand that composes attribution + stale into one block.
//!
//! The booted `7.0.11-arch1-1-wintermute` kernel stamps a structured
//! `user.prov.session` xattr on every file. This crate is the single decoder
//! for both forms the kernel emits:
//!
//! - **`CommChain`**: `comm-chain:<c0>>c1>c2;env:<KEY>=<val>;cwd:<path>;pid:<p>;uid:<u>`
//! - **`AgentId`**: a 32-hex-char (128-bit) agent namespace id
//! - **`Unstamped`**: empty, all-zero, or absent xattr
//!
//! ## Quick start
//!
//! ```rust
//! use colophon::parse;
//!
//! let p = parse(
//!     "comm-chain:cat>zsh>claude;cwd:/home/jsy;pid:1000;uid:1000",
//!     None,
//! );
//! assert_eq!(p.comm_chain, ["cat", "zsh", "claude"]);
//! ```

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod attribute;
pub mod digest;
pub mod stale;

// ── Public types ────────────────────────────────────────────────────────────

/// Discriminant for the two live xattr forms plus the unstamped case.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Form {
    /// `comm-chain:…` enriched form with per-field decomposition.
    CommChain,
    /// 32-hex-char agentns namespace id (future form when `CLONE_NEWAGENT` lands).
    AgentId,
    /// Empty, all-zero, or absent xattr — the file was not stamped.
    Unstamped,
}

/// A wintermute skill identity extracted from the comm-chain.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Skill {
    /// `/dream` pass — claude-dream* link in the chain.
    Dream,
    /// `/build` pass — claude-build* link in the chain.
    Build,
    /// `/self-review` pass — claude-self* link in the chain.
    SelfReview,
    /// A Claude link without a skill-specific prefix.
    Claude,
}

/// Decoded provenance from a `user.prov.session` + `user.prov.ts` xattr pair.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Provenance {
    /// Which form of the xattr this was decoded from.
    pub form: Form,
    /// Comm-chain links, ordered from outermost to innermost process.
    /// Empty for `AgentId` and `Unstamped` forms.
    pub comm_chain: Vec<String>,
    /// Environment variables embedded in the session xattr (`env:<KEY>=<val>`).
    pub env: BTreeMap<String, String>,
    /// Working directory at write time (`cwd:<path>`).
    pub cwd: Option<String>,
    /// PID of the writing process (`pid:<p>`).
    pub pid: Option<u32>,
    /// UID of the writing process (`uid:<u>`).
    pub uid: Option<u32>,
    /// Raw 128-bit agent-session id (hex, 32 chars) when `form == AgentId`.
    pub agent_session: Option<String>,
    /// Unix timestamp from `user.prov.ts`, if present.
    pub ts: Option<u64>,
    /// The raw session xattr value before decoding.
    pub raw: String,
}

impl Provenance {
    /// Return the originating wintermute skill inferred from the comm-chain,
    /// or `None` when the chain has no Claude link.
    ///
    /// Priority order: Dream > Build > `SelfReview` > Claude.
    #[must_use]
    pub fn originating_skill(&self) -> Option<Skill> {
        // Walk the chain once, recording the highest-priority match found.
        let mut found: Option<Skill> = None;
        for link in &self.comm_chain {
            let lower = link.to_lowercase();
            if lower.starts_with("claude-dream") {
                return Some(Skill::Dream); // highest priority — return immediately
            } else if lower.starts_with("claude-build") {
                // higher than SelfReview and plain Claude
                match found {
                    None | Some(Skill::Claude | Skill::SelfReview) => {
                        found = Some(Skill::Build);
                    }
                    Some(Skill::Build | Skill::Dream) => {}
                }
            } else if lower.starts_with("claude-self") {
                match found {
                    None | Some(Skill::Claude) => {
                        found = Some(Skill::SelfReview);
                    }
                    Some(Skill::Build | Skill::SelfReview | Skill::Dream) => {}
                }
            } else if (lower == "claude"
                || lower.starts_with("claude-")
                || is_bun_pool_claude(link))
                && found.is_none()
            {
                found = Some(Skill::Claude);
            }
        }
        found
    }
}

// ── Core parse functions ─────────────────────────────────────────────────────

/// Parse a `user.prov.session` value and an optional `user.prov.ts` value
/// into a [`Provenance`].
///
/// This function is **pure and total**: it never panics and accepts any input,
/// including empty, malformed, truncated, or reordered fields.
#[must_use]
pub fn parse(session: &str, ts: Option<&str>) -> Provenance {
    let ts_parsed: Option<u64> = ts.and_then(|s| s.trim().parse().ok());
    let raw = session.to_owned();
    let trimmed = session.trim();

    if trimmed.is_empty() {
        return Provenance::unstamped(raw, ts_parsed);
    }
    if is_agent_id(trimmed) {
        return parse_agent_id(trimmed, raw, ts_parsed);
    }
    parse_comm_chain(trimmed, raw, ts_parsed)
}

impl Provenance {
    #[allow(clippy::missing_const_for_fn)] // vec! and BTreeMap::new aren't const
    fn unstamped(raw: String, ts: Option<u64>) -> Self {
        Self {
            form: Form::Unstamped,
            comm_chain: vec![],
            env: BTreeMap::new(),
            cwd: None,
            pid: None,
            uid: None,
            agent_session: None,
            ts,
            raw,
        }
    }
}

/// Parse a 32-hex-char value as `AgentId` or all-zero `Unstamped`.
fn parse_agent_id(trimmed: &str, raw: String, ts: Option<u64>) -> Provenance {
    let is_zero = trimmed.chars().all(|c| c == '0');
    Provenance {
        form: if is_zero { Form::Unstamped } else { Form::AgentId },
        comm_chain: vec![],
        env: BTreeMap::new(),
        cwd: None,
        pid: None,
        uid: None,
        agent_session: if is_zero { None } else { Some(trimmed.to_owned()) },
        ts,
        raw,
    }
}

/// Parse a comm-chain (`;`-delimited) session string.
fn parse_comm_chain(trimmed: &str, raw: String, ts: Option<u64>) -> Provenance {
    let mut comm_chain: Vec<String> = vec![];
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let mut cwd: Option<String> = None;
    let mut pid: Option<u32> = None;
    let mut uid: Option<u32> = None;

    for segment in trimmed.split(';') {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        if let Some(rest) = seg.strip_prefix("comm-chain:") {
            comm_chain = rest
                .split('>')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
        } else if let Some(rest) = seg.strip_prefix("env:") {
            // env:<KEY>=<val> — only split on the first '='
            if let Some(eq) = rest.find('=') {
                let key = rest[..eq].to_owned();
                let val = rest[eq + 1..].to_owned();
                if !key.is_empty() {
                    env.insert(key, val);
                }
            }
            // no '=' → ignore malformed env field
        } else if let Some(rest) = seg.strip_prefix("cwd:") {
            cwd = Some(rest.to_owned());
        } else if let Some(rest) = seg.strip_prefix("pid:") {
            pid = rest.trim().parse().ok();
        } else if let Some(rest) = seg.strip_prefix("uid:") {
            uid = rest.trim().parse().ok();
        }
        // Unknown fields are silently ignored (forward-compatibility).
    }

    // All non-empty, non-hex-id values are treated as CommChain form.
    Provenance {
        form: Form::CommChain,
        comm_chain,
        env,
        cwd,
        pid,
        uid,
        agent_session: None,
        ts,
        raw,
    }
}

/// Read provenance from a file's xattrs by shelling out to `getfattr`.
///
/// Returns `Form::Unstamped` (not an error) when the file has no
/// `user.prov.session` xattr.
///
/// # Errors
///
/// Returns `Err` when the `getfattr` process cannot be spawned or when its
/// output cannot be read. A missing xattr is **not** an error.
pub fn read_file(path: &std::path::Path) -> io::Result<Provenance> {
    let output = Command::new("getfattr")
        .arg("-d")
        .arg("--absolute-names")
        .arg(path)
        .output()?;

    // getfattr exits non-zero when no xattrs exist on some versions; treat
    // that the same as empty output rather than propagating as an error.
    let text = String::from_utf8_lossy(&output.stdout);

    let session = extract_xattr_value(&text, "user.prov.session");
    let ts = extract_xattr_value(&text, "user.prov.ts");

    Ok(parse(
        session.as_deref().unwrap_or(""),
        ts.as_deref(),
    ))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Return true iff `s` is exactly 32 lowercase hex characters.
fn is_agent_id(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Returns `true` for the `Bun Pool N>claude` link pattern seen in real xattrs.
fn is_bun_pool_claude(link: &str) -> bool {
    // Real sample: "Bun Pool 3>claude-dream-he…"
    // The link already had `>` stripped by the comm-chain splitter, so the
    // remaining text is the individual process name.
    let lower = link.to_lowercase();
    lower.starts_with("bun pool")
}

/// Extract a named xattr value from `getfattr -d` output.
///
/// The output looks like:
/// ```text
/// # file: path/to/file
/// user.prov.session="comm-chain:cat>zsh>claude;..."
/// user.prov.ts="1781768028"
/// ```
fn extract_xattr_value(getfattr_output: &str, name: &str) -> Option<String> {
    for line in getfattr_output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(name) {
            if let Some(rest2) = rest.strip_prefix('=') {
                // Value is double-quoted.
                let val = rest2.trim_matches('"');
                return Some(val.to_owned());
            }
        }
    }
    None
}

// ── Stale detection ──────────────────────────────────────────────────────────

/// Why a file was flagged as stale.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum StaleReason {
    /// The stamped pid is no longer present in `/proc` and the file's `prov.ts`
    /// is older than the `min_age` floor (guarding against pid reuse).
    WriterDead,
    /// The file's `prov.ts` is earlier than the consumer binary's mtime.
    OlderThanConsumer,
    /// Both `WriterDead` and `OlderThanConsumer` apply.
    Both,
}

/// A staleness verdict for a single file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Staleness {
    /// Absolute path of the flagged file.
    pub path: std::path::PathBuf,
    /// Reason(s) the file was flagged.
    pub reason: StaleReason,
    /// The decoded provenance of the file.
    pub prov: Provenance,
    /// Age of the file's `prov.ts` in whole days from now (0 = less than one day).
    pub age_days: u64,
}

/// Options controlling [`stale`].
#[derive(Debug, Clone)]
pub struct StaleOpts {
    /// Minimum age a file's `prov.ts` must be before the writing pid is checked
    /// for liveness.  Guards against a fresh file whose pid happened to be
    /// recycled.  Default: 3600 seconds (1 h).
    pub min_age_secs: u64,
    /// Path to a consumer binary; when set, files whose `prov.ts` precedes this
    /// binary's mtime are flagged `OlderThanConsumer`.
    pub consumer: Option<std::path::PathBuf>,
    /// Path prefixes to skip (exact-prefix match on the absolute path).
    pub skip_prefixes: Vec<std::path::PathBuf>,
}

impl Default for StaleOpts {
    fn default() -> Self {
        Self {
            min_age_secs: 3600,
            consumer: None,
            skip_prefixes: vec![],
        }
    }
}

/// Walk `root` recursively, parse provenance for each regular file, and return
/// a list of staleness verdicts.
///
/// A file is flagged when:
/// - Its stamped pid is absent from `/proc` AND its `prov.ts` is old enough to
///   rule out pid recycling (`WriterDead`).
/// - Its `prov.ts` is older than the consumer binary's mtime (`OlderThanConsumer`).
/// - Both conditions hold (`Both`).
///
/// Unstamped files (no xattr) are silently skipped.
/// Per-file I/O errors are silently ignored to keep the walk best-effort.
#[must_use]
pub fn stale(root: &Path, opts: &StaleOpts) -> Vec<Staleness> {
    let consumer_mtime: Option<u64> = opts
        .consumer
        .as_ref()
        .and_then(|p| file_mtime_secs(p).ok());

    let now_secs: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    let mut results = Vec::new();

    walk_dir(root, &mut |path: &Path| {
        // Skip-prefix check.
        if opts.skip_prefixes.iter().any(|pfx| path.starts_with(pfx)) {
            return;
        }

        let Ok(prov) = read_file(path) else { return };

        // Ignore unstamped files.
        if prov.form == Form::Unstamped || prov.pid.is_none() {
            return;
        }

        if let Some((reason, age_days)) =
            judge_provenance(&prov, now_secs, consumer_mtime, opts.min_age_secs)
        {
            results.push(Staleness {
                path: path.to_path_buf(),
                reason,
                prov,
                age_days,
            });
        }
    });

    results
}

// ── Stale helpers ────────────────────────────────────────────────────────────

/// Compute a staleness verdict for a single `Provenance` value.
///
/// This function is the pure, testable core of [`stale`]: given already-decoded
/// provenance and the judgment parameters, it decides whether the file is stale
/// and why.
///
/// Returns `None` when the file is not stale.
///
/// - `now_secs`: caller-supplied current time (seconds since UNIX epoch) so tests
///   can inject a deterministic "now".
/// - `consumer_mtime`: the mtime of the consumer binary in seconds since UNIX
///   epoch, or `None` if no consumer was supplied.
#[must_use]
pub fn judge_provenance(
    prov: &Provenance,
    now_secs: u64,
    consumer_mtime: Option<u64>,
    min_age_secs: u64,
) -> Option<(StaleReason, u64)> {
    if prov.form == Form::Unstamped || prov.pid.is_none() {
        return None;
    }
    let ts = prov.ts?;
    let age_secs = now_secs.saturating_sub(ts);
    let age_days = age_secs / 86_400;

    let writer_dead = is_writer_dead(prov.pid, ts, now_secs, min_age_secs);
    let older_than_consumer = consumer_mtime.is_some_and(|cm| ts < cm);

    let reason = match (writer_dead, older_than_consumer) {
        (true, true) => StaleReason::Both,
        (true, false) => StaleReason::WriterDead,
        (false, true) => StaleReason::OlderThanConsumer,
        (false, false) => return None,
    };
    Some((reason, age_days))
}

/// Return `true` when the writing pid is considered truly dead.
///
/// Conditions: the pid is absent from `/proc` AND the file's `prov.ts` is
/// older than `min_age_secs` (to avoid false-positives from pid reuse on a
/// file written very recently).
fn is_writer_dead(pid: Option<u32>, ts: u64, now_secs: u64, min_age_secs: u64) -> bool {
    let Some(pid) = pid else { return false };
    let age_secs = now_secs.saturating_sub(ts);
    if age_secs < min_age_secs {
        return false; // too recent — pid may have been recycled
    }
    !pid_alive(pid)
}

/// Check whether `pid` is alive by testing `/proc/<pid>`.
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Return the mtime of `path` as seconds since UNIX epoch.
fn file_mtime_secs(path: &Path) -> io::Result<u64> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta
        .modified()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let secs = mtime
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    Ok(secs)
}

/// Walk `dir` recursively, calling `cb` for each regular file.
fn walk_dir<F: FnMut(&Path)>(dir: &Path, cb: &mut F) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_dir(&path, cb);
        } else if ft.is_file() {
            cb(&path);
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_is_unstamped() {
        let p = parse("", None);
        assert_eq!(p.form, Form::Unstamped);
        assert!(p.comm_chain.is_empty());
        assert!(p.agent_session.is_none());
    }

    #[test]
    fn parse_all_zero_hex_is_unstamped() {
        let p = parse("00000000000000000000000000000000", None);
        assert_eq!(p.form, Form::Unstamped);
        assert!(p.agent_session.is_none());
    }

    #[test]
    fn parse_nonzero_hex_is_agent_id() {
        let id = "0102030405060708090a0b0c0d0e0f10";
        let p = parse(id, None);
        assert_eq!(p.form, Form::AgentId);
        assert_eq!(p.agent_session.as_deref(), Some(id));
    }

    #[test]
    fn parse_comm_chain_basic() {
        let p = parse(
            "comm-chain:cat>zsh>claude;cwd:/home/jsy/wintermute/autobuilder;pid:2662703;uid:1000",
            None,
        );
        assert_eq!(p.form, Form::CommChain);
        assert_eq!(p.comm_chain, ["cat", "zsh", "claude"]);
        assert_eq!(p.cwd.as_deref(), Some("/home/jsy/wintermute/autobuilder"));
        assert_eq!(p.pid, Some(2_662_703));
        assert_eq!(p.uid, Some(1000));
    }

    #[test]
    fn parse_env_field() {
        let p = parse(
            "comm-chain:bash>claude;env:CLAUDE_TOOL=/build;cwd:/x;pid:1;uid:1000",
            None,
        );
        assert_eq!(p.env.get("CLAUDE_TOOL").map(String::as_str), Some("/build"));
    }

    #[test]
    fn originating_skill_dream() {
        let p = parse(
            "comm-chain:Bun Pool 3>claude-dream-helio;cwd:/x;pid:1;uid:0",
            None,
        );
        assert_eq!(p.originating_skill(), Some(Skill::Dream));
    }

    #[test]
    fn originating_skill_build() {
        let p = parse("comm-chain:bash>claude-build-main;cwd:/x;pid:1;uid:0", None);
        assert_eq!(p.originating_skill(), Some(Skill::Build));
    }

    #[test]
    fn originating_skill_none_for_no_claude() {
        let p = parse("comm-chain:bash>zsh>xterm;cwd:/x;pid:1;uid:0", None);
        assert_eq!(p.originating_skill(), None);
    }

    #[test]
    fn extract_xattr_value_finds_session() {
        let output = "# file: /tmp/foo\nuser.prov.session=\"comm-chain:cat>zsh\"\nuser.prov.ts=\"123\"\n";
        assert_eq!(
            extract_xattr_value(output, "user.prov.session").as_deref(),
            Some("comm-chain:cat>zsh")
        );
        assert_eq!(
            extract_xattr_value(output, "user.prov.ts").as_deref(),
            Some("123")
        );
    }

    // ── Stale / judge_provenance tests ──────────────────────────────────────

    /// Build a minimal `CommChain` provenance with given pid+ts for test use.
    fn make_prov(pid: u32, ts: u64) -> Provenance {
        parse(
            &format!("comm-chain:bash>zsh;cwd:/x;pid:{pid};uid:1000"),
            Some(&ts.to_string()),
        )
    }

    /// A pid that is definitely not alive on any Linux system.
    const DEAD_PID: u32 = 4_000_000; // beyond /proc/sys/kernel/pid_max (usually 4M - 1)

    const NOW: u64 = 1_800_000_000_u64; // arbitrary "now" for tests
    const OLD_TS: u64 = NOW - 7200; // 2 h ago — older than 1 h min_age
    const RECENT_TS: u64 = NOW - 60; // 1 min ago — newer than 1 h min_age
    const MIN_AGE: u64 = 3600; // 1 h

    #[test]
    fn writer_dead_old_dead_pid_flagged() {
        // AC2: dead pid + old ts → WriterDead
        let prov = make_prov(DEAD_PID, OLD_TS);
        let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
        assert!(
            matches!(verdict, Some((StaleReason::WriterDead, _))),
            "expected WriterDead, got {verdict:?}"
        );
    }

    #[test]
    fn writer_dead_live_pid_not_flagged() {
        // AC2: live pid (own process) + old ts → not WriterDead
        let my_pid = std::process::id();
        let prov = make_prov(my_pid, OLD_TS);
        let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
        assert!(
            verdict.is_none(),
            "live pid should not be flagged, got {verdict:?}"
        );
    }

    #[test]
    fn writer_dead_recent_ts_not_flagged() {
        // AC3: dead pid but ts too recent → not flagged (pid-reuse guard)
        let prov = make_prov(DEAD_PID, RECENT_TS);
        let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
        assert!(
            verdict.is_none(),
            "recent ts should suppress WriterDead, got {verdict:?}"
        );
    }

    #[test]
    fn older_than_consumer_flagged() {
        // AC4: file ts < consumer mtime → OlderThanConsumer
        let consumer_mtime = OLD_TS + 3600; // consumer was built after the file
        let prov = make_prov(std::process::id(), OLD_TS); // live pid, so no WriterDead
        // Ensure live-pid old-ts does NOT cause WriterDead on its own:
        let verdict = judge_provenance(&prov, NOW, Some(consumer_mtime), MIN_AGE);
        assert!(
            matches!(verdict, Some((StaleReason::OlderThanConsumer, _))),
            "expected OlderThanConsumer, got {verdict:?}"
        );
    }

    #[test]
    fn newer_than_consumer_not_flagged() {
        // AC4 (negative): file ts >= consumer mtime → not flagged
        let consumer_mtime = OLD_TS - 100; // consumer is *older* than the file
        let prov = make_prov(std::process::id(), OLD_TS);
        let verdict = judge_provenance(&prov, NOW, Some(consumer_mtime), MIN_AGE);
        assert!(
            verdict.is_none(),
            "file newer than consumer should not be flagged, got {verdict:?}"
        );
    }

    #[test]
    fn both_conditions_reported_once_as_both() {
        // AC5: dead pid AND older than consumer → Both (not two entries)
        let consumer_mtime = OLD_TS + 1000;
        let prov = make_prov(DEAD_PID, OLD_TS);
        let verdict = judge_provenance(&prov, NOW, Some(consumer_mtime), MIN_AGE);
        assert!(
            matches!(verdict, Some((StaleReason::Both, _))),
            "expected Both, got {verdict:?}"
        );
    }

    #[test]
    fn unstamped_not_flagged() {
        // Unstamped provenance must be ignored entirely.
        let prov = parse("", None);
        assert_eq!(prov.form, Form::Unstamped);
        let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
        assert!(verdict.is_none(), "unstamped must not be flagged");
    }

    #[test]
    fn no_ts_not_flagged() {
        // A provenance with no ts field cannot be judged.
        let prov = parse("comm-chain:bash>zsh;cwd:/x;pid:9999999;uid:1000", None);
        let verdict = judge_provenance(&prov, NOW, None, MIN_AGE);
        assert!(verdict.is_none(), "no-ts must not be flagged");
    }
}
