//! AC2 (MUST): parse() decodes the live sample correctly.
//!
//! Input: `comm-chain:cat>zsh>claude;cwd:/home/jsy/wintermute/autobuilder;pid:2662703;uid:1000`
//! Expected: comm_chain==["cat","zsh","claude"], cwd==Some("..."), pid==Some(2662703), uid==Some(1000), Form::CommChain
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown, clippy::doc_link_with_quotes)]

use colophon::{parse, Form};

#[test]
fn acceptance_ac2_parse_live_sample() {
    let session =
        "comm-chain:cat>zsh>claude;cwd:/home/jsy/wintermute/autobuilder;pid:2662703;uid:1000";
    let p = parse(session, None);

    assert_eq!(p.form, Form::CommChain, "form should be CommChain");
    assert_eq!(
        p.comm_chain,
        vec!["cat".to_owned(), "zsh".to_owned(), "claude".to_owned()],
        "comm_chain should be [cat, zsh, claude]"
    );
    assert_eq!(
        p.cwd.as_deref(),
        Some("/home/jsy/wintermute/autobuilder"),
        "cwd mismatch"
    );
    assert_eq!(p.pid, Some(2_662_703), "pid mismatch");
    assert_eq!(p.uid, Some(1000), "uid mismatch");
    assert_eq!(p.raw, session, "raw should preserve original input");
}
