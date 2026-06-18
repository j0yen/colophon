//! `colophon` — canonical decoder for the wintermute kernel's provfs provenance xattrs.
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
use std::process::Command;

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
    /// Priority order: Dream > Build > SelfReview > Claude.
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
                    None | Some(Skill::Claude) | Some(Skill::SelfReview) => {
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
            } else if lower == "claude"
                || lower.starts_with("claude-")
                || is_bun_pool_claude(link)
            {
                if found.is_none() {
                    found = Some(Skill::Claude);
                }
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

    // Empty → Unstamped
    if trimmed.is_empty() {
        return Provenance {
            form: Form::Unstamped,
            comm_chain: vec![],
            env: BTreeMap::new(),
            cwd: None,
            pid: None,
            uid: None,
            agent_session: None,
            ts: ts_parsed,
            raw,
        };
    }

    // 32-hex-char value (non-zero) → AgentId form
    if is_agent_id(trimmed) {
        let is_zero = trimmed.chars().all(|c| c == '0');
        return Provenance {
            form: if is_zero {
                Form::Unstamped
            } else {
                Form::AgentId
            },
            comm_chain: vec![],
            env: BTreeMap::new(),
            cwd: None,
            pid: None,
            uid: None,
            agent_session: if is_zero {
                None
            } else {
                Some(trimmed.to_owned())
            },
            ts: ts_parsed,
            raw,
        };
    }

    // CommChain form: split on ';', decode each field
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

    // Determine form: if comm-chain was found → CommChain; else treat as
    // a best-effort decode of an unrecognised but non-empty session.
    let form = if !comm_chain.is_empty() || trimmed.starts_with("comm-chain:") {
        Form::CommChain
    } else {
        // No recognised prefix at all — still not Unstamped (value is non-empty
        // and not a hex id), but there's no comm-chain. We treat this as
        // CommChain with empty chain so the caller still gets cwd/pid/uid.
        Form::CommChain
    };

    Provenance {
        form,
        comm_chain,
        env,
        cwd,
        pid,
        uid,
        agent_session: None,
        ts: ts_parsed,
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
}
