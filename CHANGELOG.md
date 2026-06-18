# Changelog

## v0.4.0 — 2026-06-18

colophon attribute: walk a tree, attribute files to writing actor via provfs xattrs; ranked table + json; --by skill|actor|cwd; skipped/unstamped buckets

## v0.3.0 — 2026-06-18

## colophon-attribute

Adds `colophon attribute <dir>` — walks a directory tree, reads each file's
`user.prov.session` xattr via provfs, and emits a ranked report grouping files
and bytes by the skill/session that wrote them.

- **`Attribution` model**: actor buckets (key, file_count, total_bytes,
  oldest/newest ts, sample paths), `skipped` count (target/.git/node_modules),
  `unstamped` count (non-skipped files with no xattr).
- **`fn attribute(root, opts) -> Attribution`**: bounded walk (no symlink-follow),
  skip-prefix detection matching provfs_lsm.c, I/O errors → unstamped.
- **CLI**: `colophon attribute <dir> --format text|json --top N --min-bytes B
  --by skill|actor|cwd`
- All ACs pass: three-bucket tempdir fixture (AC2), target/ → skipped (AC3),
  no-xattr → unstamped (AC4), --by skill grouping + --top N (AC5), read-only
  invariant (AC6), live autobuilder integration (AC7).

## v0.2.0 — 2026-06-18

## colophon-stale — provenance-grounded orphaned-state detector

Added `colophon stale <dir>`: walks a directory tree, reads each file's
`user.prov.session` + `user.prov.ts` xattrs, and flags stale files:

- **WriterDead**: stamped pid absent from `/proc`, file older than `--min-age` (default 1h)
- **OlderThanConsumer**: `prov.ts` < mtime of `--consumer <binary>`
- **Both**: both conditions apply

New public API: `stale()`, `judge_provenance()`, `StaleReason`, `Staleness`, `StaleOpts`.
CLI: `--format text|json`, `--reason writer-dead|older|any`, read-only, SIGPIPE-safe.
