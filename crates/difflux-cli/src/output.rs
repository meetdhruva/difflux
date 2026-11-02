//! Machine-readable output (`--json`, MCP). The shape is versioned by
//! [`difflux_core::SCHEMA_ID`]; `difflux schema` prints the JSON Schema.

use difflux_app::{FileCompare, FileVerdict};
use difflux_core::{FolderOptions, LineIdx, SCHEMA_ID, TextOptions};
use difflux_diff_dir::{Basis, EntryKind, EntryMeta, FolderDiff, FolderStats, NodeId, Status};
use difflux_diff_text::{DiffStats, RowKind};
use difflux_text::{Content, LineEnding};
use schemars::JsonSchema;
use serde::Serialize;

/// Top-level document for every JSON output.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    /// Always `difflux/1` for this version of the format.
    pub schema: String,
    #[serde(flatten)]
    pub report: Report,
}

impl Output {
    #[must_use]
    pub fn new(report: Report) -> Self {
        Self { schema: SCHEMA_ID.to_owned(), report }
    }
}

/// What was compared, and the result.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Report {
    Files(FilesReport),
    Folders(FoldersReport),
    Error(ErrorReport),
}

/// The comparison could not be performed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ErrorReport {
    pub message: String,
}

/// Verdict for a pair of files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// Byte-for-byte identical.
    Identical,
    /// Bytes differ only in ways the comparison rules ignore.
    Equivalent,
    Different,
    BinaryDifferent,
}

impl From<FileVerdict> for FileStatus {
    fn from(v: FileVerdict) -> Self {
        match v {
            FileVerdict::Identical => Self::Identical,
            FileVerdict::Equivalent => Self::Equivalent,
            FileVerdict::Different => Self::Different,
            FileVerdict::BinaryDifferent => Self::BinaryDifferent,
        }
    }
}

/// Facts about one input file.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FileInfo {
    pub path: String,
    pub bytes: u64,
    pub binary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_ending: Option<LineEnding>,
    pub bom: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<u64>,
}

/// Result of comparing two files.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FilesReport {
    pub status: FileStatus,
    pub left: FileInfo,
    pub right: FileInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<DiffStats>,
    pub options: TextOptions,
    /// Changed regions with surrounding context, like a unified diff.
    pub hunks: Vec<JsonHunk>,
    /// True when `max_lines` cut the hunk list short.
    pub truncated: bool,
    pub elapsed_ms: u64,
}

/// One changed region.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JsonHunk {
    /// First left line (1-based; 0 when the hunk has no left lines at the start of the file).
    pub left_start: u32,
    pub left_count: u32,
    pub right_start: u32,
    pub right_count: u32,
    pub lines: Vec<JsonLine>,
}

/// Role of a line inside a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineOp {
    Context,
    Removed,
    Added,
}

/// A line inside a hunk.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JsonLine {
    pub op: LineOp,
    /// 1-based left line number, when the line exists on the left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<u32>,
    pub text: String,
}

/// Overall verdict for two folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FolderStatus {
    Identical,
    Different,
}

/// Result of comparing two folders.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FoldersReport {
    pub status: FolderStatus,
    pub left: String,
    pub right: String,
    pub stats: FolderStats,
    pub options: FolderOptions,
    /// Entries, depth-first; identical entries only with `include_identical`.
    pub entries: Vec<JsonEntry>,
    /// True when `max_entries` cut the list short.
    pub truncated: bool,
    pub elapsed_ms: u64,
}

/// One file or folder in a folder comparison.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JsonEntry {
    /// Path relative to both roots, `/`-separated.
    pub path: String,
    pub kind: EntryKind,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<Basis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<EntryMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<EntryMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Limits that keep outputs manageable for humans and agents.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub context: u32,
    pub max_lines: usize,
    pub max_entries: usize,
    pub include_identical: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self { context: 3, max_lines: usize::MAX, max_entries: usize::MAX, include_identical: false }
    }
}

fn file_info(name: &str, content: &Content) -> FileInfo {
    match content {
        Content::Text(d) => FileInfo {
            path: name.to_owned(),
            bytes: d.byte_len,
            binary: false,
            encoding: Some(d.encoding.to_owned()),
            line_ending: Some(d.line_ending),
            bom: d.has_bom,
            lines: Some(d.line_count() as u64),
        },
        Content::Binary { byte_len } => {
            FileInfo { path: name.to_owned(), bytes: *byte_len, binary: true, encoding: None, line_ending: None, bom: false, lines: None }
        }
    }
}

