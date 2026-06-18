//! Property-based tests for parse() invariants.
//! READ-ONLY: do not modify in Stage 3 iterations.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use colophon::parse;
use proptest::prelude::*;

proptest! {
    #[test]
    fn parse_never_panics(s in ".*") {
        let _ = parse(&s, None);
    }

    #[test]
    fn parse_raw_always_preserved(s in ".*") {
        let p = parse(&s, None);
        prop_assert_eq!(&p.raw, &s);
    }

    #[test]
    fn parse_comm_chain_links_are_non_empty(s in "comm-chain:[a-z>]{1,50}(;.*)?") {
        let p = parse(&s, None);
        for link in &p.comm_chain {
            prop_assert!(!link.is_empty(), "comm_chain link should not be empty");
        }
    }

    #[test]
    fn parse_ts_round_trips(n in 0u64..u64::MAX) {
        let ts_str = n.to_string();
        let p = parse("comm-chain:sh;pid:1;uid:0", Some(&ts_str));
        prop_assert_eq!(p.ts, Some(n));
    }
}
