//! AC5 (MUST): originating_skill() matches real-captured comm-chain fixtures.
//!
//! Fixtures from actual xattrs observed on this machine:
//! - "Bun Pool 3>claude-dream-helio…" → Dream
//! - "bash>claude-build-main" → Build
//! - "zsh>claude-self-review" → SelfReview
//! - "bash>zsh>xterm" → None
//! - plain "claude" link → Claude
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::{parse, Skill};

#[test]
fn acceptance_ac5_dream() {
    // Real xattr sample from a /dream pass.
    let p = parse(
        "comm-chain:Bun Pool 3>claude-dream-helio;cwd:/home/jsy/wintermute/autobuilder;pid:9999;uid:1000",
        None,
    );
    assert_eq!(
        p.originating_skill(),
        Some(Skill::Dream),
        "claude-dream-* should map to Dream"
    );
}

#[test]
fn acceptance_ac5_build() {
    let p = parse(
        "comm-chain:bash>claude-build-main;cwd:/home/jsy;pid:42;uid:1000",
        None,
    );
    assert_eq!(
        p.originating_skill(),
        Some(Skill::Build),
        "claude-build-* should map to Build"
    );
}

#[test]
fn acceptance_ac5_self_review() {
    let p = parse(
        "comm-chain:zsh>claude-self-review-daily;cwd:/home/jsy;pid:7;uid:1000",
        None,
    );
    assert_eq!(
        p.originating_skill(),
        Some(Skill::SelfReview),
        "claude-self-* should map to SelfReview"
    );
}

#[test]
fn acceptance_ac5_none_for_no_claude() {
    let p = parse(
        "comm-chain:bash>zsh>xterm;cwd:/home/jsy;pid:1;uid:1000",
        None,
    );
    assert_eq!(
        p.originating_skill(),
        None,
        "no claude link → None"
    );
}

#[test]
fn acceptance_ac5_plain_claude_is_claude_skill() {
    let p = parse(
        "comm-chain:cat>zsh>claude;cwd:/home/jsy;pid:1;uid:1000",
        None,
    );
    assert_eq!(
        p.originating_skill(),
        Some(Skill::Claude),
        "bare 'claude' link should return Claude variant"
    );
}

#[test]
fn acceptance_ac5_dream_beats_build() {
    // If both appear in the chain, Dream wins.
    let p = parse(
        "comm-chain:claude-build-x>claude-dream-y;cwd:/x;pid:1;uid:0",
        None,
    );
    assert_eq!(p.originating_skill(), Some(Skill::Dream));
}
