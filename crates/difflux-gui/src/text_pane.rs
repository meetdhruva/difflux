//! Side-by-side text comparison: virtualized rows, synchronized scrolling,
//! word-level highlights and an overview ruler.

use std::path::PathBuf;

use difflux_app::{Command, TextView};
use difflux_core::Side;
use difflux_diff_text::{InlineDiff, RowKind};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Key, Rect, Role, ScrollArea, Sense, Stroke, TextStyle, Ui, WidgetInfo, pos2, vec2};

use crate::format;
use crate::theme::Palette;

const RULER_WIDTH: f32 = 14.0;
const DIVIDER: f32 = 6.0;
const PAD: f32 = 6.0;

/// A text comparison on the view stack.
#[derive(Debug)]
pub(crate) struct TextPane {
    pub view: TextView,
    pub left: PathBuf,
    pub right: PathBuf,
    /// Row to bring into view on the next frame.
    pub pending_row: Option<usize>,
    /// Absolute scroll offset to apply on the next frame (used to keep position on refresh).
    pub pending_offset: Option<f32>,
    offset_y: f32,
    viewport_h: f32,
    h_offset: f32,
    row_h: f32,
    max_chars: usize,
}

impl TextPane {
    pub(crate) fn new(view: TextView, left: PathBuf, right: PathBuf) -> Self {
        let max_chars = [Side::Left, Side::Right]
            .iter()
            .filter_map(|&s| view.document(s))
            .flat_map(|d| d.lines().map(|l| l.chars().count() + 3 * l.matches('\t').count()))
            .max()
            .unwrap_or(0);
        Self {
            view,
            left,
            right,
            pending_row: None,
            pending_offset: None,
            offset_y: 0.0,
            viewport_h: 0.0,
            h_offset: 0.0,
            row_h: 18.0,
            max_chars,
        }
    }

    pub(crate) fn scroll_offset(&self) -> f32 {
        self.offset_y
    }

    /// Row used as the starting point for "next/previous difference".
    pub(crate) fn nav_anchor(&self) -> Option<usize> {
        let row_h = self.row_h.max(1.0);
        let top = (self.offset_y / row_h).floor() as usize;
        let bottom = ((self.offset_y + self.viewport_h) / row_h).ceil() as usize;
        let current_visible = self
            .view
            .current_hunk
            .and_then(|h| self.view.compare.diff.as_ref().map(|d| d.hunks[h].rows.start as usize))
            .is_some_and(|r| (top..bottom).contains(&r));
        if current_visible { None } else { Some(top.saturating_sub(1)) }
    }
}

pub(crate) fn row_height(ui: &Ui) -> f32 {
    (ui.text_style_height(&TextStyle::Monospace) + 4.0).round()
}

pub(crate) fn show(ui: &mut Ui, pane: &mut TextPane, pal: &Palette, commands: &mut Vec<Command>) {
    let font = TextStyle::Monospace.resolve(ui.style());
    let row_h = row_height(ui);
    pane.row_h = row_h;
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    let line_digits = [Side::Left, Side::Right]
        .iter()
        .filter_map(|&s| pane.view.document(s).map(difflux_text::Document::line_count))
        .max()
        .map_or(1, format::digits);
    let gutter_w = (line_digits as f32 + 2.5) * char_w + PAD;

    if pane.view.compare.diff.is_none() {
        binary_notice(ui, pane, pal);
        return;
    }

    header(ui, pane, pal, gutter_w);
    keyboard_scroll(ui, pane, row_h, char_w);

    let total = pane.view.rows().len();
    let full = ui.available_rect_before_wrap();
    let rows_rect = Rect::from_min_max(full.min, pos2(full.max.x - RULER_WIDTH - 2.0, full.max.y));
    let ruler_rect = Rect::from_min_max(pos2(full.max.x - RULER_WIDTH, full.min.y), full.max);

    // Horizontal scroll (Shift+wheel or trackpad) is shared by both sides.
    if ui.rect_contains_pointer(rows_rect) {
        let dx = ui.input(|i| i.smooth_scroll_delta.x);
        if dx != 0.0 {
            pane.h_offset -= dx;
        }
    }
    let text_w = (rows_rect.width() - DIVIDER) / 2.0 - gutter_w - PAD;
    let max_h = (pane.max_chars as f32 * char_w - text_w + 4.0 * char_w).max(0.0);
    pane.h_offset = pane.h_offset.clamp(0.0, max_h);

    let mut scroll = ScrollArea::vertical().id_salt("text-rows").auto_shrink([false, false]);
    if let Some(row) = pane.pending_row.take() {
        // Put the target a third of the way down, so context above stays visible.
        let y = (row as f32 * row_h - pane.viewport_h / 3.0).max(0.0);
        scroll = scroll.vertical_scroll_offset(y);
    } else if let Some(y) = pane.pending_offset.take() {
        scroll = scroll.vertical_scroll_offset(y);
    }

    let mut rows_ui = ui.new_child(egui::UiBuilder::new().max_rect(rows_rect));
    rows_ui.spacing_mut().item_spacing.y = 0.0;
    let out = scroll.show_rows(&mut rows_ui, row_h, total, |ui, range| {
        for row in range {
            paint_row(ui, pane, pal, row, row_h, gutter_w, &font, commands);
        }
    });
    pane.offset_y = out.state.clamped_offset().y;
    pane.viewport_h = out.inner_rect.height();

    ruler(ui, pane, pal, ruler_rect, row_h);
    ui.advance_cursor_after_rect(full);
}

