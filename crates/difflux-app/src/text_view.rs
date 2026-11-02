//! View-model for a side-by-side text comparison.

use std::collections::HashMap;

use difflux_core::{LineIdx, Side};
use difflux_diff_text::{InlineDiff, Row, RowKind, inline_diff};
use difflux_text::Document;

use crate::compare::FileCompare;

/// Cached inline diffs are dropped beyond this many rows (only visible rows are ever requested).
const INLINE_CACHE_LIMIT: usize = 4096;
/// Longest line text included in an accessibility label.
const LABEL_MAX_BYTES: usize = 300;

/// Navigation and rendering state for one file comparison.
#[derive(Debug)]
pub struct TextView {
    pub compare: FileCompare,
    /// Hunk the user last navigated to.
    pub current_hunk: Option<usize>,
    inline: HashMap<u32, InlineDiff>,
}

impl TextView {
    #[must_use]
    pub fn new(compare: FileCompare) -> Self {
        Self { compare, current_hunk: None, inline: HashMap::new() }
    }

    #[must_use]
    pub fn rows(&self) -> &[Row] {
        self.compare.diff.as_ref().map_or(&[], |d| d.rows.as_slice())
    }

    #[must_use]
    pub fn hunk_count(&self) -> usize {
        self.compare.diff.as_ref().map_or(0, |d| d.hunks.len())
    }

    #[must_use]
    pub fn document(&self, side: Side) -> Option<&Document> {
        match side {
            Side::Left => self.compare.left.document(),
            Side::Right => self.compare.right.document(),
        }
    }

    /// Text of a row on one side, or `None` for a filler cell.
    #[must_use]
    pub fn cell(&self, row: usize, side: Side) -> Option<(LineIdx, &str)> {
        let r = self.rows().get(row)?;
        let idx = match side {
            Side::Left => r.left?,
            Side::Right => r.right?,
        };
        Some((idx, self.document(side)?.line(idx)))
    }

    /// Word-level highlights for a modified row (empty for other rows).
    pub fn inline(&mut self, row: usize) -> &InlineDiff {
        if self.inline.len() > INLINE_CACHE_LIMIT {
            self.inline.clear();
        }
        let key = row as u32;
        if !self.inline.contains_key(&key) {
            let value = match (self.rows().get(row).map(|r| r.kind), self.cell(row, Side::Left), self.cell(row, Side::Right)) {
                (Some(RowKind::Modify), Some((_, l)), Some((_, r))) => inline_diff(l, r, &self.compare.options),
                _ => InlineDiff::default(),
            };
            self.inline.insert(key, value);
        }
        &self.inline[&key]
    }

    /// Moves to the next difference after `from_row` and returns its first row.
    pub fn next_difference(&mut self, from_row: Option<usize>) -> Option<usize> {
        let diff = self.compare.diff.as_ref()?;
        let from = from_row.or_else(|| self.current_hunk.map(|h| diff.hunks[h].rows.start as usize));
        let h = diff.next_hunk(from.map(|r| r as u32))?;
        self.current_hunk = Some(h);
        Some(diff.hunks[h].rows.start as usize)
    }

    /// Moves to the previous difference before `from_row` and returns its first row.
    pub fn previous_difference(&mut self, from_row: Option<usize>) -> Option<usize> {
        let diff = self.compare.diff.as_ref()?;
        let from = from_row.or_else(|| self.current_hunk.map(|h| diff.hunks[h].rows.start as usize));
        let h = diff.prev_hunk(from.map(|r| r as u32))?;
        self.current_hunk = Some(h);
        Some(diff.hunks[h].rows.start as usize)
    }

    pub fn first_difference(&mut self) -> Option<usize> {
        self.current_hunk = None;
        self.next_difference(None)
    }

    pub fn last_difference(&mut self) -> Option<usize> {
        self.current_hunk = None;
        self.previous_difference(None)
    }

    /// Whether a row belongs to the hunk the user navigated to.
    #[must_use]
    pub fn in_current_hunk(&self, row: usize) -> bool {
        match (&self.compare.diff, self.current_hunk) {
            (Some(d), Some(h)) => d.hunks[h].rows.contains(&(row as u32)),
            _ => false,
        }
    }

    /// Screen-reader description of a row.
    #[must_use]
    pub fn row_label(&self, row: usize) -> String {
        let Some(r) = self.rows().get(row) else { return String::new() };
        let left = self.cell(row, Side::Left);
        let right = self.cell(row, Side::Right);
        // Screen readers get the start of very long lines, not megabytes of text.
        let fmt = |c: Option<(LineIdx, &str)>| {
            c.map(|(i, t)| {
                let cut = t.floor_char_boundary(LABEL_MAX_BYTES);
                if cut < t.len() { format!("line {i}: {}…", &t[..cut]) } else { format!("line {i}: {t}") }
            })
        };
        match r.kind {
            RowKind::Equal => format!("Unchanged {}", fmt(left).unwrap_or_default()),
            RowKind::Delete => format!("Removed from left, {}", fmt(left).unwrap_or_default()),
            RowKind::Insert => format!("Added on right, {}", fmt(right).unwrap_or_default()),
            RowKind::Modify => format!("Changed. Left {}. Right {}", fmt(left).unwrap_or_default(), fmt(right).unwrap_or_default()),
        }
    }

    /// The comparison as a unified diff, for copying or saving as a patch.
    #[must_use]
    pub fn unified(&self, context: u32) -> String {
        let mut out = String::new();
        if let (Some(d), Some(l), Some(r)) = (&self.compare.diff, self.compare.left.document(), self.compare.right.document()) {
            d.write_unified(l, r, (&self.compare.left.name, &self.compare.right.name), context, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare_texts;
    use difflux_core::{CancelToken, TextOptions};

    fn view(a: &str, b: &str) -> TextView {
        TextView::new(compare_texts(a.into(), b.into(), &TextOptions::default(), &CancelToken::new()).expect("ok"))
    }

    #[test]
    fn navigates_differences_and_wraps_at_the_ends() {
        let mut v = view("1\n2\n3\n4\n5\n", "1\nX\n3\n4\nY\n");
        assert_eq!(v.next_difference(None), Some(1));
        assert!(v.in_current_hunk(1));
        assert_eq!(v.next_difference(None), Some(4));
        assert_eq!(v.next_difference(None), None);
        assert_eq!(v.previous_difference(None), Some(1));
        assert_eq!(v.last_difference(), Some(4));
        assert_eq!(v.first_difference(), Some(1));
    }

    #[test]
    fn row_labels_describe_changes() {
        let mut v = view("keep\nold value\n", "keep\nnew value\n");
        assert_eq!(v.row_label(0), "Unchanged line 1: keep");
        assert_eq!(v.row_label(1), "Changed. Left line 2: old value. Right line 2: new value");
        assert_eq!(v.inline(1).left, vec![0..3]);
    }
}
