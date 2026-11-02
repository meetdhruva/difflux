//! Command-line front end: `difflux diff`, `difflux schema`, `difflux mcp`.
//!
//! Exit codes follow `diff(1)`: 0 = no differences, 1 = differences, 2 = trouble.
//! Every report is available as JSON with a versioned schema, so scripts and
//! AI agents never have to scrape human output.

pub mod mcp;
pub mod output;

use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use difflux_app::{CompareError, Comparison, FileCompare, FileVerdict, compare_paths};
use difflux_core::{CancelToken, CompareOptions, ContentMode, FolderOptions, Progress, TextOptions, WhitespaceMode};
use difflux_diff_dir::{FolderDiff, NodeId, Status};

pub use output::{Limits, Output, Report};

/// No differences.
pub const EXIT_SAME: u8 = 0;
/// Differences found.
pub const EXIT_DIFFERENT: u8 = 1;
/// Something went wrong.
pub const EXIT_TROUBLE: u8 = 2;

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Unified diff for files, a status list for folders.
    Text,
    /// Versioned JSON (see `difflux schema`).
    Json,
}

/// Whitespace handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WhitespaceArg {
    Exact,
    Trailing,
    Change,
    All,
}

/// Folder content comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ContentArg {
    /// Trust equal size + modification time, otherwise read bytes.
    Smart,
    /// Always read bytes when sizes match.
    Exact,
    /// Never read file contents.
    Metadata,
}

/// When to use ANSI colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

/// Arguments of `difflux diff`.
#[derive(Debug, Clone, Args)]
pub struct DiffArgs {
    /// Left file or folder.
    pub left: PathBuf,
    /// Right file or folder.
    pub right: PathBuf,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
    /// Shorthand for `--format json`.
    #[arg(long, conflicts_with = "format")]
    pub json: bool,
    /// Only report whether the inputs differ (like `diff -q`).
    #[arg(short = 'q', long)]
    pub brief: bool,

    /// How whitespace differences are treated.
    #[arg(long, value_enum, default_value_t = WhitespaceArg::Exact)]
    pub whitespace: WhitespaceArg,
    /// Ignore all whitespace (same as `--whitespace all`).
    #[arg(short = 'w', long)]
    pub ignore_all_space: bool,
    /// Ignore changes in the amount of whitespace (same as `--whitespace change`).
    #[arg(short = 'b', long)]
    pub ignore_space_change: bool,
    /// Ignore case differences.
    #[arg(short = 'i', long)]
    pub ignore_case: bool,
    /// Treat CRLF and LF line endings as different.
    #[arg(long)]
    pub strict_line_endings: bool,
    /// Lines of context around each change.
    #[arg(short = 'U', long, default_value_t = 3)]
    pub context: u32,
    /// Stop after this many diff lines (the report says it was truncated).
    #[arg(long)]
    pub max_lines: Option<usize>,

    /// How file contents are compared inside folders.
    #[arg(long, value_enum, default_value_t = ContentArg::Smart)]
    pub content: ContentArg,
    /// Skip entries matching this glob (repeatable), e.g. `-x target -x '*.log'`.
    #[arg(short = 'x', long = "exclude", value_name = "GLOB")]
    pub excludes: Vec<String>,
    /// Treat names as case-sensitive even on Windows and macOS.
    #[arg(long, conflicts_with = "case_insensitive_names")]
    pub case_sensitive_names: bool,
    /// Treat names as case-insensitive even on Linux.
    #[arg(long)]
    pub case_insensitive_names: bool,
    /// Follow symbolic links instead of comparing their targets.
    #[arg(long)]
    pub follow_symlinks: bool,
    /// Worker threads for folder scans (lower it for HDDs and network shares).
    #[arg(long)]
    pub threads: Option<usize>,
    /// Also list identical entries in folder reports.
    #[arg(long)]
    pub all: bool,
    /// Stop after this many folder entries (the report says it was truncated).
    #[arg(long)]
    pub max_entries: Option<usize>,

