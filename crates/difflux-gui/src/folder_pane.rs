//! Side-by-side folder tree with keyboard navigation.

use difflux_app::{Command, FolderView};
use difflux_core::Side;
use difflux_diff_dir::{EntryKind, EntryMeta, NodeId, Status};
use egui::{Align2, FontId, Key, Rect, Role, ScrollArea, Sense, Stroke, TextStyle, Ui, WidgetInfo, pos2, vec2};

use crate::format;
use crate::theme::Palette;

const INDENT: f32 = 16.0;
const MARKER_W: f32 = 28.0;

/// A folder comparison on the view stack.
#[derive(Debug)]
pub(crate) struct FolderPane {
    pub view: FolderView,
    pub pending_row: Option<usize>,
    offset_y: f32,
    viewport_h: f32,
}

impl FolderPane {
    pub(crate) fn new(view: FolderView) -> Self {
        Self { view, pending_row: None, offset_y: 0.0, viewport_h: 0.0 }
    }
}

fn row_height(ui: &Ui) -> f32 {
    (ui.text_style_height(&TextStyle::Body) + 8.0).round()
}

pub(crate) fn show(ui: &mut Ui, pane: &mut FolderPane, pal: &Palette, tz: &jiff::tz::TimeZone, commands: &mut Vec<Command>) {
    let row_h = row_height(ui);
    keyboard(ui, pane, row_h, commands);
    header(ui, pane, pal);

    let mut scroll = ScrollArea::vertical().id_salt("folder-rows").auto_shrink([false, false]);
    if let Some(row) = pane.pending_row.take() {
        let top = row as f32 * row_h;
        // Only scroll when the row is outside the viewport.
        if top < pane.offset_y || top + row_h > pane.offset_y + pane.viewport_h {
            scroll = scroll.vertical_scroll_offset((top - pane.viewport_h / 2.0).max(0.0));
        }
    }
    let visible: Vec<NodeId> = pane.view.visible().to_vec();
    if visible.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("No differences — every entry is identical.").color(pal.muted_fg));
        });
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = scroll.show_rows(ui, row_h, visible.len(), |ui, range| {
        for row in range {
            paint_row(ui, pane, pal, tz, visible[row], row_h, commands);
        }
    });
    pane.offset_y = out.state.clamped_offset().y;
    pane.viewport_h = out.inner_rect.height();
}

fn header(ui: &mut Ui, pane: &FolderPane, pal: &Palette) {
    let small = TextStyle::Small.resolve(ui.style());
    let h = ui.text_style_height(&TextStyle::Small) + 10.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let painter = ui.painter_at(rect);
    let half = (rect.width() - MARKER_W) / 2.0;
    let roots = [&pane.view.diff.left_root, &pane.view.diff.right_root];
    for (i, root) in roots.iter().enumerate() {
        let x0 = rect.min.x + i as f32 * (half + MARKER_W);
        let cols = columns(x0, half);
        painter.text(pos2(cols.name, rect.center().y), Align2::LEFT_CENTER, root.display().to_string(), small.clone(), pal.muted_fg);
        painter.text(pos2(cols.size_right, rect.center().y), Align2::RIGHT_CENTER, "Size", small.clone(), pal.muted_fg);
        painter.text(pos2(cols.date, rect.center().y), Align2::LEFT_CENTER, "Modified", small.clone(), pal.muted_fg);
    }
    painter.line_segment([pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)], Stroke::new(1.0, pal.gutter_bg));
}

struct Columns {
    name: f32,
    name_end: f32,
    size_right: f32,
    date: f32,
}

fn columns(x0: f32, width: f32) -> Columns {
    let date_w = 130.0_f32.min(width * 0.3);
    let size_w = 80.0_f32.min(width * 0.2);
    Columns {
        name: x0 + 6.0,
        name_end: x0 + width - date_w - size_w - 12.0,
        size_right: x0 + width - date_w - 12.0,
        date: x0 + width - date_w,
    }
}

