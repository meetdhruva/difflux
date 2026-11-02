# 0002 — egui for the desktop UI

Date: 2026-11-02 · Status: accepted

## Context
Diff panes, overview rulers and folder trees are custom-painted in any toolkit.
We need: fast iteration, virtualization of millions of rows, a screen-reader
tree, Windows/macOS/Linux, and a permissive license.

## Options
egui (immediate mode, AccessKit built in), iced (Elm architecture, weaker
accessibility today), GPUI (editor-grade, pre-1.0 with frequent breaking
changes), Slint (declarative; GPL or commercial for closed source), Tauri
(web UI; DOM virtualization fights million-line diffs).

## Decision
egui via eframe (wgpu renderer). Views are custom-painted and virtualized with
`ScrollArea::show_rows`. All state that matters lives in `difflux-app`
view-models, so the UI layer stays thin and replaceable.

## Consequences
Accessibility comes from AccessKit; custom rows must set `widget_info`.
Tests drive the real UI through `egui_kittest` using the accessibility tree.
The app repaints only on input or job completion, so it idles at 0% CPU.
