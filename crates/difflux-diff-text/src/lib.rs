//! Text comparison producing one aligned row model.
//!
//! Pipeline: normalize lines for matching → intern → histogram diff
//! (`imara-diff`) → slider post-processing → pair similar lines inside each
//! changed block → a flat [`Row`] vector. Both panes, scrolling, the overview
//! ruler and "next difference" all read that one vector.
//!
//! Inline (word-level) highlights are computed lazily per row with
//! [`inline_diff`], because only visible rows ever need them.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::ops::Range;

use difflux_core::{CancelToken, Cancelled, LineIdx, TextOptions, WhitespaceMode};
use difflux_text::{Document, normalize, word_tokens};
use imara_diff::{Algorithm, Diff, InternedInput};
use schemars::JsonSchema;
use serde::Serialize;

/// Changed blocks up to this many line pairs get similarity-based alignment;
/// larger blocks are paired positionally to keep worst-case time bounded.
const ALIGN_CELL_LIMIT: usize = 40_000;
/// Minimum similarity (0..=1) for two lines to be shown as one modified row.
const PAIR_THRESHOLD: f32 = 0.5;
/// Upper bound on `lines × hunks` for which the Histogram pass is affordable (~50 ms).
const HISTOGRAM_BUDGET: u64 = 40_000_000;
/// Lines with more tokens than this get a whole-line highlight instead of a word diff.
const INLINE_TOKEN_LIMIT: usize = 4_000;

/// What a row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RowKind {
    /// Same on both sides (under the active comparison rules).
    Equal,
    /// Only on the left.
    Delete,
    /// Only on the right.
    Insert,
    /// Present on both sides but changed.
    Modify,
}

/// One visual row of a side-by-side view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    pub left: Option<LineIdx>,
    pub right: Option<LineIdx>,
}

/// A contiguous block of non-equal rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Row indices covered by this hunk.
    pub rows: Range<u32>,
    /// Left line indices (zero-based, may be empty).
    pub left: Range<u32>,
    /// Right line indices (zero-based, may be empty).
    pub right: Range<u32>,
}

/// Line-level counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiffStats {
    pub hunks: u32,
    /// Lines only on the right.
    pub added: u32,
    /// Lines only on the left.
    pub removed: u32,
    /// Line pairs present on both sides but changed.
    pub modified: u32,
    pub unchanged: u32,
}

/// The result of comparing two documents.
#[derive(Debug, Clone, Default)]
pub struct TextDiff {
    pub rows: Vec<Row>,
    pub hunks: Vec<Hunk>,
    pub stats: DiffStats,
}

impl TextDiff {
    /// True when no differences remain under the active comparison rules.
    #[must_use]
    pub fn is_equal(&self) -> bool {
        self.hunks.is_empty()
    }

    /// Index of the hunk containing `row`, if any.
    #[must_use]
    pub fn hunk_at_row(&self, row: u32) -> Option<usize> {
        let i = self.hunks.partition_point(|h| h.rows.end <= row);
        self.hunks.get(i).filter(|h| h.rows.contains(&row)).map(|_| i)
    }

    /// First hunk starting after `row`.
    #[must_use]
    pub fn next_hunk(&self, row: Option<u32>) -> Option<usize> {
        let i = match row {
            None => 0,
            Some(row) => self.hunks.partition_point(|h| h.rows.start <= row),
        };
        (i < self.hunks.len()).then_some(i)
    }

    /// Last hunk starting before `row`.
    #[must_use]
    pub fn prev_hunk(&self, row: Option<u32>) -> Option<usize> {
        let row = row.unwrap_or(u32::MAX);
        self.hunks.partition_point(|h| h.rows.start < row).checked_sub(1)
    }

