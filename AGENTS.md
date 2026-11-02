# AGENTS.md

Guide for AI coding agents (and humans in a hurry) working on difflux.

## What this is

difflux is a native Rust file and folder compare tool, a reliability-first
replacement for Beyond Compare. One binary: desktop app (egui), CLI with JSON
output, and an MCP server.

## Commands

| Task | Command |
|---|---|
| Everything CI runs | `cargo xtask ci` |
| Fast engine-only loop (no GUI build) | `cargo test -p difflux-diff-text -p difflux-diff-dir -p difflux-app -p difflux-cli` |
| Headless binary | `cargo build -p difflux --no-default-features` |
| Run the app | `cargo run -- LEFT RIGHT` |
| Compare in the terminal | `cargo run -- diff LEFT RIGHT [--json]` |
| Performance budgets | `cargo xtask perf` (release build) |
| Regenerate JSON Schema | `cargo xtask schema` |
| Regenerate app icons from `assets/icon/difflux.svg` | `cargo xtask icons` |
| Render UI screenshots to `target/screenshots/` | `cargo test -p difflux-gui --test screenshots -- --ignored` (needs a GPU or lavapipe) |

## Layout

```
crates/
  difflux-core       shared types, options, cancellation. No I/O.
  difflux-text       decoding, encoding detection, line index, normalization. No I/O.
  difflux-diff-text  line diff (Myers, Histogram when affordable) → aligned rows; inline word diff.
  difflux-diff-dir   parallel folder compare, tiered classification, arena tree.
  difflux-app        UI-agnostic layer: loading, jobs, view-models, Command list.
  difflux-cli        `diff`, `schema`, `mcp` subcommands; JSON output types.
  difflux-gui        egui desktop app. Only crate that knows about egui.
  difflux            the binary. `gui` feature (default) pulls in difflux-gui.
xtask/               cargo xtask ci | perf | schema | icons
assets/              icon source (SVG) and generated PNG/ICO/ICNS, Linux desktop entry
.github/workflows/    ci.yml (3-OS checks), release.yml (tag vX.Y.Z → all platform builds)
docs/adr/            architecture decisions; read before changing structure
docs/schema/         generated JSON Schema for --json output (checked in CI)
```

Dependency direction is strictly downward in that list. Engines must never
depend on `difflux-app`, `difflux-cli` or `difflux-gui`.

## Rules

- **Never corrupt user files.** Nothing writes to user files yet. When that
  arrives, every write is atomic (temp file + fsync + rename), previewed, and
  undoable; see ADR 0004.
- **Engines are pure.** No I/O in `difflux-core`, `difflux-text`,
  `difflux-diff-text`. File reading lives in `difflux-app` and `difflux-diff-dir`.
- **No `unwrap()`/`expect()` in library code.** Clippy warns; CI denies warnings.
  Tests may use `expect` with a reason.
- **`unsafe` is forbidden** workspace-wide.
- **The UI thread never touches the disk.** Use `difflux_app::Job`.
- **Every user action is a `Command`** (`crates/difflux-app/src/commands.rs`).
  Add the command there, then bind keys in `crates/difflux-gui/src/keymap.rs`.
  The palette, menus and shortcut sheet pick it up automatically. Command ids
  are a stable interface; never rename one.
- **JSON output is versioned.** Adding optional fields is fine. Renaming or
  removing fields, or changing meanings, requires bumping `SCHEMA_ID` in
  `difflux-core`. Run `cargo xtask schema` after any change to output types.
- **Accessibility is not optional.** Custom-painted widgets must call
  `response.widget_info(...)` with a meaningful label. GUI tests find widgets
  through the accessibility tree, so an unlabeled widget is an untestable one.
- **Color never carries meaning alone.** Diff states have glyphs (`− + ~`,
  `≠ ← → ⇄ !`). New colors go in `theme.rs` and must pass the contrast test.
- **Exit codes** (CLI): 0 same, 1 different, 2 trouble. Do not change.

## Testing conventions

- Engine invariants are property-tested (proptest). If you change alignment
  or diffing, the properties in `difflux-diff-text` must still hold.
- Folder tests build real trees in `tempfile` directories.
- GUI tests use `egui_kittest` and query by accessible label. The harness
  panics on glyphs no bundled font can draw, so tests also catch tofu (□) icons:
  pick glyphs that render in the font family you paint with.
- Performance budgets live in `crates/difflux/tests/perf_budget.rs` and only
  run in release (`cargo xtask perf`).
