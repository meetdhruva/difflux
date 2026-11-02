//! Application state, command dispatch and the window chrome
//! (toolbar, status bar, home screen).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use difflux_app::{Command, CompareError, Comparison, Filter, FolderView, Job, TextView, compare_paths};
use difflux_core::{CompareOptions, ContentMode, Side, WhitespaceMode};
use egui::{Align, Context, Id, Key, Layout, RichText, TextEdit, Ui};
use serde::{Deserialize, Serialize};

use crate::folder_pane::{self, FolderPane};
use crate::format;
use crate::keymap;
use crate::palette::{self, PaletteState};
use crate::text_pane::{self, TextPane};
use crate::theme::{Palette, ThemeChoice};

const MAX_RECENT: usize = 10;
const SETTINGS_KEY: &str = "difflux-settings-v1";

/// Everything remembered between runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub left: String,
    pub right: String,
    pub options: CompareOptions,
    pub differences_only: bool,
    theme: ThemeChoice,
    pub recent: Vec<(String, String)>,
}

/// What a finished job does with its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A new top-level comparison: replaces the whole view stack.
    Root,
    /// Opened from a folder view: pushed on top.
    Push,
    /// Same inputs, new options: replaces the top view.
    Refresh,
}

#[derive(Debug)]
struct Pending {
    job: Job<Result<Comparison, CompareError>>,
    target: Target,
    left: PathBuf,
    right: PathBuf,
}

#[derive(Debug)]
enum View {
    Text(Box<TextPane>),
    Folder(Box<FolderPane>),
}

/// The desktop application.
#[derive(Debug)]
pub struct DiffluxApp {
    settings: Settings,
    views: Vec<View>,
    pending: Option<Pending>,
    error: Option<String>,
    notice: Option<(String, Instant)>,
    palette: PaletteState,
    show_shortcuts: bool,
    focus: Option<Side>,
    compare_on_start: bool,
    theme_applied: bool,
    tz: jiff::tz::TimeZone,
}

impl DiffluxApp {
    /// Creates the app from eframe, restoring saved settings.
    pub fn new(cc: &eframe::CreationContext<'_>, left: Option<PathBuf>, right: Option<PathBuf>) -> Self {
        let settings = cc.storage.and_then(|s| eframe::get_value::<Settings>(s, SETTINGS_KEY)).unwrap_or_default();
        Self::with_settings(settings, left, right)
    }

    /// Creates the app with explicit settings (used by tests and embedders).
    #[must_use]
    pub fn with_settings(mut settings: Settings, left: Option<PathBuf>, right: Option<PathBuf>) -> Self {
        let compare_on_start = left.is_some() && right.is_some();
        if let Some(l) = left {
            settings.left = l.display().to_string();
        }
        if let Some(r) = right {
            settings.right = r.display().to_string();
        }
        Self {
            settings,
            views: Vec::new(),
            pending: None,
            error: None,
            notice: None,
            palette: PaletteState::default(),
            show_shortcuts: false,
            focus: None,
            compare_on_start,
            theme_applied: false,
            tz: jiff::tz::TimeZone::try_system().unwrap_or(jiff::tz::TimeZone::UTC),
        }
    }

