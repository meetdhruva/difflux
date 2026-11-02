## What and why

<!-- What does this change, and what problem does it solve? Link the issue: "Fixes #123". -->

## How it was tested

<!-- Tests added or updated; manual checks on which OS. -->

## Checklist

- [ ] `cargo xtask ci` passes
- [ ] Tests cover the change (UI changes: an `egui_kittest` test by accessible label)
- [ ] `CHANGELOG.md` has an entry under `[Unreleased]`
- [ ] `--json` output changed? Ran `cargo xtask schema`; breaking changes bump `SCHEMA_ID`
- [ ] No new way to modify user files without preview, confirmation and undo
