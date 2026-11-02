//! Every user-facing action, in one list.
//!
//! Menus, keyboard shortcuts, the command palette and (later) scripting all
//! dispatch the same [`Command`]s, so they can never drift apart. Stable `id`s
//! are part of the public interface: agents and settings files refer to them.

/// A user-facing action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Compare,
    SwapSides,
    NextDifference,
    PreviousDifference,
    FirstDifference,
    LastDifference,
    OpenSelected,
    Back,
    ToggleIgnoreWhitespace,
    ToggleIgnoreCase,
    ToggleIgnoreLineEndings,
    ToggleExactContent,
    ToggleDifferencesOnly,
    ExpandAll,
    CollapseAll,
    CopyUnifiedDiff,
    BrowseLeft,
    BrowseRight,
    FocusLeftPath,
    FocusRightPath,
    CancelCompare,
    ToggleTheme,
    CommandPalette,
    ShowShortcuts,
}

impl Command {
    pub const ALL: &'static [Self] = &[
        Self::Compare,
        Self::SwapSides,
        Self::NextDifference,
        Self::PreviousDifference,
        Self::FirstDifference,
        Self::LastDifference,
        Self::OpenSelected,
        Self::Back,
        Self::ToggleIgnoreWhitespace,
        Self::ToggleIgnoreCase,
        Self::ToggleIgnoreLineEndings,
        Self::ToggleExactContent,
        Self::ToggleDifferencesOnly,
        Self::ExpandAll,
        Self::CollapseAll,
        Self::CopyUnifiedDiff,
        Self::BrowseLeft,
        Self::BrowseRight,
        Self::FocusLeftPath,
        Self::FocusRightPath,
        Self::CancelCompare,
        Self::ToggleTheme,
        Self::CommandPalette,
        Self::ShowShortcuts,
    ];

    /// Stable machine identifier.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Compare => "compare",
            Self::SwapSides => "swap_sides",
            Self::NextDifference => "next_difference",
            Self::PreviousDifference => "previous_difference",
            Self::FirstDifference => "first_difference",
            Self::LastDifference => "last_difference",
            Self::OpenSelected => "open_selected",
            Self::Back => "back",
            Self::ToggleIgnoreWhitespace => "toggle_ignore_whitespace",
            Self::ToggleIgnoreCase => "toggle_ignore_case",
            Self::ToggleIgnoreLineEndings => "toggle_ignore_line_endings",
            Self::ToggleExactContent => "toggle_exact_content",
            Self::ToggleDifferencesOnly => "toggle_differences_only",
            Self::ExpandAll => "expand_all",
            Self::CollapseAll => "collapse_all",
            Self::CopyUnifiedDiff => "copy_unified_diff",
            Self::BrowseLeft => "browse_left",
            Self::BrowseRight => "browse_right",
            Self::FocusLeftPath => "focus_left_path",
            Self::FocusRightPath => "focus_right_path",
            Self::CancelCompare => "cancel_compare",
            Self::ToggleTheme => "toggle_theme",
            Self::CommandPalette => "command_palette",
            Self::ShowShortcuts => "show_shortcuts",
        }
    }

    /// Human title, used in menus and the palette.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Compare => "Compare / refresh",
            Self::SwapSides => "Swap left and right",
            Self::NextDifference => "Next difference",
            Self::PreviousDifference => "Previous difference",
            Self::FirstDifference => "First difference",
            Self::LastDifference => "Last difference",
            Self::OpenSelected => "Open selected entry",
            Self::Back => "Back to folder view",
            Self::ToggleIgnoreWhitespace => "Ignore whitespace changes",
            Self::ToggleIgnoreCase => "Ignore case",
            Self::ToggleIgnoreLineEndings => "Ignore line endings",
            Self::ToggleExactContent => "Always compare file contents",
            Self::ToggleDifferencesOnly => "Show differences only",
            Self::ExpandAll => "Expand all folders",
            Self::CollapseAll => "Collapse all folders",
            Self::CopyUnifiedDiff => "Copy as unified diff",
            Self::BrowseLeft => "Browse for left side…",
            Self::BrowseRight => "Browse for right side…",
            Self::FocusLeftPath => "Edit left path",
            Self::FocusRightPath => "Edit right path",
            Self::CancelCompare => "Cancel running compare",
            Self::ToggleTheme => "Switch light / dark theme",
            Self::CommandPalette => "Command palette",
            Self::ShowShortcuts => "Keyboard shortcuts",
        }
    }

    /// Looks a command up by its stable id.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.id() == id)
    }
}

/// Case-insensitive subsequence match used by the command palette.
/// Returns a score (higher is better) or `None` when `query` does not match.
#[must_use]
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    let mut score = 0u32;
    let mut last: Option<usize> = None;
    let cand: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut pos = 0;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = cand[pos..].iter().position(|&c| c == q)? + pos;
        score += match last {
            Some(l) if found == l + 1 => 3,                                        // consecutive
            _ if found == 0 || cand.get(found.wrapping_sub(1)) == Some(&' ') => 2, // word start
            _ => 1,
        };
        last = Some(found);
        pos = found + 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_round_trip() {
        let mut ids: Vec<&str> = Command::ALL.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Command::ALL.len());
        for c in Command::ALL {
            assert_eq!(Command::from_id(c.id()), Some(*c));
        }
    }

    #[test]
    fn fuzzy_matching_prefers_word_starts() {
        assert!(fuzzy_score("nd", "Next difference").is_some());
        assert!(fuzzy_score("xyz", "Next difference").is_none());
        let a = fuzzy_score("swap", "Swap left and right").unwrap_or(0);
        let b = fuzzy_score("swap", "Show whitespace as plain").unwrap_or(0);
        assert!(a > b);
    }
}