fn paint_row(
    ui: &mut Ui,
    pane: &mut FolderPane,
    pal: &Palette,
    tz: &jiff::tz::TimeZone,
    id: NodeId,
    row_h: f32,
    commands: &mut Vec<Command>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
    let painter = ui.painter_at(rect);
    let body = TextStyle::Body.resolve(ui.style());
    let node = pane.view.diff.node(id).clone();
    let selected = pane.view.selected() == Some(id);
    let half = (rect.width() - MARKER_W) / 2.0;

    if selected {
        painter.rect_filled(rect, 0.0, pal.selection_bg);
    }
    let (marker, marker_color) = match node.status {
        Status::Identical => ("=", pal.muted_fg),
        Status::Different => ("≠", pal.changed_fg),
        Status::LeftOnly => ("←", pal.removed_fg),
        Status::RightOnly => ("→", pal.added_fg),
        Status::TypeMismatch => ("⇄", pal.changed_fg),
        Status::Error => ("!", pal.error_fg),
    };
    let leaf_different = node.status == Status::Different && !node.is_dir();
    for (i, side) in [Side::Left, Side::Right].into_iter().enumerate() {
        let x0 = rect.min.x + i as f32 * (half + MARKER_W);
        let cell = Rect::from_min_size(pos2(x0, rect.min.y), vec2(half, row_h));
        let tint = match (node.status, side) {
            (Status::LeftOnly, Side::Left) => Some(pal.removed_bg),
            (Status::RightOnly, Side::Right) => Some(pal.added_bg),
            (Status::LeftOnly, Side::Right) | (Status::RightOnly, Side::Left) => Some(pal.filler_bg),
            (Status::TypeMismatch | Status::Error, _) => Some(pal.changed_bg),
            _ if leaf_different => Some(pal.changed_bg),
            _ => None,
        };
        if let Some(t) = tint.filter(|_| !selected) {
            painter.rect_filled(cell, 0.0, t);
        }
        if let Some(meta) = node.side(side) {
            paint_entry(&painter, pane, pal, tz, &body, id, &node, meta, x0, half, rect.center().y);
        }
    }
    let mid = rect.min.x + half + MARKER_W / 2.0;
    let mono = TextStyle::Monospace.resolve(ui.style());
    painter.text(pos2(mid, rect.center().y), Align2::CENTER_CENTER, marker, mono, marker_color);
    if node.error.is_some() {
        painter.text(pos2(rect.max.x - 6.0, rect.center().y), Align2::RIGHT_CENTER, "!", body, pal.error_fg);
    }

    let label = pane.view.row_label(id);
    response.widget_info(|| WidgetInfo::selected(Role::TreeItem, true, selected, label.clone()));
    let response = match &node.error {
        Some(e) => response.on_hover_text(e.as_str()),
        None => response,
    };
    if response.clicked() {
        pane.view.select(id);
        let indent = f32::from(node.depth.saturating_sub(1)) * INDENT;
        let on_arrow = |x: f32| {
            [rect.min.x, rect.min.x + half + MARKER_W].iter().any(|&x0| {
                let arrow_x = columns(x0, half).name + indent;
                (arrow_x - 4.0..=arrow_x + INDENT).contains(&x)
            })
        };
        if node.has_children() && response.interact_pointer_pos().is_some_and(|p| on_arrow(p.x)) {
            pane.view.toggle(id);
        }
    }
    if response.double_clicked() {
        pane.view.select(id);
        commands.push(Command::OpenSelected);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_entry(
    painter: &egui::Painter,
    pane: &FolderPane,
    pal: &Palette,
    tz: &jiff::tz::TimeZone,
    font: &FontId,
    id: NodeId,
    node: &difflux_diff_dir::Node,
    meta: &EntryMeta,
    x0: f32,
    width: f32,
    y: f32,
) {
    let cols = columns(x0, width);
    let indent = f32::from(node.depth.saturating_sub(1)) * INDENT;
    let mut x = cols.name + indent;
    if node.has_children() {
        let arrow = if pane.view.is_expanded(id) { "⏷" } else { "⏵" };
        painter.text(pos2(x, y), Align2::LEFT_CENTER, arrow, font.clone(), pal.muted_fg);
    }
    x += INDENT;
    let is_dir = meta.kind == EntryKind::Dir;
    let color = match node.status {
        Status::Different if is_dir => pal.changed_fg,
        Status::Error => pal.error_fg,
        _ => pal.text_fg,
    };
    let name = if is_dir { format!("{}/", meta.name) } else { meta.name.clone() };
    let clip = Rect::from_min_max(pos2(x, y - 20.0), pos2(cols.name_end, y + 20.0));
    painter.with_clip_rect(clip).text(pos2(x, y), Align2::LEFT_CENTER, name, font.clone(), color);
    if meta.kind == EntryKind::File {
        painter.text(pos2(cols.size_right, y), Align2::RIGHT_CENTER, format::size(meta.size), font.clone(), pal.muted_fg);
    }
    painter.text(pos2(cols.date, y), Align2::LEFT_CENTER, format::timestamp(meta.modified_ms, tz), font.clone(), pal.muted_fg);
}

fn keyboard(ui: &Ui, pane: &mut FolderPane, row_h: f32, commands: &mut Vec<Command>) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let page_rows = ((pane.viewport_h / row_h) as isize - 1).max(1);
    let (down, up, page_down, page_up, home, end, left, right, space) = ui.input(|i| {
        let plain = i.modifiers.is_none();
        (
            plain && i.key_pressed(Key::ArrowDown),
            plain && i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::PageDown),
            i.key_pressed(Key::PageUp),
            plain && i.key_pressed(Key::Home),
            plain && i.key_pressed(Key::End),
            plain && i.key_pressed(Key::ArrowLeft),
            plain && i.key_pressed(Key::ArrowRight),
            plain && i.key_pressed(Key::Space),
        )
    });
    let moved = if down {
        pane.view.move_selection(1)
    } else if up {
        pane.view.move_selection(-1)
    } else if page_down {
        pane.view.move_selection(page_rows)
    } else if page_up {
        pane.view.move_selection(-page_rows)
    } else if home {
        pane.view.move_selection(isize::MIN / 2)
    } else if end {
        pane.view.move_selection(isize::MAX / 2)
    } else if left {
        pane.view.collapse_or_parent();
        pane.view.selected_row()
    } else if right {
        pane.view.expand_or_child();
        pane.view.selected_row()
    } else if space {
        if let Some(id) = pane.view.selected() {
            if pane.view.diff.node(id).has_children() {
                pane.view.toggle(id);
            } else {
                commands.push(Command::OpenSelected);
            }
        }
        pane.view.selected_row()
    } else {
        None
    };
    if moved.is_some() {
        pane.pending_row = moved;
    }
}
