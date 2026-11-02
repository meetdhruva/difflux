//! Performance budgets. They only mean something in release builds:
//! `cargo xtask perf`. Budgets are generous enough for shared CI runners;
//! a failure means a real regression, not noise.

#![allow(clippy::print_stderr)] // reporting timings is the point of this file

use std::fmt::Write as _;
use std::fs;
use std::time::{Duration, Instant};

use difflux_core::{CancelToken, FolderOptions, Progress, TextOptions};
use difflux_diff_text::diff_documents;
use difflux_text::{Content, decode};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn budget(name: &str, limit: Duration, f: impl FnOnce() -> TestResult) -> TestResult {
    let started = Instant::now();
    f()?;
    let took = started.elapsed();
    eprintln!("{name}: {took:.2?} (budget {limit:.2?})");
    assert!(took <= limit, "{name} took {took:.2?}, budget is {limit:.2?}");
    Ok(())
}

fn source_like(lines: usize, seed: u64, change_every: usize) -> String {
    let mut s = String::with_capacity(lines * 48);
    for i in 0..lines {
        let changed = change_every > 0 && i % change_every == (seed as usize % change_every);
        let n = if changed { i as u64 * 7 + seed } else { i as u64 };
        let _ = writeln!(s, "    let value_{} = compute(input_{}, {n}); // line {i}", i % 997, i % 13);
    }
    s
}

fn doc(text: &str) -> Result<difflux_text::Document, Box<dyn std::error::Error>> {
    match decode(text.as_bytes()) {
        Content::Text(d) => Ok(d),
        Content::Binary { .. } => Err("unexpected binary".into()),
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "performance budgets run in release: cargo xtask perf")]
fn million_line_diff_with_scattered_changes() -> TestResult {
    let (a, b) = (source_like(1_000_000, 1, 100), source_like(1_000_000, 2, 100));
    budget("1M-line decode + diff", Duration::from_millis(2500), || {
        let (da, db) = (doc(&a)?, doc(&b)?);
        let d = diff_documents(&da, &db, &TextOptions::default(), &CancelToken::new())?;
        assert!(d.stats.hunks > 1000);
        Ok(())
    })
}

#[test]
#[cfg_attr(debug_assertions, ignore = "performance budgets run in release: cargo xtask perf")]
fn identical_large_file_is_near_instant() -> TestResult {
    let a = source_like(500_000, 0, 0);
    budget("500k identical lines", Duration::from_millis(300), || {
        let (da, db) = (doc(&a)?, doc(&a)?);
        let d = diff_documents(&da, &db, &TextOptions::default(), &CancelToken::new())?;
        assert!(d.is_equal());
        Ok(())
    })
}

#[test]
#[cfg_attr(debug_assertions, ignore = "performance budgets run in release: cargo xtask perf")]
fn folder_compare_of_twenty_thousand_files() -> TestResult {
    let (l, r) = (tempfile::tempdir()?, tempfile::tempdir()?);
    for root in [l.path(), r.path()] {
        for d in 0..200 {
            let dir = root.join(format!("pkg{d:03}/src"));
            fs::create_dir_all(&dir)?;
            for f in 0..100 {
                fs::write(dir.join(format!("file{f:03}.rs")), format!("// {d} {f}\nfn main() {{}}\n"))?;
            }
        }
    }
    fs::write(r.path().join("pkg007/src/file042.rs"), "changed")?;
    let opts = FolderOptions { content: difflux_core::ContentMode::Exact, ..FolderOptions::default() };
    budget("20k-file exact folder compare", Duration::from_secs(3), || {
        let d = difflux_diff_dir::compare_folders(l.path(), r.path(), &opts, &CancelToken::new(), &Progress::new())?;
        assert_eq!(d.stats.different, 1);
        assert_eq!(d.stats.identical, 19_999);
        Ok(())
    })
}

#[test]
#[cfg_attr(debug_assertions, ignore = "performance budgets run in release: cargo xtask perf")]
fn cli_round_trip_is_fast() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    fs::write(&a, source_like(2_000, 1, 50))?;
    fs::write(&b, source_like(2_000, 2, 50))?;
    // Warm the binary once (page cache, dynamic loader).
    std::process::Command::new(env!("CARGO_BIN_EXE_difflux")).args(["diff", "-q"]).arg(&a).arg(&b).output()?;
    budget("process start + small diff", Duration::from_millis(150), || {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_difflux")).args(["diff", "-q"]).arg(&a).arg(&b).output()?;
        assert_eq!(out.status.code(), Some(1));
        Ok(())
    })
}
