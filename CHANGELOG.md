# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The `--json` output has its own version (`difflux/1`); see ADR 0005.

## [Unreleased]

## [0.1.0] - 2026-11-02

### Added
- Folder compare: parallel scan, name matching that handles case-insensitive
  filesystems and Unicode normalization, tiered verdicts (size → size + time →
  bytes) with the basis of each verdict, per-entry errors.
- Text compare: adaptive line diff (Myers, Histogram when affordable),
  similarity-based line pairing, word-level highlights, ignore rules for
  whitespace, case and line endings, encoding and BOM detection.
- Desktop app (egui): side-by-side folder tree and text views, overview ruler,
  command palette, keyboard navigation, drag and drop, recent comparisons,
  light and dark themes, screen-reader labels on every row.
- `difflux-app` launcher that opens without a console window on Windows.
- CLI: `difflux diff` with `diff(1)` exit codes, unified diff and
  `--json` output with a published JSON Schema (`difflux schema`).
- MCP server (`difflux mcp`) with read-only `compare_paths` and
  `compare_text` tools for AI agents.

[Unreleased]: https://github.com/meetdhruva/difflux/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/meetdhruva/difflux/releases/tag/v0.1.0
