//! Loading inputs and dispatching to the right engine.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use difflux_core::{CancelToken, Cancelled, CompareOptions, Progress, Side, TextOptions};
use difflux_diff_dir::FolderDiff;
use difflux_diff_text::{TextDiff, diff_documents};
use difflux_text::{Content, Document, decode};

/// Why a comparison could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum CompareError {
    #[error("{0} does not exist")]
    Missing(PathBuf),
    #[error("cannot compare a file with a folder ({left} vs {right})")]
    Mismatch { left: PathBuf, right: PathBuf },
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Folder(#[from] difflux_diff_dir::Error),
    #[error("cancelled")]
    Cancelled,
}

impl From<Cancelled> for CompareError {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

/// Overall verdict for a file pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileVerdict {
    /// Byte-for-byte identical.
    Identical,
    /// Bytes differ, but only in ways the comparison rules ignore (whitespace, case, line endings).
    Equivalent,
    /// Text differs.
    Different,
    /// At least one side is binary and the bytes differ.
    BinaryDifferent,
}

impl FileVerdict {
    #[must_use]
    pub fn is_difference(self) -> bool {
        matches!(self, Self::Different | Self::BinaryDifferent)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Identical => "Identical",
            Self::Equivalent => "Only ignored differences",
            Self::Different => "Different",
            Self::BinaryDifferent => "Binary files differ",
        }
    }
}

/// One loaded side of a file comparison.
#[derive(Debug, Clone)]
pub struct SideContent {
    /// Path, or a label such as `left` for in-memory text.
    pub name: String,
    pub content: Content,
}

impl SideContent {
    #[must_use]
    pub fn document(&self) -> Option<&Document> {
        match &self.content {
            Content::Text(d) => Some(d),
            Content::Binary { .. } => None,
        }
    }

    #[must_use]
    pub fn byte_len(&self) -> u64 {
        match &self.content {
            Content::Text(d) => d.byte_len,
            Content::Binary { byte_len } => *byte_len,
        }
    }
}

/// A finished file comparison.
#[derive(Debug, Clone)]
pub struct FileCompare {
    pub left: SideContent,
    pub right: SideContent,
    /// `None` when either side is binary.
    pub diff: Option<TextDiff>,
    pub verdict: FileVerdict,
    pub options: TextOptions,
    pub elapsed: Duration,
}

/// Result of [`compare_paths`].
#[derive(Debug, Clone)]
pub enum Comparison {
    Files(Box<FileCompare>),
    Folders(Box<FolderDiff>),
}

impl Comparison {
    /// True when the two sides differ in a way the user cares about.
    #[must_use]
    pub fn is_difference(&self) -> bool {
        match self {
            Self::Files(f) => f.verdict.is_difference(),
            Self::Folders(d) => !d.is_equal(),
        }
    }
}

/// Compares two paths, choosing the file or folder engine automatically.
pub fn compare_paths(
    left: &Path,
    right: &Path,
    opts: &CompareOptions,
    cancel: &CancelToken,
    progress: &Progress,
) -> Result<Comparison, CompareError> {
    let l = fs::metadata(left).map_err(|e| missing_or_io(left, e))?;
    let r = fs::metadata(right).map_err(|e| missing_or_io(right, e))?;
    match (l.is_dir(), r.is_dir()) {
        (true, true) => Ok(Comparison::Folders(Box::new(difflux_diff_dir::compare_folders(left, right, &opts.folder, cancel, progress)?))),
        (false, false) => Ok(Comparison::Files(Box::new(compare_files(left, right, &opts.text, cancel)?))),
        _ => Err(CompareError::Mismatch { left: left.to_path_buf(), right: right.to_path_buf() }),
    }
}

fn missing_or_io(path: &Path, e: io::Error) -> CompareError {
    if e.kind() == io::ErrorKind::NotFound {
        CompareError::Missing(path.to_path_buf())
    } else {
        CompareError::Io { path: path.to_path_buf(), source: e }
    }
}

/// Reads and compares two files.
pub fn compare_files(left: &Path, right: &Path, opts: &TextOptions, cancel: &CancelToken) -> Result<FileCompare, CompareError> {
    let started = Instant::now();
    let read = |p: &Path| fs::read(p).map_err(|e| missing_or_io(p, e));
    let (lb, rb) = (read(left)?, read(right)?);
    cancel.check()?;
    let identical = lb == rb;
    let left = SideContent { name: left.display().to_string(), content: decode(&lb) };
    let right = SideContent { name: right.display().to_string(), content: decode(&rb) };
    drop((lb, rb));
    finish(left, right, identical, opts, cancel, started)
}

