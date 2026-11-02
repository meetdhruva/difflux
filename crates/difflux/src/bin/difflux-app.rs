//! Desktop launcher for difflux.
//!
//! Same app as `difflux [LEFT RIGHT]`, but built for the Windows GUI subsystem
//! so double-clicking it does not open a console window. The macOS `.app`
//! bundle runs this binary too.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    difflux_app::crash::install(env!("CARGO_PKG_VERSION"));
    let mut paths = std::env::args_os().skip(1).map(PathBuf::from);
    let (left, right) = (paths.next(), paths.next());
    match difflux_gui::run(left, right) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "difflux: cannot start the desktop app: {e}");
            ExitCode::from(2)
        }
    }
}