    /// When to color text output (respects `NO_COLOR`).
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    pub color: ColorArg,
}

impl DiffArgs {
    /// Comparison options implied by the flags.
    #[must_use]
    pub fn options(&self) -> CompareOptions {
        let whitespace = if self.ignore_all_space {
            WhitespaceMode::All
        } else if self.ignore_space_change {
            WhitespaceMode::Change
        } else {
            match self.whitespace {
                WhitespaceArg::Exact => WhitespaceMode::Exact,
                WhitespaceArg::Trailing => WhitespaceMode::Trailing,
                WhitespaceArg::Change => WhitespaceMode::Change,
                WhitespaceArg::All => WhitespaceMode::All,
            }
        };
        let names = if self.case_sensitive_names {
            difflux_core::NameMatching::CaseSensitive
        } else if self.case_insensitive_names {
            difflux_core::NameMatching::CaseInsensitive
        } else {
            difflux_core::NameMatching::Auto
        };
        CompareOptions {
            text: TextOptions { whitespace, ignore_case: self.ignore_case, ignore_line_endings: !self.strict_line_endings },
            folder: FolderOptions {
                content: match self.content {
                    ContentArg::Smart => ContentMode::Smart,
                    ContentArg::Exact => ContentMode::Exact,
                    ContentArg::Metadata => ContentMode::Metadata,
                },
                names,
                follow_symlinks: self.follow_symlinks,
                excludes: self.excludes.clone(),
                mtime_tolerance_ms: 0,
                threads: self.threads,
            },
        }
    }

    fn limits(&self) -> Limits {
        Limits {
            context: self.context,
            max_lines: self.max_lines.unwrap_or(usize::MAX),
            max_entries: self.max_entries.unwrap_or(usize::MAX),
            include_identical: self.all,
        }
    }

    fn format(&self) -> Format {
        if self.json { Format::Json } else { self.format }
    }

    fn use_color(&self) -> bool {
        match self.color {
            ColorArg::Always => true,
            ColorArg::Never => false,
            ColorArg::Auto => std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal(),
        }
    }
}

/// Runs `difflux diff` and returns the exit code.
pub fn run_diff(args: &DiffArgs, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<u8> {
    let opts = args.options();
    let result = compare_paths(&args.left, &args.right, &opts, &CancelToken::new(), &Progress::new());
    let comparison = match result {
        Ok(c) => c,
        Err(e) => return report_error(args, &e, out, err),
    };
    let code = if comparison.is_difference() { EXIT_DIFFERENT } else { EXIT_SAME };
    match args.format() {
        Format::Json => {
            let report = match &comparison {
                Comparison::Files(f) => Report::Files(output::files_report(f, args.limits())),
                Comparison::Folders(d) => Report::Folders(output::folders_report(d, &opts.folder, args.limits())),
            };
            serde_json::to_writer_pretty(&mut *out, &Output::new(report))?;
            writeln!(out)?;
        }
        Format::Text => {
            let style = Style::new(args.use_color());
            match &comparison {
                Comparison::Files(f) => write_files_text(f, args, &style, out)?,
                Comparison::Folders(d) => write_folders_text(d, args, &style, out)?,
            }
        }
    }
    Ok(code)
}

fn report_error(args: &DiffArgs, e: &CompareError, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<u8> {
    if args.format() == Format::Json {
        let report = Report::Error(output::ErrorReport { message: e.to_string() });
        serde_json::to_writer_pretty(&mut *out, &Output::new(report))?;
        writeln!(out)?;
    } else {
        writeln!(err, "difflux: {e}")?;
    }
    Ok(EXIT_TROUBLE)
}

/// Writes the JSON Schema of every JSON output.
pub fn write_schema(out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, &output::schema())?;
    writeln!(out)
}

struct Style {
    color: bool,
}

impl Style {
    fn new(color: bool) -> Self {
        Self { color }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_owned() }
    }
}

