//! Shared vocabulary for every difflux engine.
//!
//! Rules for this crate: no I/O, no globals, no UI types. Everything here must
//! be cheap to construct in a unit test.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Identifier embedded in every machine-readable output. Bump the number on
/// any breaking change to the JSON shape; additive fields keep the version.
pub const SCHEMA_ID: &str = "difflux/1";

/// One side of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Zero-based line index into a document. Display code converts with
/// [`LineIdx::number`]; mixing the two is the classic diff-viewer bug, hence the newtype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LineIdx(pub u32);

impl LineIdx {
    /// One-based line number for humans and JSON output.
    #[must_use]
    pub fn number(self) -> u32 {
        self.0 + 1
    }

    #[must_use]
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for LineIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.number())
    }
}

/// How whitespace differences are treated when comparing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WhitespaceMode {
    /// Every byte matters.
    #[default]
    Exact,
    /// Ignore whitespace at the end of lines.
    Trailing,
    /// Treat runs of whitespace as a single space and ignore trailing whitespace (like `diff -b`).
    Change,
    /// Ignore all whitespace (like `diff -w`).
    All,
}

/// Rules for comparing text. Rules only affect *matching*; displayed text is never altered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TextOptions {
    pub whitespace: WhitespaceMode,
    pub ignore_case: bool,
    /// Treat CRLF, LF and CR line endings as equal.
    pub ignore_line_endings: bool,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self { whitespace: WhitespaceMode::Exact, ignore_case: false, ignore_line_endings: true }
    }
}

impl TextOptions {
    /// True when lines can be compared byte-for-byte without normalization.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.whitespace == WhitespaceMode::Exact && !self.ignore_case
    }
}

/// How file contents are compared during a folder compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentMode {
    /// Equal size and modification time count as identical; otherwise compare bytes.
    #[default]
    Smart,
    /// Always compare bytes when sizes match. Slowest, never wrong.
    Exact,
    /// Never read file contents; decide on size and modification time only.
    Metadata,
}

/// How entry names are matched between the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NameMatching {
    /// Case-insensitive on Windows and macOS, case-sensitive elsewhere.
    #[default]
    Auto,
    CaseSensitive,
    CaseInsensitive,
}

impl NameMatching {
    #[must_use]
    pub fn is_case_insensitive(self) -> bool {
        match self {
            Self::Auto => cfg!(any(windows, target_os = "macos")),
            Self::CaseSensitive => false,
            Self::CaseInsensitive => true,
        }
    }
}

/// Rules for comparing folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FolderOptions {
    pub content: ContentMode,
    pub names: NameMatching,
    /// Follow symbolic links instead of comparing the link targets.
    pub follow_symlinks: bool,
    /// Glob patterns (matched against the entry name and its relative path) to skip.
    pub excludes: Vec<String>,
    /// Modification times closer than this count as equal (FAT and SMB round timestamps).
    pub mtime_tolerance_ms: u64,
    /// Worker threads; `None` uses one per CPU. Lower it for spinning disks and network shares.
    pub threads: Option<usize>,
}

impl Default for FolderOptions {
    fn default() -> Self {
        Self {
            content: ContentMode::Smart,
            names: NameMatching::Auto,
            follow_symlinks: false,
            excludes: Vec::new(),
            mtime_tolerance_ms: 0,
            threads: None,
        }
    }
}

/// Everything a single compare needs to know.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CompareOptions {
    pub text: TextOptions,
    pub folder: FolderOptions,
}

/// Cooperative cancellation shared between a job and whoever started it.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Returns `Err(Cancelled)` once cancellation was requested.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() { Err(Cancelled) } else { Ok(()) }
    }
}

/// The operation was cancelled by the user or superseded by a newer request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("operation cancelled")]
pub struct Cancelled;

/// Lock-free progress counters a UI can poll without blocking workers.
#[derive(Debug, Clone, Default)]
pub struct Progress(Arc<ProgressInner>);

#[derive(Debug, Default)]
struct ProgressInner {
    items: AtomicU64,
    bytes: AtomicU64,
}

impl Progress {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_items(&self, n: u64) {
        self.0.items.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_bytes(&self, n: u64) {
        self.0.bytes.fetch_add(n, Ordering::Relaxed);
    }

    #[must_use]
    pub fn items(&self) -> u64 {
        self.0.items.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.0.bytes.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_are_one_based() {
        assert_eq!(LineIdx(0).number(), 1);
        assert_eq!(LineIdx(41).to_string(), "42");
    }

    #[test]
    fn cancel_token_is_shared_between_clones() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(b.check().is_ok());
        a.cancel();
        assert_eq!(b.check(), Err(Cancelled));
    }

    #[test]
    fn options_load_from_partial_settings() -> Result<(), serde_json::Error> {
        // Settings written by an older version, or by hand, must still load.
        let opts: CompareOptions = serde_json::from_str(r#"{"text":{"ignore_case":true}}"#)?;
        assert!(opts.text.ignore_case);
        assert!(opts.text.ignore_line_endings);
        assert_eq!(opts.folder.content, ContentMode::Smart);
        Ok(())
    }
}