fn header(ui: &mut Ui, pane: &TextPane, pal: &Palette, gutter_w: f32) {
    let h = ui.text_style_height(&TextStyle::Small) + 8.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let painter = ui.painter_at(rect);
    let half = (rect.width() - RULER_WIDTH - 2.0 - DIVIDER) / 2.0;
    let small = TextStyle::Small.resolve(ui.style());
    for (i, side) in [Side::Left, Side::Right].into_iter().enumerate() {
        let x = rect.min.x + i as f32 * (half + DIVIDER) + gutter_w;
        let doc = pane.view.document(side);
        let info = doc.map_or_else(String::new, |d| {
            let bom = if d.has_bom { " BOM" } else { "" };
            let lossy = if d.lossy { " · decoding errors" } else { "" };
            format!("{}{bom} · {} · {} lines{lossy}", d.encoding, d.line_ending.label(), format::thousands(d.line_count() as u64))
        });
        painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, info, small.clone(), pal.muted_fg);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &mut Ui,
    pane: &mut TextPane,
    pal: &Palette,
    row: usize,
    row_h: f32,
    gutter_w: f32,
    font: &FontId,
    commands: &mut Vec<Command>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    let Some(kind) = pane.view.rows().get(row).map(|r| r.kind) else { return };
    let inline = if kind == RowKind::Modify { pane.view.inline(row).clone() } else { InlineDiff::default() };
    let in_hunk = pane.view.in_current_hunk(row);
    let painter = ui.painter_at(rect);
    let half = (rect.width() - DIVIDER) / 2.0;

    for (i, side) in [Side::Left, Side::Right].into_iter().enumerate() {
        let x0 = rect.min.x + i as f32 * (half + DIVIDER);
        let cell = Rect::from_min_size(pos2(x0, rect.min.y), vec2(half, row_h));
        let gutter = Rect::from_min_size(cell.min, vec2(gutter_w, row_h));
        let text_rect = Rect::from_min_max(pos2(gutter.max.x, cell.min.y), cell.max);
        let content = pane.view.cell(row, side);

        let (bg, strong, marker, marker_color) = match (kind, side, content.is_some()) {
            (RowKind::Equal, ..) => (None, pal.changed_bg, "", pal.muted_fg),
            (_, _, false) => (Some(pal.filler_bg), pal.filler_bg, "", pal.muted_fg),
            (RowKind::Delete, ..) => (Some(pal.removed_bg), pal.removed_strong, "−", pal.removed_fg),
            (RowKind::Insert, ..) => (Some(pal.added_bg), pal.added_strong, "+", pal.added_fg),
            (RowKind::Modify, Side::Left, _) => (Some(pal.changed_bg), pal.removed_strong, "~", pal.changed_fg),
            (RowKind::Modify, Side::Right, _) => (Some(pal.changed_bg), pal.added_strong, "~", pal.changed_fg),
        };
        painter.rect_filled(gutter, 0.0, pal.gutter_bg);
        if let Some(bg) = bg {
            painter.rect_filled(text_rect, 0.0, bg);
        }
        if let Some((idx, text)) = content {
            let number_x = gutter.max.x - PAD - font.size * 1.2;
            painter.text(pos2(number_x, rect.center().y), Align2::RIGHT_CENTER, idx.to_string(), font.clone(), pal.muted_fg);
            painter.text(pos2(gutter.max.x - PAD * 0.5, rect.center().y), Align2::RIGHT_CENTER, marker, font.clone(), marker_color);

            let spans = match side {
                Side::Left => &inline.left,
                Side::Right => &inline.right,
            };
            let job = line_job(text, spans, font, pal.text_fg, strong);
            let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
            let pos = pos2(text_rect.min.x + PAD - pane.h_offset, rect.center().y - galley.size().y / 2.0);
            painter.with_clip_rect(text_rect).galley(pos, galley, pal.text_fg);
        }
    }
    if in_hunk {
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, row_h)), 0.0, pal.current_hunk);
    }
    let mid = rect.min.x + half + DIVIDER / 2.0;
    painter.line_segment([pos2(mid, rect.min.y), pos2(mid, rect.max.y)], Stroke::new(1.0, pal.gutter_bg));

    response.widget_info(|| WidgetInfo::labeled(Role::Row, true, pane.view.row_label(row)));
    if response.clicked()
        && let Some(diff) = pane.view.compare.diff.as_ref()
        && let Some(h) = diff.hunk_at_row(row as u32)
    {
        pane.view.current_hunk = Some(h);
    }
    if response.double_clicked() && kind == RowKind::Equal {
        commands.push(Command::NextDifference);
    }
}