// Colors pair red with cyan (not green) so they stay distinct for the most common
// color-vision deficiencies; markers carry the meaning on their own anyway.
const RED: &str = "31";
const CYAN: &str = "36";
const YELLOW: &str = "33";
const BOLD: &str = "1";
const DIM: &str = "2";

fn write_files_text(f: &FileCompare, args: &DiffArgs, style: &Style, out: &mut dyn Write) -> io::Result<()> {
    let (l, r) = (&f.left.name, &f.right.name);
    match f.verdict {
        FileVerdict::Identical | FileVerdict::Equivalent => {
            if args.brief {
                return Ok(());
            }
            if f.verdict == FileVerdict::Equivalent {
                writeln!(out, "{}", style.paint(DIM, "# files differ only in ignored whitespace, case or line endings"))?;
            }
        }
        FileVerdict::BinaryDifferent => writeln!(out, "Binary files {l} and {r} differ")?,
        FileVerdict::Different if args.brief => writeln!(out, "Files {l} and {r} differ")?,
        FileVerdict::Different => {
            let mut text = String::new();
            if let (Some(d), Some(ld), Some(rd)) = (&f.diff, f.left.document(), f.right.document()) {
                d.write_unified(ld, rd, (l, r), args.context, &mut text);
            }
            let max = args.max_lines.unwrap_or(usize::MAX);
            for (i, line) in text.lines().enumerate() {
                if i >= max {
                    writeln!(out, "{}", style.paint(DIM, "# output truncated by --max-lines"))?;
                    break;
                }
                let painted = if line.starts_with("---") || line.starts_with("+++") {
                    style.paint(BOLD, line)
                } else if line.starts_with("@@") {
                    style.paint(YELLOW, line)
                } else if line.starts_with('-') {
                    style.paint(RED, line)
                } else if line.starts_with('+') {
                    style.paint(CYAN, line)
                } else {
                    line.to_owned()
                };
                writeln!(out, "{painted}")?;
            }
        }
    }
    Ok(())
}

fn write_folders_text(d: &FolderDiff, args: &DiffArgs, style: &Style, out: &mut dyn Write) -> io::Result<()> {
    if args.brief {
        if !d.is_equal() {
            writeln!(out, "Folders {} and {} differ", d.left_root.display(), d.right_root.display())?;
        }
        return Ok(());
    }
    let max = args.max_entries.unwrap_or(usize::MAX);
    let mut shown = 0usize;
    let mut stack: Vec<NodeId> = d.children(FolderDiff::ROOT).rev().collect();
    while let Some(id) = stack.pop() {
        let n = d.node(id);
        let is_container = n.is_dir() && matches!(n.status, Status::Different | Status::Identical);
        let show = !is_container && (args.all || n.status.is_difference());
        if show {
            if shown >= max {
                writeln!(out, "{}", style.paint(DIM, "# output truncated by --max-entries"))?;
                break;
            }
            shown += 1;
            let (marker, color) = match n.status {
                Status::Identical => ("=", DIM),
                Status::Different => ("M", YELLOW),
                Status::LeftOnly => ("<", RED),
                Status::RightOnly => (">", CYAN),
                Status::TypeMismatch => ("T", YELLOW),
                Status::Error => ("!", RED),
            };
            let slash = if n.is_dir() { "/" } else { "" };
            let mut line = format!("{marker} {}{slash}", d.rel_path(id));
            if let Some(e) = &n.error {
                let _ = write!(line, "  ({e})");
            }
            writeln!(out, "{}", style.paint(color, &line))?;
        }
        let one_sided = matches!(n.status, Status::LeftOnly | Status::RightOnly);
        if !one_sided || args.all {
            stack.extend(n.children.clone().rev());
        }
    }
    let s = &d.stats;
    let summary = format!(
        "{} different, {} left only, {} right only, {} type mismatches, {} errors · {} identical · {:.2} s",
        s.different,
        s.left_only,
        s.right_only,
        s.type_mismatches,
        s.errors,
        s.identical,
        d.elapsed.as_secs_f64()
    );
    writeln!(out, "{}", style.paint(DIM, &summary))
}
