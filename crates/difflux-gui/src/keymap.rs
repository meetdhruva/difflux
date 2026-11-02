//! Keyboard shortcuts for [`Command`]s.
//!
//! Several bindings per command on purpose: Beyond Compare users expect
//! `Ctrl+N`/`Ctrl+P`, VS Code users `F8`/`Shift+F8`, and `Alt+↑/↓` works everywhere.

use difflux_app::Command;
use egui::{Context, Key, KeyboardShortcut, Modifiers};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Binding {
    pub command: Command,
    pub shortcut: KeyboardShortcut,
}

const fn bind(command: Command, modifiers: Modifiers, key: Key) -> Binding {
    Binding { command, shortcut: KeyboardShortcut::new(modifiers, key) }
}

const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

/// All bindings, most specific first (egui matches extra Shift/Alt loosely).
pub(crate) const BINDINGS: &[Binding] = &[
    bind(Command::CommandPalette, CMD_SHIFT, Key::P),
    bind(Command::SwapSides, CMD_SHIFT, Key::X),
    bind(Command::CopyUnifiedDiff, CMD_SHIFT, Key::C),
    bind(Command::BrowseRight, CMD_SHIFT, Key::O),
    bind(Command::FocusRightPath, CMD_SHIFT, Key::L),
    bind(Command::PreviousDifference, SHIFT, Key::F8),
    bind(Command::Compare, CMD, Key::Enter),
    bind(Command::NextDifference, CMD, Key::N),
    bind(Command::PreviousDifference, CMD, Key::P),
    bind(Command::BrowseLeft, CMD, Key::O),
    bind(Command::FocusLeftPath, CMD, Key::L),
    bind(Command::ShowShortcuts, CMD, Key::Slash),
    bind(Command::NextDifference, ALT, Key::ArrowDown),
    bind(Command::PreviousDifference, ALT, Key::ArrowUp),
    bind(Command::FirstDifference, ALT, Key::Home),
    bind(Command::LastDifference, ALT, Key::End),
    bind(Command::Back, ALT, Key::ArrowLeft),
    bind(Command::ToggleIgnoreWhitespace, ALT, Key::W),
    bind(Command::ToggleIgnoreCase, ALT, Key::C),
    bind(Command::ToggleIgnoreLineEndings, ALT, Key::L),
    bind(Command::ToggleExactContent, ALT, Key::X),
    bind(Command::ToggleDifferencesOnly, ALT, Key::D),
    bind(Command::ExpandAll, ALT, Key::Plus),
    bind(Command::ExpandAll, ALT, Key::Equals),
    bind(Command::CollapseAll, ALT, Key::Minus),
    bind(Command::Compare, NONE, Key::F5),
    bind(Command::NextDifference, NONE, Key::F8),
    bind(Command::CommandPalette, NONE, Key::F1),
    // Bare keys: only when no text field has focus.
    bind(Command::OpenSelected, NONE, Key::Enter),
    bind(Command::Back, NONE, Key::Backspace),
    bind(Command::CancelCompare, NONE, Key::Escape),
];

fn is_bare(s: KeyboardShortcut) -> bool {
    s.modifiers.is_none() && !matches!(s.logical_key, Key::F1 | Key::F5 | Key::F8)
}

/// Consumes pressed shortcuts and returns their commands.
pub(crate) fn pressed(ctx: &Context) -> Vec<Command> {
    let typing = ctx.egui_wants_keyboard_input();
    ctx.input_mut(|i| {
        BINDINGS
            .iter()
            .filter(|b| !(typing && is_bare(b.shortcut)))
            .filter(|b| i.consume_shortcut(&b.shortcut))
            .map(|b| b.command)
            .collect()
    })
}

/// The shortcut shown in menus and tooltips: a function key when there is one
/// (F8 / Shift+F8 read as a pair), otherwise the simplest binding.
pub(crate) fn label(ctx: &Context, command: Command) -> Option<String> {
    let is_f_key = |b: &&Binding| matches!(b.shortcut.logical_key, Key::F1 | Key::F5 | Key::F8);
    let mut bound = BINDINGS.iter().filter(|b| b.command == command);
    let first_f = bound.clone().find(is_f_key);
    first_f.or_else(|| bound.next_back()).map(|b| ctx.format_shortcut(&b.shortcut))
}

/// Every label bound to `command`, for the shortcuts sheet.
pub(crate) fn labels(ctx: &Context, command: Command) -> Vec<String> {
    BINDINGS.iter().filter(|b| b.command == command).map(|b| ctx.format_shortcut(&b.shortcut)).collect()
}
