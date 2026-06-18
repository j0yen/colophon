# Changelog

## v0.2.0 — 2026-06-18

## colophon-stale — provenance-grounded orphaned-state detector

Added `colophon stale <dir>`: walks a directory tree, reads each file's
`user.prov.session` + `user.prov.ts` xattrs, and flags stale files:

- **WriterDead**: stamped pid absent from `/proc`, file older than `--min-age` (default 1h)
- **OlderThanConsumer**: `prov.ts` < mtime of `--consumer <binary>`
- **Both**: both conditions apply

New public API: `stale()`, `judge_provenance()`, `StaleReason`, `Staleness`, `StaleOpts`.
CLI: `--format text|json`, `--reason writer-dead|older|any`, read-only, SIGPIPE-safe.
