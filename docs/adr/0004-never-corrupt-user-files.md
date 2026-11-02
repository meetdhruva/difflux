# 0004 — Never corrupt user files

Date: 2026-11-02 · Status: accepted (applies when editing/sync land)

## Decision
- Writes are atomic: temp file in the same folder → fsync → rename over the
  original (plus parent-directory fsync on Unix). Optional `.bak`.
- Sync is `plan → preview → confirm → execute`, modeled as typestate so an
  unconfirmed plan cannot run. Every executed step is journaled for resume
  or rollback after a crash.
- Before acting, re-check size and mtime; if a file changed since the compare,
  stop and re-compare.
- Deletion goes to the OS trash, never `unlink`.
- No memory-mapping of user files (truncation by another process can crash us).