/// Lays out one line, highlighting changed byte ranges.
fn line_job(text: &str, spans: &[std::ops::Range<usize>], font: &FontId, color: Color32, highlight: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let plain = TextFormat::simple(font.clone(), color);
    let marked = TextFormat { background: highlight, ..TextFormat::simple(font.clone(), color) };
    let mut pos = 0;
    for span in spans {
        let (start, end) = (span.start.min(text.len()), span.end.min(text.len()));
        if start > pos {
            job.append(&format::expand_tabs(&text[pos..start]), 0.0, plain.clone());
        }
        if end > start {
            job.append(&format::expand_tabs(&text[start..end]), 0.0, marked.clone());
        }
        pos = end.max(pos);
    }
    if pos < text.len() || text.is_empty() {
        job.append(&format::expand_tabs(&text[pos..]), 0.0, plain);
    }
    job
}

/// Overview of all differences; click or drag to jump.
fn ruler(ui: &mut Ui, pane: &mut TextPane, pal: &Palette, rect: Rect, row_h: f32) {
    let response = ui.interact(rect, ui.scope_id().with("ruler"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, pal.ruler_bg);
    let total = pane.view.rows().len().max(1) as f32;
    let y_of = |row: f32| rect.min.y + rect.height() * (row / total);
    if let Some(diff) = &pane.view.compare.diff {
        for (i, h) in diff.hunks.iter().enumerate() {
            let rows = &diff.rows[h.rows.start as usize..h.rows.end as usize];
            let color = if pane.view.current_hunk == Some(i) {
                pal.current_hunk
            } else if rows.iter().all(|r| r.kind == RowKind::Delete) {
                pal.removed_fg
            } else if rows.iter().all(|r| r.kind == RowKind::Insert) {
                pal.added_fg
            } else {
                pal.changed_fg
            };
            let y0 = y_of(h.rows.start as f32);
            let y1 = y_of(h.rows.end as f32).max(y0 + 2.0);
            painter.rect_filled(Rect::from_x_y_ranges(rect.x_range().shrink(2.0), y0..=y1), 1.0, color);
        }
    }
    let view_top = y_of(pane.offset_y / row_h);
    let view_bottom = y_of((pane.offset_y + pane.viewport_h) / row_h).max(view_top + 4.0);
    painter.rect_filled(Rect::from_x_y_ranges(rect.x_range(), view_top..=view_bottom), 2.0, pal.ruler_view);

    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        let row = ((p.y - rect.min.y) / rect.height() * total).clamp(0.0, total - 1.0);
        pane.pending_offset = Some((row * row_h - pane.viewport_h / 2.0).max(0.0));
    }
    response.on_hover_text("Overview of differences — click to jump");
}

fn keyboard_scroll(ui: &Ui, pane: &mut TextPane, row_h: f32, char_w: f32) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let page_h = (pane.viewport_h - row_h * 2.0).max(row_h);
    let total_h = pane.view.rows().len() as f32 * row_h;
    let delta = ui.input(|i| {
        let plain = i.modifiers.is_none();
        let ctrl = i.modifiers.command_only();
        if plain && i.key_pressed(Key::ArrowDown) {
            Some(row_h)
        } else if plain && i.key_pressed(Key::ArrowUp) {
            Some(-row_h)
        } else if i.key_pressed(Key::PageDown) {
            Some(page_h)
        } else if i.key_pressed(Key::PageUp) {
            Some(-page_h)
        } else if ctrl && i.key_pressed(Key::Home) || plain && i.key_pressed(Key::Home) {
            Some(-total_h)
        } else if ctrl && i.key_pressed(Key::End) || plain && i.key_pressed(Key::End) {
            Some(total_h)
        } else {
            None
        }
    });
    if let Some(d) = delta {
        pane.pending_offset = Some((pane.offset_y + d).clamp(0.0, (total_h - pane.viewport_h).max(0.0)));
    }
    let dx = ui.input(|i| {
        let plain = i.modifiers.is_none();
        if plain && i.key_pressed(Key::ArrowRight) {
            4.0 * char_w
        } else if plain && i.key_pressed(Key::ArrowLeft) {
            -4.0 * char_w
        } else {
            0.0
        }
    });
    pane.h_offset += dx;
}

fn binary_notice(ui: &mut Ui, pane: &TextPane, pal: &Palette) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.heading(pane.view.compare.verdict.label());
        ui.add_space(8.0);
        for side in [&pane.view.compare.left, &pane.view.compare.right] {
            ui.label(egui::RichText::new(format!("{} — {}", side.name, format::size(side.byte_len()))).color(pal.muted_fg));
        }
        ui.add_space(8.0);
        ui.label("A hex view for binary files is on the roadmap.");
    });
}
