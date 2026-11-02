//! Folder comparison.
//!
//! 1. Both trees are listed in parallel, one task per directory pair, on a
//!    dedicated rayon pool (size configurable for spinning disks and network shares).
//! 2. Names are matched by a merge-join over sorted keys (NFC-normalized,
//!    case-folded where the platform is case-insensitive).
//! 3. File pairs are classified in tiers, stopping at the first decisive answer:
//!    size → (smart mode) size + mtime → streamed byte comparison with early exit.
//! 4. Errors are per entry: an unreadable file becomes an error row, never an aborted scan.
//!
//! The result is an arena ([`FolderDiff::nodes`]) with contiguous children,
//! compact enough for millions of entries.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use difflux_core::{CancelToken, Cancelled, ContentMode, FolderOptions, Progress};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

/// Index into [`FolderDiff::nodes`].
pub type NodeId = u32;

const CHUNK: usize = 256 * 1024;

/// Kind of filesystem entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Metadata of an entry on one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct EntryMeta {
    #[serde(skip)]
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Modification time in milliseconds since the Unix epoch.
    pub modified_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
}

/// Verdict for one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Identical,
    Different,
    LeftOnly,
    RightOnly,
    /// A file on one side and a folder (or link) on the other.
    TypeMismatch,
    Error,
}

impl Status {
    #[must_use]
    pub fn is_difference(self) -> bool {
        self != Self::Identical
    }
}

/// What a verdict was based on, so users and agents know how much to trust it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Bytes were compared.
    Content,
    /// Sizes differ.
    Size,
    /// Size and modification time only (contents were not read).
    Metadata,
    /// Symbolic link targets were compared.
    LinkTarget,
    /// Folder verdict derived from its children.
    Children,
}

/// One row of the folder tree.
#[derive(Debug, Clone)]
pub struct Node {
    /// Display name (left name when both exist).
    pub name: String,
    pub parent: Option<NodeId>,
    pub children: Range<NodeId>,
    pub depth: u16,
    pub left: Option<EntryMeta>,
    pub right: Option<EntryMeta>,
    pub status: Status,
    pub basis: Option<Basis>,
    pub error: Option<String>,
}

impl Node {
    /// Kind used for display: the left kind when present.
    #[must_use]
    pub fn kind(&self) -> EntryKind {
        self.left.as_ref().or(self.right.as_ref()).map_or(EntryKind::Other, |m| m.kind)
    }

    #[must_use]
    pub fn is_dir(&self) -> bool {
        self.kind() == EntryKind::Dir
    }

    #[must_use]
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    #[must_use]
    pub fn side(&self, side: difflux_core::Side) -> Option<&EntryMeta> {
        match side {
            difflux_core::Side::Left => self.left.as_ref(),
            difflux_core::Side::Right => self.right.as_ref(),
        }
    }
}

/// Counts over files (folders are counted separately).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FolderStats {
    pub identical: u64,
    pub different: u64,
    pub left_only: u64,
    pub right_only: u64,
    pub type_mismatches: u64,
    pub errors: u64,
    /// Folders present on at least one side (the root excluded).
    pub folders: u64,
    pub bytes_compared: u64,
}

impl FolderStats {
    /// Number of entries that are not identical.
    #[must_use]
    pub fn differences(&self) -> u64 {
        self.different + self.left_only + self.right_only + self.type_mismatches + self.errors
    }
}

/// Result of [`compare_folders`].
#[derive(Debug, Clone)]
pub struct FolderDiff {
    pub left_root: PathBuf,
    pub right_root: PathBuf,
    /// `nodes[0]` is the root; children of a node are contiguous.
    pub nodes: Vec<Node>,
    pub stats: FolderStats,
    pub elapsed: Duration,
}

impl FolderDiff {
    pub const ROOT: NodeId = 0;

    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// True when every entry is identical.
    #[must_use]
    pub fn is_equal(&self) -> bool {
        self.nodes[0].status == Status::Identical
    }

