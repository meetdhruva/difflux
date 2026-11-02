# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The `--json` output has its own version (`difflux/1`); see ADR 0005.

## [Unreleased]

## [0.1.1] - 2026-11-02

### Added
- App icon on every platform: window and taskbar icon, Windows `.exe` icon and
  version details, macOS `.app` icon, Linux desktop entry and hicolor icons.
- Debian/Ubuntu `.deb` packages for x64 and Arm64.
- Opening a file that exists on only one side shows its whole content as
  added or removed, instead of only a notice.
- Crash reports: a panic writes a report with version, OS and backtrace to the
  platform log folder and points to the issue tracker.
- `cargo xtask icons` regenerates every icon format from the SVG source.

### Fixed
- Very long single lines (minified files, logs) no longer slow down
  rendering; display and screen-reader text are capped per line.
- Home screen shortcut list is centered, and "Previous difference" shows
  `Shift+F8` alongside `F8`.
- Release smoke test no longer fails on the expected exit code 1.

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

[Unreleased]: https://github.com/meetdhruva/difflux/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/meetdhruva/difflux/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/meetdhruva/difflux/releases/tag/v0.1.0