/// Compares a file that exists on only one side against nothing, so its whole
/// content shows as added (right side) or removed (left side).
pub fn compare_one_sided(path: &Path, side: Side, opts: &TextOptions, cancel: &CancelToken) -> Result<FileCompare, CompareError> {
    let started = Instant::now();
    let bytes = fs::read(path).map_err(|e| missing_or_io(path, e))?;
    cancel.check()?;
    let present = SideContent { name: path.display().to_string(), content: decode(&bytes) };
    let missing = SideContent { name: "(not present)".into(), content: Content::Text(Document::from_text(String::new())) };
    let identical = bytes.is_empty();
    let (left, right) = match side {
        Side::Left => (present, missing),
        Side::Right => (missing, present),
    };
    finish(left, right, identical, opts, cancel, started)
}

/// Compares two in-memory texts (used by the MCP server and tests).
pub fn compare_texts(left: String, right: String, opts: &TextOptions, cancel: &CancelToken) -> Result<FileCompare, CompareError> {
    let started = Instant::now();
    let identical = left == right;
    let left = SideContent { name: "left".into(), content: Content::Text(Document::from_text(left)) };
    let right = SideContent { name: "right".into(), content: Content::Text(Document::from_text(right)) };
    finish(left, right, identical, opts, cancel, started)
}

fn finish(
    left: SideContent,
    right: SideContent,
    identical: bool,
    opts: &TextOptions,
    cancel: &CancelToken,
    started: Instant,
) -> Result<FileCompare, CompareError> {
    let diff = match (left.document(), right.document()) {
        (Some(l), Some(r)) => Some(diff_documents(l, r, opts, cancel)?),
        _ => None,
    };
    let verdict = match (&diff, identical) {
        (_, true) => FileVerdict::Identical,
        (None, false) => FileVerdict::BinaryDifferent,
        (Some(d), false) if d.is_equal() => FileVerdict::Equivalent,
        (Some(_), false) => FileVerdict::Different,
    };
    Ok(FileCompare { left, right, diff, verdict, options: opts.clone(), elapsed: started.elapsed() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use difflux_core::WhitespaceMode;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn verdicts() -> TestResult {
        let c = CancelToken::new();
        let opts = TextOptions::default();
        assert_eq!(compare_texts("a".into(), "a".into(), &opts, &c)?.verdict, FileVerdict::Identical);
        assert_eq!(compare_texts("a\r\n".into(), "a\n".into(), &opts, &c)?.verdict, FileVerdict::Equivalent);
        assert_eq!(compare_texts("a".into(), "b".into(), &opts, &c)?.verdict, FileVerdict::Different);
        let ws = TextOptions { whitespace: WhitespaceMode::All, ..TextOptions::default() };
        assert_eq!(compare_texts("a b".into(), "ab".into(), &ws, &c)?.verdict, FileVerdict::Equivalent);
        Ok(())
    }

    #[test]
    fn one_sided_files_show_everything_as_added_or_removed() -> TestResult {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("new.txt");
        fs::write(&path, "one\ntwo\n")?;
        let c = compare_one_sided(&path, Side::Right, &TextOptions::default(), &CancelToken::new())?;
        assert_eq!(c.verdict, FileVerdict::Different);
        let stats = c.diff.as_ref().map(|d| d.stats).ok_or("no diff")?;
        assert_eq!((stats.added, stats.removed), (2, 0));
        assert_eq!(c.left.name, "(not present)");
        Ok(())
    }

    #[test]
    fn dispatches_on_path_kind() -> TestResult {
        let dir = tempfile::tempdir()?;
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&a, "x\n")?;
        fs::write(&b, [0u8, 1, 2])?;
        let opts = CompareOptions::default();
        let (cancel, progress) = (CancelToken::new(), Progress::new());
        match compare_paths(&a, &b, &opts, &cancel, &progress)? {
            Comparison::Files(f) => assert_eq!(f.verdict, FileVerdict::BinaryDifferent),
            Comparison::Folders(_) => return Err("expected files".into()),
        }
        assert!(matches!(compare_paths(dir.path(), dir.path(), &opts, &cancel, &progress)?, Comparison::Folders(_)));
        assert!(matches!(compare_paths(&a, dir.path(), &opts, &cancel, &progress), Err(CompareError::Mismatch { .. })));
        assert!(matches!(compare_paths(&a, &dir.path().join("nope"), &opts, &cancel, &progress), Err(CompareError::Missing(_))));
        Ok(())
    }
}
