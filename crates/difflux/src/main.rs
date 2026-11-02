//! `difflux` — fast, reliable file and folder compare.
//!
//! ```text
//! difflux [LEFT RIGHT]          open the desktop app (optionally comparing two paths)
//! difflux diff LEFT RIGHT       compare in the terminal (text or --json)
//! difflux mcp                   run as an MCP server for AI agents (stdio)
//! difflux schema                print the JSON Schema of --json output
//! ```

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use difflux_cli::{DiffArgs, EXIT_TROUBLE};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Debug, Parser)]
#[command(
    name = "difflux",
    version,
    about = "Fast, reliable file and folder compare",
    long_about = None,
    args_conflicts_with_subcommands = true,
    after_help = "Exit codes: 0 = no differences, 1 = differences found, 2 = trouble.\n\
                  Logging: set DIFFLUX_LOG=debug (logs go to stderr)."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Paths to open in the desktop app: LEFT RIGHT.
    #[arg(num_args = 0..=2, value_name = "PATH")]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compare two files or folders in the terminal.
    Diff(DiffArgs),
    /// Run a Model Context Protocol server on stdio so AI agents can compare files.
    Mcp,
    /// Print the JSON Schema for `--json` output.
    Schema,
    /// Open the desktop app.
    Gui {
        #[arg(num_args = 0..=2, value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
}

fn main() -> ExitCode {
    difflux_app::crash::install(env!("CARGO_PKG_VERSION"));
    init_logging();
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Diff(args)) => {
            let (stdout, stderr) = (io::stdout(), io::stderr());
            difflux_cli::run_diff(&args, &mut stdout.lock(), &mut stderr.lock())
        }
        Some(Command::Mcp) => difflux_cli::mcp::serve(io::stdin().lock(), io::stdout().lock()).map(|()| 0),
        Some(Command::Schema) => difflux_cli::write_schema(&mut io::stdout().lock()).map(|()| 0),
        Some(Command::Gui { paths }) => Ok(gui(paths)),
        None => Ok(gui(cli.paths)),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        // Broken pipe (e.g. `difflux diff … | head`) is not an error worth reporting.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::from(0),
        Err(e) => {
            let _ = writeln!(io::stderr(), "difflux: {e}");
            ExitCode::from(EXIT_TROUBLE)
        }
    }
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("DIFFLUX_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_writer(io::stderr).try_init();
}

#[cfg(feature = "gui")]
fn gui(paths: Vec<PathBuf>) -> u8 {
    let mut it = paths.into_iter();
    let (left, right) = (it.next(), it.next());
    match difflux_gui::run(left, right) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(io::stderr(), "difflux: cannot start the desktop app: {e}");
            EXIT_TROUBLE
        }
    }
}

#[cfg(not(feature = "gui"))]
fn gui(_paths: Vec<PathBuf>) -> u8 {
    let _ = writeln!(
        io::stderr(),
        "difflux: this build has no desktop app (built without the `gui` feature).\n\
         Use `difflux diff LEFT RIGHT` or `difflux mcp`."
    );
    EXIT_TROUBLE
}
