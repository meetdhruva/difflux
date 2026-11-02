//! Crash reports.
//!
//! A panic anywhere writes a plain-text report (version, OS, thread, message,
//! location, backtrace) to the platform's log folder and tells the user where
//! it is, so a bug report can include it. Panics inside background jobs are
//! still caught and shown in the UI; the report is written for those too,
//! because every panic is a bug.

use std::backtrace::Backtrace;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where to report bugs.
pub const ISSUES_URL: &str = "https://github.com/meetdhruva/difflux/issues/new/choose";

/// Folder for crash reports:
/// Windows `%LOCALAPPDATA%\difflux\crashes`, macOS `~/Library/Logs/difflux`,
/// elsewhere `$XDG_STATE_HOME/difflux/crashes` (default `~/.local/state/...`).
#[must_use]
pub fn report_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        env("LOCALAPPDATA").map(|d| d.join("difflux").join("crashes"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Logs/difflux"))
    } else {
        env("XDG_STATE_HOME").or_else(|| env("HOME").map(|h| h.join(".local/state"))).map(|d| d.join("difflux").join("crashes"))
    }
}

/// Builds the report text for a panic.
#[must_use]
pub fn report(info: &PanicHookInfo<'_>, version: &str) -> String {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(ToString::to_string)
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned());
    let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
    let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
    format!(
        "difflux {version} crashed\n\
         os: {} {}\n\
         thread: {thread}\n\
         message: {message}\n\
         location: {location}\n\n\
         backtrace:\n{}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        Backtrace::force_capture()
    )
}

/// Installs the crash reporter. Call once, first thing in `main`.
pub fn install(version: &'static str) {
    std::panic::set_hook(Box::new(move |info| {
        let text = report(info, version);
        let saved = save(&text);
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "\n{text}");
        match saved {
            Some(path) => {
                let _ = writeln!(err, "A crash report was saved to {}\nPlease attach it to a bug report: {ISSUES_URL}", path.display());
            }
            None => {
                let _ = writeln!(err, "Please include the text above in a bug report: {ISSUES_URL}");
            }
        }
    }));
}

fn save(text: &str) -> Option<PathBuf> {
    let dir = report_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let path = dir.join(format!("crash-{stamp}-{}.txt", std::process::id()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_folder_is_under_the_users_profile() {
        if let Some(dir) = report_dir() {
            assert!(dir.ends_with("crashes") || dir.ends_with("Logs/difflux"), "{}", dir.display());
        }
    }
}
