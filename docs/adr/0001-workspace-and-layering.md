# 0001 — Cargo workspace with strict layering

Date: 2026-11-02 · Status: accepted

## Context
A compare tool has a small, performance-critical core (decoding, diffing,
scanning) and several front ends (desktop, terminal, agents). Coupling the core
to any one front end makes it slow to test and hard to reuse.

## Decision
One workspace, crates layered strictly downward:
`core → text → diff-text / diff-dir → app → cli / gui → difflux (binary)`.
`core`, `text` and `diff-text` do no I/O. Only `difflux-gui` depends on egui.
The binary's `gui` feature is on by default; `--no-default-features` builds a
headless CLI + MCP binary for servers, CI and agents.

## Consequences
Engines are unit- and property-testable in milliseconds and reusable from any
front end. Adding a front end (TUI, web) means adding a crate, not refactoring.
The cost is some plumbing in `difflux-app` (view-models, jobs).
