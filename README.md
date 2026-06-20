# colophon

Reads the provenance xattr the wintermute kernel stamps on every file, and tells you who wrote what — which session, which skill, whether the writer is still alive.

## Why it exists

The `7.0.11-arch1-1-wintermute` kernel stamps a `user.prov.session` xattr on every file it writes: the comm-chain that produced the write (`cat>zsh>claude`), the cwd, pid, uid, and — when present — a 32-hex agent-namespace id. The data is there on disk, but nothing in userspace reads it, so it sits unused. A file's origin is recorded and yet unanswerable.

`colophon` is the one decoder for that format. The `parse()` function and `Provenance` model are the canonical interpretation of the stamp; the rest of the tool builds questions on top of it — who wrote this tree, what's gone stale, what's the provenance story of this directory in one block.

## Install

Requires `cargo` / `rustc` 1.85+. Installs to `~/.cargo/bin/colophon`.

```sh
git clone https://github.com/j0yen/colophon.git
cd colophon
cargo install --path . --locked
```

## Commands

```
colophon parse <file>                  decode one file's provenance stamp
colophon parse --from-string '<val>'   decode a literal xattr value
colophon attribute <dir>               walk a tree: who wrote which files
colophon stale <dir>                   flag files whose writer is dead or out of date
colophon digest                        compose attribute + stale into one block
```

Every command is read-only and resets `SIGPIPE`, so piping into `head` never panics.

### parse

```sh
$ colophon parse --from-string 'comm-chain:cat>zsh>claude;cwd:/home/jsy/wintermute;pid:2662703;uid:1000'
form:        CommChain
comm-chain:  cat > zsh > claude
skill:       Claude
cwd:         /home/jsy/wintermute
pid:         2662703
uid:         1000
raw:         comm-chain:cat>zsh>claude;cwd:/home/jsy/wintermute;pid:2662703;uid:1000
```

`--format json` emits the structured fields instead. The decoder handles three forms — `CommChain` (the enriched stamp above, with optional `env:` pairs), `AgentId` (a 32-hex agent-namespace id), and `Unstamped` (empty, all-zero, or absent). Malformed, truncated, or reordered input never panics; it falls back to `Unstamped` or a best-effort parse.

### attribute

```sh
colophon attribute <dir> --by skill   # rank actors by skill, --by actor, or --by cwd
colophon attribute <dir> --top 10 --min-bytes 4096 --format json
```

Walks a tree (no symlink-follow), reads each file's stamp, and ranks the sessions or skills that wrote it by file count and total bytes. `target/`, `.git/`, and `node_modules/` go in a `skipped` bucket; stamped-but-unreadable files go in `unstamped`.

### stale

```sh
colophon stale <dir> --min-age 3600 --consumer /path/to/binary
```

Flags files whose writing process is gone or whose work is out of date:

- **WriterDead** — the stamped pid is absent from `/proc` and the file is older than `--min-age` (the age gate guards against pid reuse).
- **OlderThanConsumer** — the file's `prov.ts` predates the mtime of `--consumer`.
- **Both** — both hold.

### digest

```sh
colophon digest --attribute <cruft-dir> --stale <state-dir> --format markdown
```

Composes `attribute` and `stale` over the directories you point it at into a single markdown (or JSON) block. When provfs isn't present, it degrades to an honest one-line note rather than inventing output.

## How it's built

A library crate (`parse`, `Provenance`, `attribute`, `stale`, `digest`) with the `colophon` binary on top. Reading xattrs requires the wintermute provfs kernel module; integration tests that need a live stamp are gated on its presence and skip with a logged note where it's absent.

## Status

`v0.5.0`. All four subcommands work; each acceptance criterion has a matching test under `tests/`. See [CHANGELOG.md](CHANGELOG.md) for the build history.

## License

MIT OR Apache-2.0, at your option.