    #[must_use]
    pub fn children(&self, id: NodeId) -> Range<NodeId> {
        self.nodes[id as usize].children.clone()
    }

    /// Path relative to the roots, `/`-separated.
    #[must_use]
    pub fn rel_path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(i) = cur.filter(|&i| i != Self::ROOT) {
            let node = &self.nodes[i as usize];
            parts.push(node.name.as_str());
            cur = node.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// Absolute path of the entry on one side, using that side's own spelling of each name.
    #[must_use]
    pub fn side_path(&self, id: NodeId, side: difflux_core::Side) -> Option<PathBuf> {
        let mut names = Vec::new();
        let mut cur = Some(id);
        while let Some(i) = cur.filter(|&i| i != Self::ROOT) {
            let node = &self.nodes[i as usize];
            names.push(node.side(side)?.name.as_str());
            cur = node.parent;
        }
        let root = match side {
            difflux_core::Side::Left => &self.left_root,
            difflux_core::Side::Right => &self.right_root,
        };
        Some(names.iter().rev().fold(root.clone(), |p, n| p.join(n)))
    }
}

/// Why a folder compare could not run at all.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Cancelled(#[from] Cancelled),
    #[error("{0} is not a folder")]
    NotAFolder(PathBuf),
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid exclude pattern {pattern:?}: {message}")]
    BadPattern { pattern: String, message: String },
    #[error("cannot start worker threads: {0}")]
    Threads(String),
}

/// Compares two folder trees.
pub fn compare_folders(
    left: &Path,
    right: &Path,
    opts: &FolderOptions,
    cancel: &CancelToken,
    progress: &Progress,
) -> Result<FolderDiff, Error> {
    let started = Instant::now();
    let left_meta = root_meta(left)?;
    let right_meta = root_meta(right)?;
    let ctx = Ctx { opts, excludes: build_excludes(&opts.excludes)?, case_insensitive: opts.names.is_case_insensitive(), cancel, progress };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads.unwrap_or(0))
        .stack_size(8 << 20)
        .thread_name(|i| format!("difflux-scan-{i}"))
        .build()
        .map_err(|e| Error::Threads(e.to_string()))?;

    let root = pool.install(|| {
        let left = Entry { path: left.to_path_buf(), meta: left_meta };
        let right = Entry { path: right.to_path_buf(), meta: right_meta };
        compare_entry(&ctx, String::new(), "", Some(left), Some(right))
    })?;

    let (nodes, stats) = flatten(root);
    Ok(FolderDiff { left_root: left.to_path_buf(), right_root: right.to_path_buf(), nodes, stats, elapsed: started.elapsed() })
}

fn root_meta(path: &Path) -> Result<EntryMeta, Error> {
    let md = fs::metadata(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
    if !md.is_dir() {
        return Err(Error::NotAFolder(path.to_path_buf()));
    }
    Ok(EntryMeta { name: String::new(), kind: EntryKind::Dir, size: 0, modified_ms: modified_ms(&md), link_target: None })
}

fn build_excludes(patterns: &[String]) -> Result<GlobSet, Error> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p).map_err(|e| Error::BadPattern { pattern: p.clone(), message: e.to_string() })?;
        builder.add(glob);
    }
    builder.build().map_err(|e| Error::BadPattern { pattern: patterns.join(", "), message: e.to_string() })
}

struct Ctx<'a> {
    opts: &'a FolderOptions,
    excludes: GlobSet,
    case_insensitive: bool,
    cancel: &'a CancelToken,
    progress: &'a Progress,
}

#[derive(Debug)]
struct Entry {
    path: PathBuf,
    meta: EntryMeta,
}

/// Owned tree produced by the parallel walk, flattened into the arena afterwards.
struct Temp {
    name: String,
    left: Option<EntryMeta>,
    right: Option<EntryMeta>,
    status: Status,
    basis: Option<Basis>,
    error: Option<String>,
    children: Vec<Temp>,
}

