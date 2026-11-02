//! View-model for a folder comparison: expansion, filtering, selection.

use difflux_diff_dir::{EntryKind, FolderDiff, NodeId, Status};

/// Folders with differences are opened automatically until this many rows are visible.
const AUTO_EXPAND_ROWS: usize = 20_000;

/// Which rows to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    /// Hide identical entries.
    Differences,
}

/// Tree state for one folder comparison.
#[derive(Debug)]
pub struct FolderView {
    pub diff: FolderDiff,
    expanded: Vec<bool>,
    filter: Filter,
    visible: Vec<NodeId>,
    selected: Option<NodeId>,
}

impl FolderView {
    /// Opens folders that contain differences so they are visible immediately.
    #[must_use]
    pub fn new(diff: FolderDiff, filter: Filter) -> Self {
        let mut expanded = vec![false; diff.nodes.len()];
        let mut budget = AUTO_EXPAND_ROWS;
        let mut stack = vec![FolderDiff::ROOT];
        while let Some(id) = stack.pop() {
            let node = diff.node(id);
            let both_sides = node.left.is_some() && node.right.is_some();
            let open = id == FolderDiff::ROOT || (both_sides && node.status.is_difference());
            if open && node.children.len() <= budget {
                expanded[id as usize] = true;
                budget -= node.children.len();
                stack.extend(node.children.clone().rev());
            }
        }
        let mut view = Self { diff, expanded, filter, visible: Vec::new(), selected: None };
        view.rebuild();
        view.selected = view.visible.first().copied();
        view
    }

    #[must_use]
    pub fn visible(&self) -> &[NodeId] {
        &self.visible
    }