    /// Row ranges to print for a unified-style view: every hunk widened by
    /// `context` rows, overlapping ranges merged.
    #[must_use]
    pub fn context_groups(&self, context: u32) -> Vec<Range<u32>> {
        let total = self.rows.len() as u32;
        let mut groups: Vec<Range<u32>> = Vec::new();
        for h in &self.hunks {
            let start = h.rows.start.saturating_sub(context);
            let end = (h.rows.end + context).min(total);
            match groups.last_mut() {
                Some(last) if start <= last.end => last.end = end,
                _ => groups.push(start..end),
            }
        }
        groups
    }

    /// Writes a unified diff (`diff -u` format) with the given context.
    pub fn write_unified(&self, left: &Document, right: &Document, names: (&str, &str), context: u32, out: &mut String) {
        if self.is_equal() {
            return;
        }
        let _ = writeln!(out, "--- {}\n+++ {}", names.0, names.1);
        for group in self.context_groups(context) {
            let rows = &self.rows[group.start as usize..group.end as usize];
            let (l_start, l_count) = self.side_span(group.clone(), |r| r.left);
            let (r_start, r_count) = self.side_span(group, |r| r.right);
            let _ = writeln!(out, "@@ -{} +{} @@", range_label(l_start, l_count), range_label(r_start, r_count));
            let mut i = 0;
            while i < rows.len() {
                if rows[i].kind == RowKind::Equal {
                    if let Some(l) = rows[i].left {
                        push_line(out, ' ', left, l);
                    }
                    i += 1;
                    continue;
                }
                // Unified diffs list a whole changed block as removals, then additions.
                let block_end = rows[i..].iter().position(|r| r.kind == RowKind::Equal).map_or(rows.len(), |p| p + i);
                for row in &rows[i..block_end] {
                    if let Some(l) = row.left {
                        push_line(out, '-', left, l);
                    }
                }
                for row in &rows[i..block_end] {
                    if let Some(r) = row.right {
                        push_line(out, '+', right, r);
                    }
                }
                i = block_end;
            }
        }
    }

    /// Zero-based first line and count of one side within a row range.
    fn side_span(&self, rows: Range<u32>, side: impl Fn(&Row) -> Option<LineIdx>) -> (u32, u32) {
        let slice = &self.rows[rows.start as usize..rows.end as usize];
        let count = slice.iter().filter(|r| side(r).is_some()).count() as u32;
        let first = slice.iter().find_map(&side).map_or_else(
            || {
                // No line on this side: position is one past the previous line.
                self.rows[..rows.start as usize].iter().rev().find_map(&side).map_or(0, |l| l.0 + 1)
            },
            |l| l.0,
        );
        (first, count)
    }
}

fn range_label(first: u32, count: u32) -> String {
    let start = if count == 0 { first } else { first + 1 };
    if count == 1 { start.to_string() } else { format!("{start},{count}") }
}

fn push_line(out: &mut String, marker: char, doc: &Document, idx: LineIdx) {
    out.push(marker);
    out.push_str(doc.line(idx));
    out.push('\n');
    if doc.terminator(idx).is_empty() {
        out.push_str("\\ No newline at end of file\n");
    }
}

/// Interned line key; borrowed unless a comparison rule had to rewrite the line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
struct Key<'a>(Cow<'a, str>);

impl AsRef<[u8]> for Key<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

fn line_keys<'a>(doc: &'a Document, opts: &'a TextOptions) -> impl Iterator<Item = Key<'a>> + 'a {
    (0..doc.line_count() as u32).map(move |i| {
        let idx = LineIdx(i);
        let key = normalize(doc.line(idx), opts);
        if opts.ignore_line_endings { Key(key) } else { Key(Cow::Owned(format!("{key}{}", doc.terminator(idx)))) }
    })
}