    /// True while a comparison is running.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Message of the last failed comparison.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Number of views on the navigation stack.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.views.len()
    }

    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Runs a command as if the user triggered it.
    pub fn run_command(&mut self, ctx: &Context, command: Command) {
        self.dispatch(ctx, command);
    }

    /// Draws one frame. Separate from `eframe::App` so tests can drive it.
    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if !self.theme_applied {
            ctx.set_theme(self.settings.theme.preference());
            self.theme_applied = true;
        }
        if std::mem::take(&mut self.compare_on_start) {
            self.dispatch(&ctx, Command::Compare);
        }
        self.poll(&ctx);
        self.handle_drops(&ctx);

        let mut commands = if self.palette.open { Vec::new() } else { keymap::pressed(&ctx) };
        let pal = Palette::for_visuals(ui.visuals());

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, &mut commands));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &pal, &mut commands));
        egui::CentralPanel::default().show(ui, |ui| self.central(ui, &pal, &mut commands));

        let views = &self.views;
        if let Some(cmd) = self.palette.show(&ctx, |c| available(views, c)) {
            commands.push(cmd);
        }
        palette::shortcuts_window(&ctx, &mut self.show_shortcuts);

        for cmd in commands {
            self.dispatch(&ctx, cmd);
        }
        if self.pending.is_some() {
            // Keep progress counters moving; the job itself wakes us when done.
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if let Some((_, at)) = &self.notice {
            let left = Duration::from_secs(3).saturating_sub(at.elapsed());
            if left.is_zero() {
                self.notice = None;
            } else {
                ctx.request_repaint_after(left);
            }
        }
    }

    // ----- jobs -------------------------------------------------------------

    fn start(&mut self, ctx: &Context, left: PathBuf, right: PathBuf, target: Target) {
        self.error = None;
        let opts = self.settings.options.clone();
        let (l, r) = (left.clone(), right.clone());
        let repaint = ctx.clone();
        match Job::spawn(
            "compare",
            move |cancel, progress| compare_paths(&l, &r, &opts, cancel, progress),
            move || repaint.request_repaint(),
        ) {
            // Replacing the pending job drops (and so cancels) the previous one.
            Ok(job) => self.pending = Some(Pending { job, target, left, right }),
            Err(e) => self.error = Some(format!("cannot start compare: {e}")),
        }
    }

    fn poll(&mut self, ctx: &Context) {
        let Some(result) = self.pending.as_ref().and_then(|p| p.job.poll()) else { return };
        let Some(p) = self.pending.take() else { return };
        let comparison = match result {
            Ok(Ok(c)) => c,
            Ok(Err(CompareError::Cancelled)) => return,
            Ok(Err(e)) => {
                self.error = Some(e.to_string());
                return;
            }
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let previous_offset = match (p.target, self.views.last()) {
            (Target::Refresh, Some(View::Text(t))) => Some(t.scroll_offset()),
            _ => None,
        };
        let view = match comparison {
            Comparison::Files(f) => {
                let mut pane = TextPane::new(TextView::new(*f), p.left.clone(), p.right.clone());
                match previous_offset {
                    Some(y) => pane.pending_offset = Some(y),
                    None => pane.pending_row = pane.view.first_difference(),
                }
                View::Text(Box::new(pane))
            }
            Comparison::Folders(d) => {
                let filter = if self.settings.differences_only { Filter::Differences } else { Filter::All };
                View::Folder(Box::new(FolderPane::new(FolderView::new(*d, filter))))
            }
        };
        match p.target {
            Target::Root => {
                self.views = vec![view];
                self.remember(&p.left, &p.right);
            }
            Target::Push => self.views.push(view),
            Target::Refresh => {
                self.views.pop();
                self.views.push(view);
            }
        }
        ctx.request_repaint();
    }

    fn remember(&mut self, left: &Path, right: &Path) {
        let entry = (left.display().to_string(), right.display().to_string());
        self.settings.recent.retain(|e| e != &entry);
        self.settings.recent.insert(0, entry);
        self.settings.recent.truncate(MAX_RECENT);
    }

    fn handle_drops(&mut self, ctx: &Context) {
        let (dropped, pointer_x, width) = ctx.input(|i| {
            (
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>(),
                i.pointer.hover_pos().map(|p| p.x),
                i.viewport_rect().width(),
            )
        });
        match dropped.as_slice() {
            [] => {}
            [one] => {
                let right_half = pointer_x.is_some_and(|x| x > width / 2.0);
                if right_half {
                    self.settings.right = one.display().to_string();
                } else {
                    self.settings.left = one.display().to_string();
                }
                if !self.settings.left.is_empty() && !self.settings.right.is_empty() {
                    self.dispatch(ctx, Command::Compare);
                }
            }
            [a, b, ..] => {
                self.settings.left = a.display().to_string();
                self.settings.right = b.display().to_string();
                self.dispatch(ctx, Command::Compare);
            }
        }
    }

    // ----- commands ---------------------------------------------------------

    fn dispatch(&mut self, ctx: &Context, command: Command) {
        match command {
            Command::Compare => {
                let (l, r) = (self.settings.left.trim(), self.settings.right.trim());
                if l.is_empty() || r.is_empty() {
                    self.error = Some("Choose both a left and a right file or folder.".into());
                } else {
                    let (l, r) = (PathBuf::from(l), PathBuf::from(r));
                    self.start(ctx, l, r, Target::Root);
                }
            }
            Command::SwapSides => {
                std::mem::swap(&mut self.settings.left, &mut self.settings.right);
                if !self.views.is_empty() {
                    self.dispatch(ctx, Command::Compare);
                }
            }
            Command::NextDifference | Command::PreviousDifference | Command::FirstDifference | Command::LastDifference => {
                self.navigate(command);
            }
            Command::OpenSelected => self.open_selected(ctx),
            Command::Back => {
                if self.views.len() > 1 {
                    self.views.pop();
                }
            }
            Command::ToggleIgnoreWhitespace => {
                let t = &mut self.settings.options.text;
                t.whitespace = if t.whitespace == WhitespaceMode::Exact { WhitespaceMode::Change } else { WhitespaceMode::Exact };
                self.refresh(ctx, false);
            }
            Command::ToggleIgnoreCase => {
                self.settings.options.text.ignore_case ^= true;
                self.refresh(ctx, false);
            }
            Command::ToggleIgnoreLineEndings => {
                self.settings.options.text.ignore_line_endings ^= true;
                self.refresh(ctx, false);
            }
            Command::ToggleExactContent => {
                let f = &mut self.settings.options.folder;
                f.content = if f.content == ContentMode::Exact { ContentMode::Smart } else { ContentMode::Exact };
                self.refresh(ctx, true);
            }
            Command::ToggleDifferencesOnly => {
                self.settings.differences_only ^= true;
                let filter = if self.settings.differences_only { Filter::Differences } else { Filter::All };
                for v in &mut self.views {
                    if let View::Folder(f) = v {
                        f.view.set_filter(filter);
                    }
                }
            }
            Command::ExpandAll | Command::CollapseAll => {
                if let Some(View::Folder(f)) = self.views.last_mut() {
                    if command == Command::ExpandAll {
                        f.view.expand_all();
                    } else {
                        f.view.collapse_all();
                    }
                }
            }
            Command::CopyUnifiedDiff => {
                if let Some(View::Text(t)) = self.views.last() {
                    ctx.copy_text(t.view.unified(3));
                    self.notify("Copied unified diff to the clipboard");
                }
            }
            Command::BrowseLeft => self.browse(Side::Left, None),
            Command::BrowseRight => self.browse(Side::Right, None),
            Command::FocusLeftPath => self.focus = Some(Side::Left),
            Command::FocusRightPath => self.focus = Some(Side::Right),
            Command::CancelCompare => {
                if self.pending.take().is_some() {
                    self.notify("Compare cancelled");
                } else if self.error.take().is_none() {
                    self.dispatch(ctx, Command::Back);
                }
            }
            Command::ToggleTheme => {
                self.settings.theme = self.settings.theme.next(ctx.global_style().visuals.dark_mode);
                ctx.set_theme(self.settings.theme.preference());
            }
            Command::CommandPalette => self.palette.open(),
            Command::ShowShortcuts => self.show_shortcuts ^= true,
        }
    }

    fn notify(&mut self, text: &str) {
        self.notice = Some((text.to_owned(), Instant::now()));
    }

    /// Re-runs the comparison on screen after an option changed.
    fn refresh(&mut self, ctx: &Context, folder_option: bool) {
        match (self.views.last(), self.views.len()) {
            (None, _) => {}
            (Some(View::Text(t)), n) if n > 1 && !folder_option => {
                let (l, r) = (t.left.clone(), t.right.clone());
                self.start(ctx, l, r, Target::Refresh);
            }
            (Some(View::Text(t)), 1) => {
                let (l, r) = (t.left.clone(), t.right.clone());
                self.start(ctx, l, r, Target::Refresh);
            }
            _ => self.dispatch(ctx, Command::Compare),
        }
    }

    fn navigate(&mut self, command: Command) {
        match self.views.last_mut() {
            Some(View::Text(t)) => {
                let anchor = t.nav_anchor();
                let row = match command {
                    Command::NextDifference => t.view.next_difference(anchor),
                    Command::PreviousDifference => t.view.previous_difference(anchor.map(|a| a + 1)),
                    Command::FirstDifference => t.view.first_difference(),
                    _ => t.view.last_difference(),
                };
                match row {
                    Some(r) => t.pending_row = Some(r),
                    None if t.view.hunk_count() == 0 => self.notify("No differences"),
                    None => {
                        self.notify(if command == Command::NextDifference {
                            "Last difference reached"
                        } else {
                            "First difference reached"
                        });
                    }
                }
            }
            Some(View::Folder(f)) => {
                let forward = matches!(command, Command::NextDifference | Command::LastDifference);
                if command == Command::FirstDifference {
                    f.view.move_selection(isize::MIN / 2);
                }
                let row = f.view.select_difference(forward);
                if row.is_none() {
                    self.notify("No more differences");
                }
                if let Some(View::Folder(f)) = self.views.last_mut() {
                    f.pending_row = row.or(f.view.selected_row());
                }
            }
            None => {}
        }
    }

    fn open_selected(&mut self, ctx: &Context) {
        let Some(View::Folder(f)) = self.views.last_mut() else { return };
        let Some(id) = f.view.selected() else { return };
        let node = f.view.diff.node(id);
        if node.has_children() {
            f.view.toggle(id);
            return;
        }
        let left = f.view.diff.side_path(id, Side::Left).filter(|_| node.left.is_some());
        let right = f.view.diff.side_path(id, Side::Right).filter(|_| node.right.is_some());
        match (left, right) {
            (Some(l), Some(r)) if !node.is_dir() => self.start(ctx, l, r, Target::Push),
            (Some(_), None) => self.notify("This entry only exists on the left"),
            (None, Some(_)) => self.notify("This entry only exists on the right"),
            _ => {}
        }
    }

    fn browse(&mut self, side: Side, folder: Option<bool>) {
        let current = match side {
            Side::Left => &self.settings.left,
            Side::Right => &self.settings.right,
        };
        let current = Path::new(current.trim());
        let pick_folder = folder.unwrap_or_else(|| !current.is_file());
        let start_dir = if current.is_dir() { Some(current.to_path_buf()) } else { current.parent().map(Path::to_path_buf) };
        let mut dialog = rfd::FileDialog::new();
        if let Some(d) = start_dir.filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(d);
        }
        let picked = if pick_folder { dialog.pick_folder() } else { dialog.pick_file() };
        if let Some(p) = picked {
            let s = p.display().to_string();
            match side {
                Side::Left => self.settings.left = s,
                Side::Right => self.settings.right = s,
            }
        }
    }

    // ----- chrome -----------------------------------------------------------

    fn toolbar(&mut self, ui: &mut Ui, commands: &mut Vec<Command>) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let ctx = ui.ctx().clone();
            let spacing = ui.spacing().item_spacing.x;
            let height = ui.spacing().interact_size.y;
            let (compare_w, swap_w, browse_w, label_w) = (96.0, 32.0, 64.0, 34.0);
            // 8 widgets in the row → 7 gaps, plus a little slack so nothing wraps.
            let fixed = compare_w + swap_w + 2.0 * (browse_w + label_w) + 7.0 * spacing + 8.0;
            let field_w = ((ui.available_width() - fixed) / 2.0).max(120.0);

            for side in [Side::Left, Side::Right] {
                let (name, text) = match side {
                    Side::Left => ("Left", &mut self.settings.left),
                    Side::Right => ("Right", &mut self.settings.right),
                };
                let label = ui.add_sized([label_w, height], egui::Label::new(name));
                let edit = ui
                    .add(
                        TextEdit::singleline(text)
                            .id(Id::unique(("path", name)))
                            .hint_text("Path to a file or folder — or drop it here")
                            .desired_width(field_w),
                    )
                    .labelled_by(label.id);
                if self.focus == Some(side) {
                    edit.request_focus();
                    self.focus = None;
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    commands.push(Command::Compare);
                }
                let (browse_file, browse_folder) = browse_menu(ui, name);
                if browse_file {
                    self.browse(side, Some(false));
                }
                if browse_folder {
                    self.browse(side, Some(true));
                }
                if side == Side::Left
                    && ui
                        .add(egui::Button::new("↔").min_size(egui::vec2(swap_w, height)))
                        .on_hover_text(hint(&ctx, Command::SwapSides))
                        .clicked()
                {
                    commands.push(Command::SwapSides);
                }
            }
            let compare = ui.add(egui::Button::new(RichText::new("Compare").strong()).min_size(egui::vec2(compare_w, height)));
            if compare.on_hover_text(hint(&ctx, Command::Compare)).clicked() {
                commands.push(Command::Compare);
            }
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let ctx = ui.ctx().clone();
            if self.views.len() > 1 && ui.button("⏴ Folder").on_hover_text(hint(&ctx, Command::Back)).clicked() {
                commands.push(Command::Back);
            }
            match self.views.last() {
                Some(View::Text(t)) => {
                    if ui.button("⏶ Previous").on_hover_text(hint(&ctx, Command::PreviousDifference)).clicked() {
                        commands.push(Command::PreviousDifference);
                    }
                    if ui.button("⏷ Next").on_hover_text(hint(&ctx, Command::NextDifference)).clicked() {
                        commands.push(Command::NextDifference);
                    }
                    let n = t.view.hunk_count();
                    let label = match (t.view.current_hunk, n) {
                        (_, 0) => "No differences".to_owned(),
                        (Some(h), n) => format!("Difference {} of {n}", h + 1),
                        (None, n) => format!("{n} differences"),
                    };
                    ui.label(label);
                }
                Some(View::Folder(f)) => {
                    let mut only = self.settings.differences_only;
                    if ui.checkbox(&mut only, "Differences only").on_hover_text(hint(&ctx, Command::ToggleDifferencesOnly)).changed() {
                        commands.push(Command::ToggleDifferencesOnly);
                    }
                    if ui.button("Expand all").clicked() {
                        commands.push(Command::ExpandAll);
                    }
                    if ui.button("Collapse all").clicked() {
                        commands.push(Command::CollapseAll);
                    }
                    if ui.button("⏷ Next difference").on_hover_text(hint(&ctx, Command::NextDifference)).clicked() {
                        commands.push(Command::NextDifference);
                    }
                    let _ = f;
                }
                None => {}
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("☰ Commands").on_hover_text(hint(&ctx, Command::CommandPalette)).clicked() {
                    commands.push(Command::CommandPalette);
                }
                self.rules_menu(ui, commands);
            });
        });
        ui.add_space(4.0);
    }

    fn rules_menu(&mut self, ui: &mut Ui, commands: &mut Vec<Command>) {
        let opts = &self.settings.options;
        let active = usize::from(opts.text.whitespace != WhitespaceMode::Exact)
            + usize::from(opts.text.ignore_case)
            + usize::from(!opts.text.ignore_line_endings)
            + usize::from(opts.folder.content != ContentMode::Smart);
        let title = if active == 0 { "Rules".to_owned() } else { format!("Rules ({active})") };
        ui.menu_button(title, |ui| {
            let ctx = ui.ctx().clone();
            ui.label(RichText::new("Text").strong());
            let before = self.settings.options.text.whitespace;
            egui::ComboBox::from_label("Whitespace")
                .selected_text(whitespace_label(before))
                .show_ui(ui, |ui| {
                    for mode in [WhitespaceMode::Exact, WhitespaceMode::Trailing, WhitespaceMode::Change, WhitespaceMode::All] {
                        ui.selectable_value(&mut self.settings.options.text.whitespace, mode, whitespace_label(mode));
                    }
                });
            if self.settings.options.text.whitespace != before {
                self.refresh(&ctx, false);
            }
            let mut case = self.settings.options.text.ignore_case;
            if ui.checkbox(&mut case, "Ignore case").on_hover_text(hint(&ctx, Command::ToggleIgnoreCase)).changed() {
                commands.push(Command::ToggleIgnoreCase);
            }
            let mut eol = self.settings.options.text.ignore_line_endings;
            if ui.checkbox(&mut eol, "Ignore line endings (CRLF / LF)").on_hover_text(hint(&ctx, Command::ToggleIgnoreLineEndings)).changed() {
                commands.push(Command::ToggleIgnoreLineEndings);
            }
            ui.separator();
            ui.label(RichText::new("Folders").strong());
            let mut exact = self.settings.options.folder.content == ContentMode::Exact;
            if ui
                .checkbox(&mut exact, "Always compare file contents")
                .on_hover_text("Off: files with equal size and modified time are trusted to be identical (fast).\nOn: bytes are always read (slower, never wrong).")
                .changed()
            {
                commands.push(Command::ToggleExactContent);
            }
            ui.separator();
            if ui.button("Keyboard shortcuts…").clicked() {
                commands.push(Command::ShowShortcuts);
                ui.close();
            }
            if ui.button("Switch light / dark theme").clicked() {
                commands.push(Command::ToggleTheme);
                ui.close();
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui, pal: &Palette, commands: &mut Vec<Command>) {
        ui.horizontal(|ui| {
            if let Some(p) = &self.pending {
                ui.spinner();
                let items = p.job.progress().items();
                let mb = p.job.progress().bytes() as f64 / 1_048_576.0;
                ui.label(format!(
                    "Comparing… {} entries · {mb:.1} MB read · {:.1} s",
                    format::thousands(items),
                    p.job.elapsed().as_secs_f32()
                ));
                if ui.button("Cancel").on_hover_text("Esc").clicked() {
                    commands.push(Command::CancelCompare);
                }
            } else if let Some((text, _)) = &self.notice {
                ui.label(RichText::new(text).color(pal.changed_fg));
            } else {
                match self.views.last() {
                    Some(View::Text(t)) => {
                        let c = &t.view.compare;
                        ui.label(RichText::new(c.verdict.label()).strong());
                        if let Some(d) = &c.diff {
                            let s = d.stats;
                            ui.label(RichText::new(format!("− {}", s.removed)).color(pal.removed_fg));
                            ui.label(RichText::new(format!("+ {}", s.added)).color(pal.added_fg));
                            ui.label(RichText::new(format!("~ {}", s.modified)).color(pal.changed_fg));
                        }
                        ui.label(RichText::new(format!("{} ms", c.elapsed.as_millis())).color(pal.muted_fg));
                    }
                    Some(View::Folder(f)) => {
                        let s = f.view.diff.stats;
                        let verdict = if f.view.diff.is_equal() { "Folders are identical" } else { "Folders differ" };
                        ui.label(RichText::new(verdict).strong());
                        ui.label(RichText::new(format!("≠ {}", s.different)).color(pal.changed_fg));
                        ui.label(RichText::new(format!("◀ {}", s.left_only)).color(pal.removed_fg));
                        ui.label(RichText::new(format!("▶ {}", s.right_only)).color(pal.added_fg));
                        if s.errors > 0 {
                            ui.label(RichText::new(format!("! {} errors", s.errors)).color(pal.error_fg));
                        }
                        ui.label(RichText::new(format!("= {}", s.identical)).color(pal.muted_fg));
                        ui.label(RichText::new(format!("{:.2} s", f.view.diff.elapsed.as_secs_f64())).color(pal.muted_fg));
                    }
                    None => {
                        ui.label(RichText::new("Ready").color(pal.muted_fg));
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ctx = ui.ctx().clone();
                ui.label(
                    RichText::new(format!("{} for commands", keymap::label(&ctx, Command::CommandPalette).unwrap_or_default()))
                        .color(pal.muted_fg),
                );
            });
        });
    }

    fn central(&mut self, ui: &mut Ui, pal: &Palette, commands: &mut Vec<Command>) {
        if let Some(err) = self.error.clone() {
            egui::Frame::new().fill(pal.changed_bg).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").color(pal.error_fg));
                    ui.label(RichText::new(&err).color(pal.text_fg));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        let tz = self.tz.clone();
        match self.views.last_mut() {
            Some(View::Text(t)) => text_pane::show(ui, t, pal, commands),
            Some(View::Folder(f)) => folder_pane::show(ui, f, pal, &tz, commands),
            None => self.home(ui, pal),
        }
    }

    fn home(&mut self, ui: &mut Ui, pal: &Palette) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        ui.add_space(48.0);
        ui.vertical_centered(|ui| {
            ui.heading(RichText::new("difflux").size(32.0).strong());
            ui.label(RichText::new("Compare files and folders — fast, and without surprises.").color(pal.muted_fg));
            ui.add_space(20.0);
            let drop = if hovering { "Release to compare" } else { "Drop two files or folders here, or enter paths above" };
            ui.label(RichText::new(drop).size(16.0));
            ui.add_space(24.0);
            if !self.settings.recent.is_empty() {
                ui.label(RichText::new("Recent").strong());
                let mut chosen = None;
                for (l, r) in &self.settings.recent {
                    if ui.link(format!("{l}  ↔  {r}")).clicked() {
                        chosen = Some((l.clone(), r.clone()));
                    }
                }
                if let Some((l, r)) = chosen {
                    self.settings.left = l;
                    self.settings.right = r;
                    self.dispatch(&ui.ctx().clone(), Command::Compare);
                }
                ui.add_space(24.0);
            }
            let ctx = ui.ctx().clone();
            egui::Grid::new("home-keys").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                for cmd in
                    [Command::Compare, Command::NextDifference, Command::PreviousDifference, Command::SwapSides, Command::CommandPalette]
                {
                    ui.label(RichText::new(keymap::label(&ctx, cmd).unwrap_or_default()).monospace().color(pal.muted_fg));
                    ui.label(cmd.title());
                    ui.end_row();
                }
            });
        });
    }
}

impl eframe::App for DiffluxApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }
}