    #[must_use]
    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.rebuild();
    }

    #[must_use]
    pub fn is_expanded(&self, id: NodeId) -> bool {
        self.expanded[id as usize]
    }

    pub fn set_expanded(&mut self, id: NodeId, open: bool) {
        if self.diff.node(id).has_children() && self.expanded[id as usize] != open {
            self.expanded[id as usize] = open;
            self.rebuild();
        }
    }

    pub fn toggle(&mut self, id: NodeId) {
        self.set_expanded(id, !self.is_expanded(id));
    }

    pub fn expand_all(&mut self) {
        self.expanded.fill(true);
        self.rebuild();
    }

    pub fn collapse_all(&mut self) {
        self.expanded.fill(false);
        self.expanded[FolderDiff::ROOT as usize] = true;
        self.rebuild();
    }

    #[must_use]
    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    /// Position of the selection among visible rows.
    #[must_use]
    pub fn selected_row(&self) -> Option<usize> {
        let s = self.selected?;
        self.visible.iter().position(|&v| v == s)
    }

    pub fn select(&mut self, id: NodeId) {
        self.selected = Some(id);
    }

    /// Moves the selection by `delta` visible rows (clamped) and returns the new row.
    pub fn move_selection(&mut self, delta: isize) -> Option<usize> {
        if self.visible.is_empty() {
            return None;
        }
        let max = self.visible.len() - 1;
        let row = self.selected_row().map_or(0, |r| r.saturating_add_signed(delta).min(max));
        self.selected = Some(self.visible[row]);
        Some(row)
    }

    /// Selects the next (or previous) visible entry that differs and returns its row.
    pub fn select_difference(&mut self, forward: bool) -> Option<usize> {
        let start = self.selected_row();
        let is_diff = |id: &NodeId| {
            let n = self.diff.node(*id);
            n.status.is_difference() && (n.kind() != EntryKind::Dir || n.status != Status::Different)
        };
        let row = if forward {
            let from = start.map_or(0, |r| r + 1);
            self.visible.iter().skip(from).position(is_diff).map(|p| p + from)
        } else {
            let to = start.unwrap_or(self.visible.len());
            self.visible[..to].iter().rposition(is_diff)
        }?;
        self.selected = Some(self.visible[row]);
        Some(row)
    }

    /// Collapses the selection or moves to its parent (Left arrow behaviour).
    pub fn collapse_or_parent(&mut self) {
        let Some(id) = self.selected else { return };
        if self.is_expanded(id) && self.diff.node(id).has_children() {
            self.set_expanded(id, false);
        } else if let Some(parent) = self.diff.node(id).parent.filter(|&p| p != FolderDiff::ROOT) {
            self.selected = Some(parent);
        }
    }

    /// Expands the selection or moves to its first child (Right arrow behaviour).
    pub fn expand_or_child(&mut self) {
        let Some(id) = self.selected else { return };
        if !self.diff.node(id).has_children() {
            return;
        }
        if self.is_expanded(id) {
            if let Some(row) = self.selected_row().filter(|r| r + 1 < self.visible.len()) {
                self.selected = Some(self.visible[row + 1]);
            }
        } else {
            self.set_expanded(id, true);
        }
    }

    /// Screen-reader description of an entry.
    #[must_use]
    pub fn row_label(&self, id: NodeId) -> String {
        let n = self.diff.node(id);
        let kind = match n.kind() {
            EntryKind::Dir => "folder",
            EntryKind::File => "file",
            EntryKind::Symlink => "link",
            EntryKind::Other => "entry",
        };
        let status = match n.status {
            Status::Identical => "identical",
            Status::Different => "different",
            Status::LeftOnly => "only on left",
            Status::RightOnly => "only on right",
            Status::TypeMismatch => "file on one side, folder on the other",
            Status::Error => "error",
        };
        let state = if n.has_children() { if self.is_expanded(id) { ", expanded" } else { ", collapsed" } } else { "" };
        let error = n.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default();
        format!("{} {kind}, {status}{state}{error}", self.diff.rel_path(id))
    }

    fn rebuild(&mut self) {
        self.visible.clear();
        let mut stack: Vec<NodeId> = self.diff.children(FolderDiff::ROOT).rev().collect();
        while let Some(id) = stack.pop() {
            let node = self.diff.node(id);
            if self.filter == Filter::Differences && !node.status.is_difference() {
                continue;
            }
            self.visible.push(id);
            if self.expanded[id as usize] {
                stack.extend(node.children.clone().rev());
            }
        }
        if let Some(s) = self.selected.filter(|s| !self.visible.contains(s)) {
            // Keep the selection on the nearest visible ancestor.
            let mut cur = self.diff.node(s).parent;
            self.selected = None;
            while let Some(p) = cur {
                if self.visible.contains(&p) {
                    self.selected = Some(p);
                    break;
                }
                cur = self.diff.node(p).parent;
            }
            if self.selected.is_none() {
                self.selected = self.visible.first().copied();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use difflux_core::{CancelToken, FolderOptions, Progress};
    use std::fs;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn tree() -> Result<(tempfile::TempDir, tempfile::TempDir, FolderDiff), Box<dyn std::error::Error>> {
        let (l, r) = (tempfile::tempdir()?, tempfile::tempdir()?);
        for root in [l.path(), r.path()] {
            fs::create_dir_all(root.join("same"))?;
            fs::write(root.join("same/a.txt"), "1")?;
            fs::create_dir_all(root.join("changed"))?;
            fs::write(root.join("top.txt"), "same")?;
        }
        fs::write(l.path().join("changed/b.txt"), "left")?;
        fs::write(r.path().join("changed/b.txt"), "right!")?;
        let d = difflux_diff_dir::compare_folders(l.path(), r.path(), &FolderOptions::default(), &CancelToken::new(), &Progress::new())?;
        Ok((l, r, d))
    }

    fn names(v: &FolderView) -> Vec<String> {
        v.visible().iter().map(|&id| v.diff.rel_path(id)).collect()
    }

    #[test]
    fn auto_expands_only_folders_with_differences() -> TestResult {
        let (_l, _r, d) = tree()?;
        let v = FolderView::new(d, Filter::All);
        assert_eq!(names(&v), ["changed", "changed/b.txt", "same", "top.txt"]);
        Ok(())
    }

    #[test]
    fn differences_filter_hides_identical_entries() -> TestResult {
        let (_l, _r, d) = tree()?;
        let mut v = FolderView::new(d, Filter::Differences);
        assert_eq!(names(&v), ["changed", "changed/b.txt"]);
        v.set_filter(Filter::All);
        v.expand_all();
        assert_eq!(names(&v), ["changed", "changed/b.txt", "same", "same/a.txt", "top.txt"]);
        Ok(())
    }

    #[test]
    fn keyboard_tree_navigation() -> TestResult {
        let (_l, _r, d) = tree()?;
        let mut v = FolderView::new(d, Filter::All);
        assert_eq!(v.selected_row(), Some(0));
        assert_eq!(v.select_difference(true), Some(1));
        v.collapse_or_parent();
        assert_eq!(v.selected_row(), Some(0));
        v.collapse_or_parent();
        assert_eq!(names(&v), ["changed", "same", "top.txt"]);
        v.expand_or_child();
        v.expand_or_child();
        assert_eq!(v.selected().map(|id| v.diff.rel_path(id)), Some("changed/b.txt".into()));
        assert!(v.row_label(v.selected().ok_or("none")?).contains("different"));
        Ok(())
    }
}