/// Compares two documents line by line.
///
/// Histogram diff gives the most readable result but its cost grows with
/// `lines × hunks` (12 s for 1M lines with 10k scattered edits). Myers with
/// git's heuristics is near-linear. So Myers runs first; if the Histogram
/// cost estimate fits [`HISTOGRAM_BUDGET`], the diff is recomputed with
/// Histogram for nicer hunks. Either way the slider post-processing runs.
pub fn diff_documents(left: &Document, right: &Document, opts: &TextOptions, cancel: &CancelToken) -> Result<TextDiff, Cancelled> {
    cancel.check()?;
    if left.text() == right.text() {
        // Same text ⇒ same line split ⇒ every row is equal under any rule.
        let mut builder = Builder::with_capacity(left.line_count());
        builder.equal(0..left.line_count() as u32, 0);
        return Ok(builder.finish());
    }
    let mut input: InternedInput<Key<'_>> = InternedInput::default();
    input.reserve(left.line_count() as u32, right.line_count() as u32);
    input.update_before(line_keys(left, opts));
    input.update_after(line_keys(right, opts));
    cancel.check()?;

    let mut diff = Diff::compute(Algorithm::Myers, &input);
    let lines = (input.before.len() + input.after.len()) as u64;
    let hunks = diff.hunks().count() as u64;
    if lines.saturating_mul(hunks + 1) <= HISTOGRAM_BUDGET {
        cancel.check()?;
        diff = Diff::compute(Algorithm::Histogram, &input);
    }
    diff.postprocess_lines(&input);
    cancel.check()?;

    let mut builder = Builder::with_capacity(left.line_count().max(right.line_count()));
    let (mut l, mut r) = (0u32, 0u32);
    for (n, hunk) in diff.hunks().enumerate() {
        if n % 256 == 0 {
            cancel.check()?;
        }
        builder.equal(l..hunk.before.start, r);
        builder.change(hunk.before.clone(), hunk.after.clone(), left, right, opts);
        l = hunk.before.end;
        r = hunk.after.end;
    }
    builder.equal(l..left.line_count() as u32, r);
    Ok(builder.finish())
}

struct Builder {
    diff: TextDiff,
}

impl Builder {
    fn with_capacity(n: usize) -> Self {
        Self { diff: TextDiff { rows: Vec::with_capacity(n), ..TextDiff::default() } }
    }

    fn equal(&mut self, left: Range<u32>, right_start: u32) {
        let n = left.len() as u32;
        self.diff.rows.extend((0..n).map(|i| Row {
            kind: RowKind::Equal,
            left: Some(LineIdx(left.start + i)),
            right: Some(LineIdx(right_start + i)),
        }));
        self.diff.stats.unchanged += n;
    }

    fn change(&mut self, before: Range<u32>, after: Range<u32>, left: &Document, right: &Document, opts: &TextOptions) {
        let first_row = self.diff.rows.len() as u32;
        let l_lines: Vec<&str> = before.clone().map(|i| left.line(LineIdx(i))).collect();
        let r_lines: Vec<&str> = after.clone().map(|i| right.line(LineIdx(i))).collect();
        for (l, r) in align(&l_lines, &r_lines, opts) {
            let left_idx = l.map(|i| LineIdx(before.start + i as u32));
            let right_idx = r.map(|i| LineIdx(after.start + i as u32));
            let kind = match (left_idx, right_idx) {
                (Some(_), Some(_)) => {
                    self.diff.stats.modified += 1;
                    RowKind::Modify
                }
                (Some(_), None) => {
                    self.diff.stats.removed += 1;
                    RowKind::Delete
                }
                (None, Some(_)) => {
                    self.diff.stats.added += 1;
                    RowKind::Insert
                }
                (None, None) => continue,
            };
            self.diff.rows.push(Row { kind, left: left_idx, right: right_idx });
        }
        let last_row = self.diff.rows.len() as u32;
        self.diff.hunks.push(Hunk { rows: first_row..last_row, left: before, right: after });
        self.diff.stats.hunks += 1;
    }

    fn finish(self) -> TextDiff {
        self.diff
    }
}

