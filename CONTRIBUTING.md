# Contributing to difflux

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Before you start

- **Bugs:** open an issue with the bug template. Include `difflux --version`,
  your OS, and the smallest pair of files or folders that reproduces it.
- **Features:** open an issue first for anything larger than a small fix, so
  we can agree on the approach before you spend time on it.
- **Security issues:** do not open a public issue. See [SECURITY.md](SECURITY.md).

## Development setup

You need Rust 1.97 (installed automatically by `rustup` from
`rust-toolchain.toml`).

```sh
git clone https://github.com/meetdhruva/difflux
cd difflux
cargo run -- path/to/left path/to/right   # desktop app
cargo run -- diff a.txt b.txt              # terminal
cargo xtask ci                             # everything CI checks
```

`AGENTS.md` describes the crate layout and the rules the codebase follows.
The decisions behind the design are in [`docs/adr`](docs/adr).

## Making a change

1. Fork and create a branch from `main`.
2. Keep the change focused; one topic per pull request.
3. Add or update tests. Engine changes need unit or property tests; UI
   changes need an `egui_kittest` test that finds widgets by accessible label.
4. Run `cargo xtask ci`. It must pass: formatting, clippy with warnings
   denied, tests, schema drift and `cargo deny`.
5. If you change `--json` output types, run `cargo xtask schema` and commit
   the updated schema. Breaking changes need a new `SCHEMA_ID`.
6. Add a line under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md).
7. Open the pull request and fill in the template.

### Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/):
`feat: …`, `fix: …`, `perf: …`, `docs: …`, `test: …`, `ci: …`, `chore: …`.

### Rules worth repeating

- Never corrupt user files: writes are atomic, previewed and undoable (ADR 0004).
- No `unwrap()`/`expect()` in library code; `unsafe` is forbidden.
- The UI thread never touches the disk.
- Every user action is a `Command`; never rename a command id.
- Color never carries meaning alone, and new colors must pass the contrast test.

## Releasing (maintainers)

1. Move the `[Unreleased]` notes in `CHANGELOG.md` under a new version heading.
2. Bump `version` in the root `Cargo.toml` and run `cargo check` to update `Cargo.lock`.
3. Commit, then tag: `git tag -a vX.Y.Z -m "difflux vX.Y.Z" && git push --follow-tags`.
4. The `release` workflow builds every platform, attaches checksums and
   build provenance, and publishes the GitHub release with the changelog section.

## License

By contributing, you agree that your contributions are dual-licensed under
the MIT and Apache-2.0 licenses, as described in the README, without any
additional terms or conditions.