/// Builds the JSON report for a file comparison.
#[must_use]
pub fn files_report(c: &FileCompare, limits: Limits) -> FilesReport {
    let mut hunks = Vec::new();
    let mut truncated = false;
    if let (Some(diff), Some(l), Some(r)) = (&c.diff, c.left.document(), c.right.document()) {
        let mut budget = limits.max_lines;
        'groups: for group in diff.context_groups(limits.context) {
            let rows = &diff.rows[group.start as usize..group.end as usize];
            let mut hunk = JsonHunk { left_start: 0, left_count: 0, right_start: 0, right_count: 0, lines: Vec::new() };
            let mut i = 0;
            while i < rows.len() {
                let end = if rows[i].kind == RowKind::Equal {
                    i + 1
                } else {
                    rows[i..].iter().position(|r| r.kind == RowKind::Equal).map_or(rows.len(), |p| p + i)
                };
                let block = &rows[i..end];
                let mut lines: Vec<JsonLine> = Vec::new();
                if rows[i].kind == RowKind::Equal {
                    let (li, ri) = (rows[i].left, rows[i].right);
                    if let (Some(li), Some(ri)) = (li, ri) {
                        lines.push(JsonLine {
                            op: LineOp::Context,
                            left: Some(li.number()),
                            right: Some(ri.number()),
                            text: l.line(li).to_owned(),
                        });
                    }
                } else {
                    lines.extend(block.iter().filter_map(|row| row.left).map(|li: LineIdx| JsonLine {
                        op: LineOp::Removed,
                        left: Some(li.number()),
                        right: None,
                        text: l.line(li).to_owned(),
                    }));
                    lines.extend(block.iter().filter_map(|row| row.right).map(|ri: LineIdx| JsonLine {
                        op: LineOp::Added,
                        left: None,
                        right: Some(ri.number()),
                        text: r.line(ri).to_owned(),
                    }));
                }
                for line in lines {
                    if budget == 0 {
                        truncated = true;
                        finish_hunk(&mut hunk);
                        if !hunk.lines.is_empty() {
                            hunks.push(hunk);
                        }
                        break 'groups;
                    }
                    budget -= 1;
                    hunk.lines.push(line);
                }
                i = end;
            }
            finish_hunk(&mut hunk);
            hunks.push(hunk);
        }
        // Fill start positions for hunks that have no lines on one side.
        for (h, group) in hunks.iter_mut().zip(diff.context_groups(limits.context)) {
            if h.left_count == 0 {
                h.left_start = position_before(diff, group.start, |r| r.left);
            }
            if h.right_count == 0 {
                h.right_start = position_before(diff, group.start, |r| r.right);
            }
        }
    }
    FilesReport {
        status: c.verdict.into(),
        left: file_info(&c.left.name, &c.left.content),
        right: file_info(&c.right.name, &c.right.content),
        stats: c.diff.as_ref().map(|d| d.stats),
        options: c.options.clone(),
        hunks,
        truncated,
        elapsed_ms: c.elapsed.as_millis() as u64,
    }
}

fn finish_hunk(h: &mut JsonHunk) {
    let first_left = h.lines.iter().find_map(|l| l.left);
    let first_right = h.lines.iter().find_map(|l| l.right);
    h.left_count = h.lines.iter().filter(|l| l.left.is_some()).count() as u32;
    h.right_count = h.lines.iter().filter(|l| l.right.is_some()).count() as u32;
    h.left_start = first_left.unwrap_or(0);
    h.right_start = first_right.unwrap_or(0);
}

fn position_before(diff: &difflux_diff_text::TextDiff, row: u32, side: impl Fn(&difflux_diff_text::Row) -> Option<LineIdx>) -> u32 {
    diff.rows[..row as usize].iter().rev().find_map(side).map_or(0, LineIdx::number)
}

/// Builds the JSON report for a folder comparison.
#[must_use]
pub fn folders_report(d: &FolderDiff, options: &FolderOptions, limits: Limits) -> FoldersReport {
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut stack: Vec<NodeId> = d.children(FolderDiff::ROOT).rev().collect();
    while let Some(id) = stack.pop() {
        let n = d.node(id);
        let show = limits.include_identical || n.status.is_difference();
        if show {
            if entries.len() >= limits.max_entries {
                truncated = true;
                break;
            }
            entries.push(JsonEntry {
                path: d.rel_path(id),
                kind: n.kind(),
                status: n.status,
                basis: n.basis,
                left: n.left.clone(),
                right: n.right.clone(),
                error: n.error.clone(),
            });
        }
        // One-sided folders are reported once; their contents add noise, not information.
        let one_sided = matches!(n.status, Status::LeftOnly | Status::RightOnly);
        if !one_sided || limits.include_identical {
            stack.extend(n.children.clone().rev());
        }
    }
    FoldersReport {
        status: if d.is_equal() { FolderStatus::Identical } else { FolderStatus::Different },
        left: d.left_root.display().to_string(),
        right: d.right_root.display().to_string(),
        stats: d.stats,
        options: options.clone(),
        entries,
        truncated,
        elapsed_ms: d.elapsed.as_millis() as u64,
    }
}

/// JSON Schema for [`Output`].
#[must_use]
pub fn schema() -> schemars::Schema {
    schemars::schema_for!(Output)
}
