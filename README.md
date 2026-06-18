# colophon

The booted `7.0.11-arch1-1-wintermute` kernel stamps a structured

## Overview

The booted `7.0.11-arch1-1-wintermute` kernel stamps a structured
`user.prov.session` xattr on every file, but no userspace tool parses the
structure. This PRD builds `colophon`, a new Rust CLI+library whose
`Provenance` model and `parse()` function are the single canonical decoder of
the enriched provfs format. It is the foundation crate the rest of the
`colophon` fleet extends.


## Acceptance


1. `cargo build` and `cargo test` are green; `cargo clippy` adds no new warnings
   over the autobuilder baseline; binary installs to `~/.local/bin/colophon`.
2. `parse()` decodes the live sample
   `comm-chain:cat>zsh>claude;cwd:/home/jsy/wintermute/autobuilder;pid:2662703;uid:1000`
   into `comm_chain == ["cat","zsh","claude"]`, `cwd == Some("/home/jsy/wintermute/autobuilder")`,
   `pid == Some(2662703)`, `uid == Some(1000)`, `Form::CommChain`.
3. `parse()` decodes an `env:`-bearing sample
   (`comm-chain:bash>claude;env:CLAUDE_TOOL=/build;cwd:/x;pid:1;uid:1000`) with
   `env["CLAUDE_TOOL"] == "/build"`.
4. A 32-hex-char non-zero value parses as `Form::AgentId` with
   `agent_session` set; an all-zero id and an empty string both parse as
   `Form::Unstamped`. No input — including malformed, truncated, or reordered —
   causes a panic (proven by a fuzz-style table of bad inputs).
5. `originating_skill()` returns `Dream` for a chain containing
   `claude-dream-he…`, `Build` for `claude-build…`, `SelfReview` for
   `claude-self…`, and `None` for a pure `bash>zsh>xterm` chain (fixtures from
   real captured xattrs).
6. `colophon parse --from-string '<value>' --format json` emits the structured
   fields; `colophon parse <real-file>` round-trips a file actually stamped by
   provfs on this machine (integration test gated on provfs presence, skipped
   with a logged note if `getfattr` reports no `user.prov.session`).
7. `colophon parse` piped to `head` does not panic (SIGPIPE reset verified).

## Install

```sh
cargo install --path .
```

## License

MIT © Joe Yen
