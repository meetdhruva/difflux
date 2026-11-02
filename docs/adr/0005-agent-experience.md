# 0005 — Agent experience (AX) is a first-class interface

Date: 2026-11-02 · Status: accepted

## Context
AI agents increasingly drive developer tools. Scraping human output is
brittle; agents need structured, versioned, bounded results.

## Decision
- Every comparison is available as JSON (`--json`) with a `schema` field
  (`difflux/1`) and a published JSON Schema (`difflux schema`, checked into
  `docs/schema`). Additive changes keep the version; breaking ones bump it.
- Errors are JSON too (`kind: "error"`) and exit code 2.
- Outputs are bounded (`--max-lines`, `--max-entries`) and say when truncated.
- Folder verdicts include `basis` so an agent knows whether bytes were read.
- `difflux mcp` exposes read-only `compare_paths` and `compare_text` tools
  over stdio (JSON-RPC 2.0), returning the same reports as structured content.
- Every UI action is a `Command` with a stable id, ready for scripting.
- `AGENTS.md` documents the codebase for coding agents.
