//! Folder compare behaviour on real (temporary) trees.

use std::fs;
use std::path::Path;

use difflux_core::{CancelToken, ContentMode, FolderOptions, Progress, Side};
use difflux_diff_dir::{Basis, FolderDiff, NodeId, Status, compare_folders};
use filetime::{FileTime, set_file_mtime};
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write(root: &Path, rel: &str, content: &str) -> std::io::Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

fn run(l: &Path, r: &Path, opts: &FolderOptions) -> Result<FolderDiff, difflux_diff_dir::Error> {
    compare_folders(l, r, opts, &CancelToken::new(), &Progress::new())
}

fn find(d: &FolderDiff, rel: &str) -> Option<NodeId> {
    (0..d.nodes.len() as NodeId).find(|&i| d.rel_path(i) == rel)
}

fn status(d: &FolderDiff, rel: &str) -> Option<Status> {
    find(d, rel).map(|i| d.node(i).status)
}

#[test]
fn classifies_every_kind_of_difference() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    write(l.path(), "same.txt", "hello")?;
    write(r.path(), "same.txt", "hello")?;
    write(l.path(), "size.txt", "short")?;
    write(r.path(), "size.txt", "much longer")?;
    write(l.path(), "bytes.txt", "aaaa")?;
    write(r.path(), "bytes.txt", "bbbb")?;
    write(l.path(), "only-left.txt", "x")?;
    write(r.path(), "only-right.txt", "x")?;
    write(l.path(), "sub/deep/a.txt", "1")?;
    write(r.path(), "sub/deep/a.txt", "1")?;
    write(l.path(), "kind", "file")?;
    fs::create_dir(r.path().join("kind"))?;

    let d = run(l.path(), r.path(), &FolderOptions { content: ContentMode::Exact, ..FolderOptions::default() })?;

    assert_eq!(status(&d, "same.txt"), Some(Status::Identical));
    assert_eq!(status(&d, "size.txt"), Some(Status::Different));
    assert_eq!(find(&d, "size.txt").and_then(|i| d.node(i).basis), Some(Basis::Size));
    assert_eq!(status(&d, "bytes.txt"), Some(Status::Different));
    assert_eq!(find(&d, "bytes.txt").and_then(|i| d.node(i).basis), Some(Basis::Content));
    assert_eq!(status(&d, "only-left.txt"), Some(Status::LeftOnly));
    assert_eq!(status(&d, "only-right.txt"), Some(Status::RightOnly));
    assert_eq!(status(&d, "sub"), Some(Status::Identical));
    assert_eq!(status(&d, "sub/deep/a.txt"), Some(Status::Identical));
    assert_eq!(status(&d, "kind"), Some(Status::TypeMismatch));
    assert_eq!(d.node(FolderDiff::ROOT).status, Status::Different);
    assert!(!d.is_equal());
    assert_eq!(d.stats.different, 2);
    assert_eq!(d.stats.left_only, 1);
    assert_eq!(d.stats.right_only, 1);
    assert_eq!(d.stats.type_mismatches, 1);
    Ok(())
}

#[test]
fn identical_trees_are_equal() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    for root in [l.path(), r.path()] {
        write(root, "a/b/c.txt", "same")?;
        write(root, "d.txt", "same")?;
    }
    let d = run(l.path(), r.path(), &FolderOptions::default())?;
    assert!(d.is_equal());
    assert_eq!(d.stats.differences(), 0);
    assert_eq!(d.stats.folders, 2);
    Ok(())
}

#[test]
fn children_are_contiguous_and_folders_sort_first() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    write(l.path(), "zeta/x", "1")?;
    write(l.path(), "Alpha.txt", "1")?;
    write(l.path(), "beta.txt", "1")?;
    write(r.path(), "beta.txt", "1")?;
    let d = run(l.path(), r.path(), &FolderOptions::default())?;
    let names: Vec<&str> = d.children(FolderDiff::ROOT).map(|i| d.node(i).name.as_str()).collect();
    assert_eq!(names, ["zeta", "Alpha.txt", "beta.txt"]);
    for i in d.children(FolderDiff::ROOT) {
        assert_eq!(d.node(i).parent, Some(FolderDiff::ROOT));
    }
    Ok(())
}