/// Whether a command makes sense in the current state (used to filter the palette).
fn available(views: &[View], c: Command) -> bool {
    match c {
        Command::NextDifference | Command::PreviousDifference | Command::FirstDifference | Command::LastDifference => !views.is_empty(),
        Command::CopyUnifiedDiff => matches!(views.last(), Some(View::Text(_))),
        Command::OpenSelected | Command::ExpandAll | Command::CollapseAll | Command::ToggleDifferencesOnly => {
            matches!(views.last(), Some(View::Folder(_)))
        }
        Command::Back => views.len() > 1,
        _ => true,
    }
}

fn browse_menu(ui: &mut Ui, side: &str) -> (bool, bool) {
    let mut out = (false, false);
    ui.menu_button("Browse", |ui| {
        if ui.button("Choose file…").clicked() {
            out.0 = true;
            ui.close();
        }
        if ui.button("Choose folder…").clicked() {
            out.1 = true;
            ui.close();
        }
    })
    .response
    .on_hover_text(format!("Browse for the {} side", side.to_lowercase()));
    out
}

fn hint(ctx: &Context, c: Command) -> String {
    match keymap::label(ctx, c) {
        Some(k) => format!("{} ({k})", c.title()),
        None => c.title().to_owned(),
    }
}

fn whitespace_label(m: WhitespaceMode) -> &'static str {
    match m {
        WhitespaceMode::Exact => "Compare exactly",
        WhitespaceMode::Trailing => "Ignore trailing",
        WhitespaceMode::Change => "Ignore amount",
        WhitespaceMode::All => "Ignore all",
    }
}
