//! Command palette (Ctrl+Shift+P / F1) and keyboard shortcut sheet.

use difflux_app::Command;
use difflux_app::commands::fuzzy_score;
use egui::{Context, Id, Key, Modal, RichText, ScrollArea, TextEdit};

use crate::keymap;

#[derive(Debug, Default)]
pub(crate) struct PaletteState {
    pub open: bool,
    query: String,
    selected: usize,
    just_opened: bool,
}

impl PaletteState {
    pub(crate) fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.just_opened = true;
    }

    /// Shows the palette; returns the chosen command.
    pub(crate) fn show(&mut self, ctx: &Context, available: impl Fn(Command) -> bool) -> Option<Command> {
        if !self.open {
            return None;
        }
        let mut matches: Vec<(u32, Command)> = Command::ALL
            .iter()
            .copied()
            .filter(|&c| c != Command::CommandPalette && available(c))
            .filter_map(|c| fuzzy_score(&self.query, c.title()).map(|s| (s, c)))
            .collect();
        matches.sort_by_key(|m| std::cmp::Reverse(m.0));
        self.selected = self.selected.min(matches.len().saturating_sub(1));

        let (up, down, enter, escape) = ctx
            .input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if down {
            self.selected = (self.selected + 1).min(matches.len().saturating_sub(1));
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }

        let mut chosen = None;
        let modal = Modal::new(Id::unique("command-palette")).show(ctx, |ui| {
            ui.set_width(460.0);
            let edit = ui.add(
                TextEdit::singleline(&mut self.query)
                    .hint_text("Type a command…")
                    .desired_width(f32::INFINITY)
                    .id(Id::unique("palette-query")),
            );
            if self.just_opened {
                edit.request_focus();
                self.just_opened = false;
            }
            ui.add_space(6.0);
            ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (i, &(_, cmd)) in matches.iter().enumerate() {
                    let selected = i == self.selected;
                    let shortcut = keymap::label(ctx, cmd).unwrap_or_default();
                    let resp = ui
                        .horizontal(|ui| {
                            let r = ui.selectable_label(selected, cmd.title());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(shortcut).weak().monospace());
                            });
                            r
                        })
                        .inner;
                    if selected {
                        resp.scroll_to_me(None);
                    }
                    if resp.clicked() {
                        chosen = Some(cmd);
                    }
                }
                if matches.is_empty() {
                    ui.label(RichText::new("No matching command").weak());
                }
            });
        });
        if enter && chosen.is_none() {
            chosen = matches.get(self.selected).map(|&(_, c)| c);
        }
        if chosen.is_some() || escape || modal.should_close() {
            self.open = false;
        }
        chosen
    }
}

/// Window listing every shortcut.
pub(crate) fn shortcuts_window(ctx: &Context, open: &mut bool) {
    let mut still_open = *open;
    egui::Window::new("Keyboard shortcuts").open(&mut still_open).collapsible(false).resizable(true).default_width(420.0).show(ctx, |ui| {
        ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
            egui::Grid::new("shortcuts").num_columns(2).striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
                for &cmd in Command::ALL {
                    let keys = keymap::labels(ctx, cmd);
                    if keys.is_empty() {
                        continue;
                    }
                    ui.label(cmd.title());
                    ui.label(RichText::new(keys.join("   ")).monospace());
                    ui.end_row();
                }
                for (keys, what) in [
                    ("↑ ↓ PgUp PgDn Home End", "Move / scroll"),
                    ("← →", "Collapse / expand folder, scroll text sideways"),
                    ("Space", "Toggle folder or open file"),
                    ("Ctrl + / Ctrl −", "Zoom"),
                ] {
                    ui.label(what);
                    ui.label(RichText::new(keys).monospace());
                    ui.end_row();
                }
            });
        });
    });
    *open = still_open;
}