fn compare_entry(ctx: &Ctx<'_>, name: String, rel: &str, left: Option<Entry>, right: Option<Entry>) -> Result<Temp, Cancelled> {
    ctx.cancel.check()?;
    ctx.progress.add_items(1);
    let mut t = Temp { name, left: None, right: None, status: Status::Identical, basis: None, error: None, children: Vec::new() };

    match (&left, &right) {
        (Some(l), Some(r)) if l.meta.kind != r.meta.kind => {
            t.status = Status::TypeMismatch;
        }
        (Some(l), Some(r)) => match l.meta.kind {
            EntryKind::Dir => {
                let (children, error) = compare_dirs(ctx, rel, Some(&l.path), Some(&r.path))?;
                t.status = if error.is_some() {
                    Status::Error
                } else if children.iter().any(|c| c.status.is_difference()) {
                    Status::Different
                } else {
                    Status::Identical
                };
                t.basis = Some(Basis::Children);
                t.error = error;
                t.children = children;
            }
            EntryKind::File => classify_files(ctx, l, r, &mut t)?,
            EntryKind::Symlink => {
                t.basis = Some(Basis::LinkTarget);
                t.status = if l.meta.link_target == r.meta.link_target { Status::Identical } else { Status::Different };
            }
            EntryKind::Other => {
                t.basis = Some(Basis::Metadata);
                t.status = if l.meta.size == r.meta.size { Status::Identical } else { Status::Different };
            }
        },
        (Some(only), None) | (None, Some(only)) => {
            t.status = if left.is_some() { Status::LeftOnly } else { Status::RightOnly };
            if only.meta.kind == EntryKind::Dir {
                let (l, r) = if left.is_some() { (Some(only.path.as_path()), None) } else { (None, Some(only.path.as_path())) };
                let (children, error) = compare_dirs(ctx, rel, l, r)?;
                t.children = children;
                t.error = error;
            }
        }
        (None, None) => {}
    }
    t.left = left.map(|e| e.meta);
    t.right = right.map(|e| e.meta);
    Ok(t)
}

fn classify_files(ctx: &Ctx<'_>, left: &Entry, right: &Entry, out: &mut Temp) -> Result<(), Cancelled> {
    if left.meta.size != right.meta.size {
        out.status = Status::Different;
        out.basis = Some(Basis::Size);
        return Ok(());
    }
    let tolerance = i64::try_from(ctx.opts.mtime_tolerance_ms).unwrap_or(i64::MAX);
    let same_time = match (left.meta.modified_ms, right.meta.modified_ms) {
        (Some(l), Some(r)) => (l - r).abs() <= tolerance,
        _ => false,
    };
    match ctx.opts.content {
        ContentMode::Metadata => {
            out.basis = Some(Basis::Metadata);
            out.status = if same_time { Status::Identical } else { Status::Different };
        }
        ContentMode::Smart if same_time => {
            out.basis = Some(Basis::Metadata);
            out.status = Status::Identical;
        }
        ContentMode::Smart | ContentMode::Exact => {
            out.basis = Some(Basis::Content);
            match same_bytes(ctx, &left.path, &right.path) {
                Ok(true) => out.status = Status::Identical,
                Ok(false) => out.status = Status::Different,
                Err(_) if ctx.cancel.is_cancelled() => return Err(Cancelled),
                Err(e) => {
                    out.status = Status::Error;
                    out.error = Some(e.to_string());
                }
            }
        }
    }
    Ok(())
}

thread_local! {
    static BUFFERS: RefCell<(Vec<u8>, Vec<u8>)> = RefCell::new((vec![0; CHUNK], vec![0; CHUNK]));
}

