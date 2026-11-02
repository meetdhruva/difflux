# 0006 — Accessibility and color

Date: 2026-11-02 · Status: accepted

- Every custom-painted row exposes an AccessKit node with a full sentence
  ("Changed. Left line 12: … Right line 12: …").
- Diff colors use the Okabe–Ito palette (vermillion = left/removed,
  blue = right/added, amber = changed). Each state also has a glyph, so color is
  never the only signal.
- Text on every tint meets WCAG AA (4.5:1); a unit test enforces it for both
  themes.
- Everything is reachable from the keyboard; the command palette lists every
  action with its shortcut. System light/dark theme is followed by default.