type Pairing = Vec<(Option<usize>, Option<usize>)>;

/// Pairs lines of a changed block so similar lines sit on the same row.
fn align(left: &[&str], right: &[&str], opts: &TextOptions) -> Pairing {
    let (n, m) = (left.len(), right.len());
    if n == 0 || m == 0 || n == m || n * m > ALIGN_CELL_LIMIT {
        return positional(n, m);
    }
    let lb: Vec<Vec<u64>> = left.iter().map(|l| bigrams(&normalize(l, opts))).collect();
    let rb: Vec<Vec<u64>> = right.iter().map(|r| bigrams(&normalize(r, opts))).collect();

    // Order-preserving maximum-similarity matching (weighted LCS).
    let width = m + 1;
    let mut score = vec![0f32; (n + 1) * width];
    for i in 1..=n {
        for j in 1..=m {
            let skip = score[(i - 1) * width + j].max(score[i * width + j - 1]);
            let s = dice(&lb[i - 1], &rb[j - 1]);
            let take = if s >= PAIR_THRESHOLD { score[(i - 1) * width + j - 1] + s } else { f32::MIN };
            score[i * width + j] = skip.max(take);
        }
    }

    let mut pairs = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let here = score[i * width + j];
        if (here - score[(i - 1) * width + j]).abs() < f32::EPSILON {
            i -= 1;
        } else if (here - score[i * width + j - 1]).abs() < f32::EPSILON {
            j -= 1;
        } else {
            pairs.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        }
    }
    pairs.reverse();

    let mut out = Vec::with_capacity(n.max(m));
    let (mut li, mut ri) = (0, 0);
    for (pl, pr) in pairs.into_iter().chain(std::iter::once((n, m))) {
        out.extend((li..pl).map(|l| (Some(l), None)));
        out.extend((ri..pr).map(|r| (None, Some(r))));
        if pl < n && pr < m {
            out.push((Some(pl), Some(pr)));
        }
        li = pl + 1;
        ri = pr + 1;
    }
    out
}

fn positional(n: usize, m: usize) -> Pairing {
    (0..n.max(m)).map(|i| ((i < n).then_some(i), (i < m).then_some(i))).collect()
}

/// Sorted character bigrams of a trimmed line.
fn bigrams(line: &str) -> Vec<u64> {
    let chars: Vec<char> = line.trim().chars().collect();
    if chars.len() < 2 {
        return chars.iter().map(|&c| u64::from(c)).collect();
    }
    let mut v: Vec<u64> = chars.windows(2).map(|w| (u64::from(w[0]) << 32) | u64::from(w[1])).collect();
    v.sort_unstable();
    v
}

/// Sørensen–Dice coefficient of two sorted multisets.
fn dice(a: &[u64], b: &[u64]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (mut i, mut j, mut common) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    (2 * common) as f32 / (a.len() + b.len()) as f32
}

/// Byte ranges that differ inside a modified line pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineDiff {
    pub left: Vec<Range<usize>>,
    pub right: Vec<Range<usize>>,
}

/// Word-level differences between two lines, honoring the comparison rules
/// (case-only or whitespace-only changes are not highlighted when ignored).
#[must_use]
pub fn inline_diff(left: &str, right: &str, opts: &TextOptions) -> InlineDiff {
    let lt = word_tokens(left);
    let rt = word_tokens(right);
    if lt.len() > INLINE_TOKEN_LIMIT || rt.len() > INLINE_TOKEN_LIMIT {
        let whole = |s: &str| std::iter::once(0..s.len()).filter(|r| !r.is_empty()).collect();
        return InlineDiff { left: whole(left), right: whole(right) };
    }
    let fold = |s: &str| -> String { if opts.ignore_case { s.to_lowercase() } else { s.to_owned() } };
    let mut input: InternedInput<String> = InternedInput::default();
    input.update_before(lt.iter().map(|r| fold(&left[r.clone()])));
    input.update_after(rt.iter().map(|r| fold(&right[r.clone()])));
    let mut diff = Diff::compute(Algorithm::Myers, &input);
    diff.postprocess_no_heuristic(&input);

    let ignore_ws = opts.whitespace != WhitespaceMode::Exact;
    let is_blank = |s: &str, toks: &[Range<usize>], r: Range<u32>| {
        toks[r.start as usize..r.end as usize].iter().all(|t| s[t.clone()].trim().is_empty())
    };
    let mut out = InlineDiff::default();
    for h in diff.hunks() {
        if ignore_ws && is_blank(left, &lt, h.before.clone()) && is_blank(right, &rt, h.after.clone()) {
            continue;
        }
        push_span(&mut out.left, &lt, h.before);
        push_span(&mut out.right, &rt, h.after);
    }
    out
}