#[test]
fn smart_mode_trusts_equal_size_and_time_but_exact_mode_reads() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    write(l.path(), "f.bin", "AAAA")?;
    write(r.path(), "f.bin", "BBBB")?;
    let t = FileTime::from_unix_time(1_700_000_000, 0);
    set_file_mtime(l.path().join("f.bin"), t)?;
    set_file_mtime(r.path().join("f.bin"), t)?;

    let smart = run(l.path(), r.path(), &FolderOptions::default())?;
    let node = smart.node(find(&smart, "f.bin").ok_or("missing")?);
    assert_eq!((node.status, node.basis), (Status::Identical, Some(Basis::Metadata)));

    let exact = run(l.path(), r.path(), &FolderOptions { content: ContentMode::Exact, ..FolderOptions::default() })?;
    assert_eq!(status(&exact, "f.bin"), Some(Status::Different));
    Ok(())
}

#[test]
fn excludes_skip_names_and_paths() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    write(l.path(), "target/debug/out", "1")?;
    write(l.path(), "src/gen/skip.rs", "1")?;
    write(l.path(), "keep.rs", "1")?;
    write(r.path(), "keep.rs", "1")?;
    let opts = FolderOptions { excludes: vec!["target".into(), "src/gen".into()], ..FolderOptions::default() };
    let d = run(l.path(), r.path(), &opts)?;
    assert_eq!(find(&d, "target"), None);
    assert_eq!(find(&d, "src/gen"), None);
    assert_eq!(status(&d, "src"), Some(Status::LeftOnly));
    Ok(())
}

#[test]
fn case_insensitive_matching_pairs_names_and_keeps_each_spelling() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    write(l.path(), "README.md", "same")?;
    write(r.path(), "readme.md", "same")?;
    let opts = FolderOptions { names: difflux_core::NameMatching::CaseInsensitive, ..FolderOptions::default() };
    let d = run(l.path(), r.path(), &opts)?;
    let id = find(&d, "README.md").ok_or("missing")?;
    assert_eq!(d.node(id).status, Status::Identical);
    assert_eq!(d.side_path(id, Side::Right), Some(r.path().join("readme.md")));
    Ok(())
}

#[test]
fn unicode_names_match_across_normalization_forms() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    write(l.path(), "caf\u{e9}.txt", "same")?; // NFC
    write(r.path(), "cafe\u{301}.txt", "same")?; // NFD, as macOS stores it
    let d = run(l.path(), r.path(), &FolderOptions::default())?;
    assert!(d.is_equal(), "{:?}", d.nodes);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinks_compare_targets_by_default() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    std::os::unix::fs::symlink("one", l.path().join("link"))?;
    std::os::unix::fs::symlink("two", r.path().join("link"))?;
    let d = run(l.path(), r.path(), &FolderOptions::default())?;
    let node = d.node(find(&d, "link").ok_or("missing")?);
    assert_eq!((node.status, node.basis), (Status::Different, Some(Basis::LinkTarget)));
    Ok(())
}

#[test]
fn rejects_files_as_roots() -> TestResult {
    let dir = TempDir::new()?;
    write(dir.path(), "f", "x")?;
    let err = run(&dir.path().join("f"), dir.path(), &FolderOptions::default());
    assert!(matches!(err, Err(difflux_diff_dir::Error::NotAFolder(_))));
    Ok(())
}

#[test]
fn cancellation_stops_the_scan() -> TestResult {
    let (l, r) = (TempDir::new()?, TempDir::new()?);
    write(l.path(), "a", "1")?;
    let cancel = CancelToken::new();
    cancel.cancel();
    let res = compare_folders(l.path(), r.path(), &FolderOptions::default(), &cancel, &Progress::new());
    assert!(matches!(res, Err(difflux_diff_dir::Error::Cancelled(_))));
    Ok(())
}