/// Streams both files and stops at the first differing chunk.
fn same_bytes(ctx: &Ctx<'_>, a: &Path, b: &Path) -> io::Result<bool> {
    let mut fa = File::open(a).map_err(|e| with_path(e, a))?;
    let mut fb = File::open(b).map_err(|e| with_path(e, b))?;
    BUFFERS.with(|cell| {
        let mut bufs = cell.borrow_mut();
        let (ba, bb) = &mut *bufs;
        loop {
            if ctx.cancel.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let na = read_full(&mut fa, ba).map_err(|e| with_path(e, a))?;
            let nb = read_full(&mut fb, bb).map_err(|e| with_path(e, b))?;
            ctx.progress.add_bytes(na as u64);
            if na != nb || ba[..na] != bb[..nb] {
                return Ok(false); // also covers files that changed size while being read
            }
            if na == 0 {
                return Ok(true);
            }
        }
    })
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(|e| with_path(e, p))`
fn with_path(e: io::Error, path: &Path) -> io::Error {
    io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

fn read_full(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match f.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

type Listing = Vec<(String, Entry)>;

/// Lists both sides of a folder pair, matches names and compares children in parallel.
fn compare_dirs(ctx: &Ctx<'_>, rel: &str, left: Option<&Path>, right: Option<&Path>) -> Result<(Vec<Temp>, Option<String>), Cancelled> {
    let mut error = None;
    let mut list = |p: Option<&Path>| -> Listing {
        match p.map(|p| list_dir(ctx, rel, p)) {
            Some(Ok(v)) => v,
            Some(Err(e)) => {
                error.get_or_insert_with(|| e.to_string());
                Vec::new()
            }
            None => Vec::new(),
        }
    };
    let (l, r) = (list(left), list(right));
    let mut pairs = merge_join(l, r);
    pairs.sort_by_key(display_order);

    let children = pairs
        .into_par_iter()
        .map(|(name, l, r)| {
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            compare_entry(ctx, name, &child_rel, l, r)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((children, error))
}

type Pair = (String, Option<Entry>, Option<Entry>);

fn display_order(p: &Pair) -> (bool, String) {
    let is_dir = p.1.as_ref().or(p.2.as_ref()).is_some_and(|e| e.meta.kind == EntryKind::Dir);
    (!is_dir, p.0.to_lowercase())
}

/// Matches two sorted listings in O(n). Duplicate keys (possible when folding case
/// on a case-sensitive filesystem) pair up in order; extras stay one-sided.
fn merge_join(mut left: Listing, mut right: Listing) -> Vec<Pair> {
    left.sort_by(|a, b| a.0.cmp(&b.0));
    right.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::with_capacity(left.len().max(right.len()));
    let mut li = left.into_iter().peekable();
    let mut ri = right.into_iter().peekable();
    loop {
        let order = match (li.peek(), ri.peek()) {
            (Some(l), Some(r)) => l.0.cmp(&r.0),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => break,
        };
        match order {
            std::cmp::Ordering::Less => {
                if let Some((_, e)) = li.next() {
                    out.push((e.meta.name.clone(), Some(e), None));
                }
            }
            std::cmp::Ordering::Greater => {
                if let Some((_, e)) = ri.next() {
                    out.push((e.meta.name.clone(), None, Some(e)));
                }
            }
            std::cmp::Ordering::Equal => {
                if let (Some((_, l)), Some((_, r))) = (li.next(), ri.next()) {
                    out.push((l.meta.name.clone(), Some(l), Some(r)));
                }
            }
        }
    }
    out
}

fn match_key(name: &str, case_insensitive: bool) -> String {
    let nfc: String = if name.is_ascii() { name.to_owned() } else { name.nfc().collect() };
    if case_insensitive { nfc.to_lowercase() } else { nfc }
}

fn list_dir(ctx: &Ctx<'_>, rel: &str, dir: &Path) -> io::Result<Listing> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| with_path(e, dir))? {
        let entry = entry.map_err(|e| with_path(e, dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !ctx.excludes.is_empty() {
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if ctx.excludes.is_match(&name) || ctx.excludes.is_match(&child_rel) {
                continue;
            }
        }
        let path = entry.path();
        let meta = entry_meta(&path, name.clone(), ctx.opts.follow_symlinks);
        out.push((match_key(&name, ctx.case_insensitive), Entry { path, meta }));
    }
    Ok(out)
}

fn entry_meta(path: &Path, name: String, follow: bool) -> EntryMeta {
    let md = if follow { fs::metadata(path).or_else(|_| fs::symlink_metadata(path)) } else { fs::symlink_metadata(path) };
    match md {
        Ok(md) => {
            let ft = md.file_type();
            let kind = if ft.is_dir() {
                EntryKind::Dir
            } else if ft.is_file() {
                EntryKind::File
            } else if ft.is_symlink() {
                EntryKind::Symlink
            } else {
                EntryKind::Other
            };
            let link_target =
                (kind == EntryKind::Symlink).then(|| fs::read_link(path).ok().map(|t| t.to_string_lossy().into_owned())).flatten();
            EntryMeta { name, kind, size: if kind == EntryKind::File { md.len() } else { 0 }, modified_ms: modified_ms(&md), link_target }
        }
        // Vanished or unreadable between listing and stat: keep it visible as "other".
        Err(_) => EntryMeta { name, kind: EntryKind::Other, size: 0, modified_ms: None, link_target: None },
    }
}

fn modified_ms(md: &fs::Metadata) -> Option<i64> {
    let t = md.modified().ok()?;
    Some(match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    })
}

/// Breadth-first layout so every node's children occupy one contiguous range.
fn flatten(root: Temp) -> (Vec<Node>, FolderStats) {
    let mut nodes = Vec::new();
    let mut stats = FolderStats::default();
    let mut queue: VecDeque<(NodeId, Vec<Temp>)> = VecDeque::new();

    let push = |nodes: &mut Vec<Node>, t: Temp, parent: Option<NodeId>, depth: u16| -> (NodeId, Vec<Temp>) {
        let id = nodes.len() as NodeId;
        nodes.push(Node {
            name: t.name,
            parent,
            children: 0..0,
            depth,
            left: t.left,
            right: t.right,
            status: t.status,
            basis: t.basis,
            error: t.error,
        });
        (id, t.children)
    };

    let (root_id, root_children) = push(&mut nodes, root, None, 0);
    queue.push_back((root_id, root_children));
    while let Some((parent, children)) = queue.pop_front() {
        let start = nodes.len() as NodeId;
        let depth = nodes[parent as usize].depth.saturating_add(1);
        for child in children {
            let (id, grandchildren) = push(&mut nodes, child, Some(parent), depth);
            count(&mut stats, &nodes[id as usize]);
            queue.push_back((id, grandchildren));
        }
        nodes[parent as usize].children = start..nodes.len() as NodeId;
    }
    (nodes, stats)
}

fn count(stats: &mut FolderStats, node: &Node) {
    if node.status == Status::Error {
        stats.errors += 1;
    }
    if node.is_dir() && node.status != Status::TypeMismatch {
        stats.folders += 1;
        return;
    }
    match node.status {
        Status::Identical => stats.identical += 1,
        Status::Different => stats.different += 1,
        Status::LeftOnly => stats.left_only += 1,
        Status::RightOnly => stats.right_only += 1,
        Status::TypeMismatch => stats.type_mismatches += 1,
        Status::Error => {}
    }
    if node.basis == Some(Basis::Content) && node.status != Status::Error {
        stats.bytes_compared += node.left.as_ref().map_or(0, |m| m.size);
    }
}

/// Converts a millisecond timestamp back to `SystemTime` (for display code).
#[must_use]
pub fn system_time(ms: i64) -> SystemTime {
    if ms >= 0 { UNIX_EPOCH + Duration::from_millis(ms.unsigned_abs()) } else { UNIX_EPOCH - Duration::from_millis(ms.unsigned_abs()) }
}