fn push_span(spans: &mut Vec<Range<usize>>, tokens: &[Range<usize>], range: Range<u32>) {
    if range.is_empty() {
        return;
    }
    let span = tokens[range.start as usize].start..tokens[range.end as usize - 1].end;
    match spans.last_mut() {
        Some(last) if last.end == span.start => last.end = span.end,
        _ => spans.push(span),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> Document {
        Document::from_text(s.to_owned())
    }

    fn run(a: &str, b: &str, opts: &TextOptions) -> TextDiff {
        diff_documents(&doc(a), &doc(b), opts, &CancelToken::new()).expect("not cancelled")
    }

    fn kinds(d: &TextDiff) -> String {
        d.rows
            .iter()
            .map(|r| match r.kind {
                RowKind::Equal => '=',
                RowKind::Delete => '-',
                RowKind::Insert => '+',
                RowKind::Modify => '~',
            })
            .collect()
    }

    #[test]
    fn identical_text_has_no_hunks() {
        let d = run("a\nb\n", "a\nb\n", &TextOptions::default());
        assert!(d.is_equal());
        assert_eq!(kinds(&d), "==");
    }

    #[test]
    fn single_modified_line_is_one_row() {
        let d = run("a\nb\nc\n", "a\nB!\nc\n", &TextOptions::default());
        assert_eq!(kinds(&d), "=~=");
        assert_eq!(d.stats, DiffStats { hunks: 1, added: 0, removed: 0, modified: 1, unchanged: 2 });
    }

    #[test]
    fn similar_lines_are_paired_inside_uneven_blocks() {
        let a = "fn main() {\n    let total = compute(1);\n}\n";
        let b = "fn main() {\n    log(\"start\");\n    let total = compute(2);\n    log(\"end\");\n}\n";
        let d = run(a, b, &TextOptions::default());
        assert_eq!(kinds(&d), "=+~+=");
    }

    #[test]
    fn whitespace_and_case_rules_hide_differences() {
        let opts = TextOptions { whitespace: WhitespaceMode::All, ignore_case: true, ..TextOptions::default() };
        assert!(run("Hello  World\n", "hello world\n", &opts).is_equal());
        assert!(!run("Hello  World\n", "hello world\n", &TextOptions::default()).is_equal());
    }

    #[test]
    fn line_endings_matter_only_when_asked() {
        assert!(run("a\r\nb\r\n", "a\nb\n", &TextOptions::default()).is_equal());
        let strict = TextOptions { ignore_line_endings: false, ..TextOptions::default() };
        assert_eq!(run("a\r\nb\n", "a\nb\n", &strict).stats.modified, 1);
    }

    #[test]
    fn hunk_navigation() {
        let d = run("1\n2\n3\n4\n5\n6\n", "1\nX\n3\n4\nY\n6\n", &TextOptions::default());
        assert_eq!(d.hunks.len(), 2);
        assert_eq!(d.next_hunk(None), Some(0));
        assert_eq!(d.next_hunk(Some(1)), Some(1));
        assert_eq!(d.next_hunk(Some(4)), None);
        assert_eq!(d.prev_hunk(Some(4)), Some(0));
        assert_eq!(d.prev_hunk(None), Some(1));
        assert_eq!(d.hunk_at_row(4), Some(1));
        assert_eq!(d.hunk_at_row(2), None);
    }

    #[test]
    fn unified_output_matches_diff_u() {
        let a = doc("a\nb\nc\nd\n");
        let b = doc("a\nB\nc\nd\ne");
        let d = diff_documents(&a, &b, &TextOptions::default(), &CancelToken::new()).expect("ok");
        let mut out = String::new();
        d.write_unified(&a, &b, ("a.txt", "b.txt"), 1, &mut out);
        assert_eq!(out, "--- a.txt\n+++ b.txt\n@@ -1,4 +1,5 @@\n a\n-b\n+B\n c\n d\n+e\n\\ No newline at end of file\n");
    }

    #[test]
    fn inline_diff_marks_changed_words() {
        let opts = TextOptions::default();
        let l = "let total = compute(1);";
        let r = "let sum = compute(2);";
        let d = inline_diff(l, r, &opts);
        let words = |s: &str, v: &[Range<usize>]| v.iter().map(|x| s[x.clone()].to_owned()).collect::<Vec<_>>();
        assert_eq!(words(l, &d.left), ["total", "1"]);
        assert_eq!(words(r, &d.right), ["sum", "2"]);
    }

    #[test]
    fn inline_diff_respects_ignore_rules() {
        let opts = TextOptions { whitespace: WhitespaceMode::Change, ignore_case: true, ..TextOptions::default() };
        let d = inline_diff("Let  x = 1", "let x = 2", &opts);
        assert_eq!(d.left, vec![9..10]);
    }

    #[test]
    fn cancellation_is_honored() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(diff_documents(&doc("a"), &doc("b"), &TextOptions::default(), &cancel).is_err());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        /// Every line of each side appears exactly once, in order, and equal rows really are equal.
        #[test]
        fn rows_cover_both_sides_in_order(
            a in proptest::collection::vec("[abc]{0,3}", 0..30),
            b in proptest::collection::vec("[abc]{0,3}", 0..30),
        ) {
            let (a, b) = (a.join("\n"), b.join("\n"));
            let (da, db) = (doc(&a), doc(&b));
            let d = diff_documents(&da, &db, &TextOptions::default(), &CancelToken::new()).expect("ok");
            let left: Vec<u32> = d.rows.iter().filter_map(|r| r.left.map(|l| l.0)).collect();
            let right: Vec<u32> = d.rows.iter().filter_map(|r| r.right.map(|l| l.0)).collect();
            proptest::prop_assert_eq!(left, (0..da.line_count() as u32).collect::<Vec<_>>());
            proptest::prop_assert_eq!(right, (0..db.line_count() as u32).collect::<Vec<_>>());
            for row in d.rows.iter().filter(|r| r.kind == RowKind::Equal) {
                let (l, r) = (row.left.expect("equal has left"), row.right.expect("equal has right"));
                proptest::prop_assert_eq!(da.line(l), db.line(r));
            }
            proptest::prop_assert_eq!(d.is_equal(), a.lines().eq(b.lines()));
        }

        /// Hunks partition the non-equal rows exactly.
        #[test]
        fn hunks_partition_changed_rows(
            a in proptest::collection::vec("[ab]{0,2}", 0..20),
            b in proptest::collection::vec("[ab]{0,2}", 0..20),
        ) {
            let d = diff_documents(&doc(&a.join("\n")), &doc(&b.join("\n")), &TextOptions::default(), &CancelToken::new()).expect("ok");
            for (i, row) in d.rows.iter().enumerate() {
                proptest::prop_assert_eq!(row.kind != RowKind::Equal, d.hunk_at_row(i as u32).is_some());
            }
        }
    }
}
