//! Project automation: `cargo xtask <task>`.
//!
//! * `ci`     — format check, clippy (warnings denied), tests, schema drift, cargo-deny if installed
//! * `perf`   — performance budgets in a release build
//! * `schema` — regenerate `docs/schema/difflux-output.schema.json`
//! * `fix`    — apply rustfmt and clippy fixes
//! * `icons`  — render `assets/icon/difflux.svg` to PNG sizes, `.ico` and `.icns`

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const SCHEMA_PATH: &str = "docs/schema/difflux-output.schema.json";

fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_default();
    let root = workspace_root();
    std::env::set_current_dir(&root)?;
    match task.as_str() {
        "ci" => ci(),
        "perf" => cargo(&[
            "test",
            "--release",
            "-p",
            "difflux",
            "--no-default-features",
            "--test",
            "perf_budget",
            "--",
            "--include-ignored",
            "--nocapture",
        ]),
        "schema" => write_schema(),
        "icons" => icons(),
        "fix" => {
            cargo(&["fmt", "--all"])?;
            cargo(&["clippy", "--workspace", "--all-targets", "--fix", "--allow-dirty", "--allow-staged"])
        }
        _ => {
            eprintln!("usage: cargo xtask <ci|perf|schema|fix|icons>");
            std::process::exit(2);
        }
    }
}

fn ci() -> Result<()> {
    step("format", || cargo(&["fmt", "--all", "--check"]))?;
    step("clippy", || cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))?;
    step("clippy (headless build)", || cargo(&["clippy", "-p", "difflux", "--no-default-features", "--", "-D", "warnings"]))?;
    step("tests", || {
        if has("cargo-nextest") {
            cargo(&["nextest", "run", "--workspace"])?;
            cargo(&["test", "--workspace", "--doc"])
        } else {
            cargo(&["test", "--workspace"])
        }
    })?;
    step("schema drift", check_schema)?;
    if has("cargo-deny") {
        step("licenses and advisories", || cargo(&["deny", "check"]))?;
    } else {
        eprintln!("note: cargo-deny not installed; skipping license/advisory check (cargo install cargo-deny)");
    }
    eprintln!("\nall checks passed");
    Ok(())
}

fn step(name: &str, f: impl FnOnce() -> Result<()>) -> Result<()> {
    eprintln!("\n── {name} ──");
    f().with_context(|| format!("{name} failed"))
}

fn generated_schema() -> Result<String> {
    let out = Command::new(cargo_bin())
        .args(["run", "--quiet", "-p", "difflux", "--no-default-features", "--", "schema"])
        .output()
        .context("running difflux schema")?;
    if !out.status.success() {
        bail!("difflux schema failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn write_schema() -> Result<()> {
    let schema = generated_schema()?;
    std::fs::create_dir_all(Path::new(SCHEMA_PATH).parent().context("schema dir")?)?;
    std::fs::write(SCHEMA_PATH, schema)?;
    eprintln!("wrote {SCHEMA_PATH}");
    Ok(())
}

fn check_schema() -> Result<()> {
    let current = std::fs::read_to_string(SCHEMA_PATH).unwrap_or_default();
    if generated_schema()? != current {
        bail!("{SCHEMA_PATH} is out of date: run `cargo xtask schema` and review the change (breaking changes need a new SCHEMA_ID)");
    }
    Ok(())
}

fn cargo(args: &[&str]) -> Result<()> {
    let status = Command::new(cargo_bin()).args(args).status().with_context(|| format!("running cargo {}", args.join(" ")))?;
    if !status.success() {
        bail!("cargo {} exited with {status}", args.join(" "));
    }
    Ok(())
}

fn has(tool: &str) -> bool {
    Command::new(cargo_bin()).args([tool.trim_start_matches("cargo-"), "--version"]).output().is_ok_and(|o| o.status.success())
}

fn cargo_bin() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

const ICON_SVG: &str = "assets/icon/difflux.svg";
const PNG_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256, 512, 1024];
const ICO_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];
const ICNS_SIZES: &[u32] = &[16, 32, 64, 128, 256, 512, 1024];

/// Straight (non-premultiplied) RGBA pixels of the icon at `size`.
fn render_icon(tree: &resvg::usvg::Tree, size: u32) -> Result<Vec<u8>> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).context("pixmap")?;
    let scale = size as f32 / tree.size().width();
    resvg::render(tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect())
}

fn write_png(path: &Path, size: u32, rgba: &[u8]) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn icons() -> Result<()> {
    let svg = std::fs::read(ICON_SVG).context(ICON_SVG)?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;
    let out = Path::new("assets/icon");
    std::fs::create_dir_all(out.join("png"))?;

    let mut ico = ico::IconDir::new(ico::ResourceType::Icon);
    let mut icns = icns::IconFamily::new();
    for &size in PNG_SIZES {
        let rgba = render_icon(&tree, size)?;
        write_png(&out.join(format!("png/difflux-{size}.png")), size, &rgba)?;
        if ICO_SIZES.contains(&size) {
            ico.add_entry(ico::IconDirEntry::encode(&ico::IconImage::from_rgba_data(size, size, rgba.clone()))?);
        }
        if ICNS_SIZES.contains(&size) {
            icns.add_icon(&icns::Image::from_data(icns::PixelFormat::RGBA, size, size, rgba)?)?;
        }
    }
    ico.write(std::fs::File::create(out.join("difflux.ico"))?)?;
    icns.write(std::io::BufWriter::new(std::fs::File::create(out.join("difflux.icns"))?))?;
    eprintln!("wrote assets/icon/png/*.png, difflux.ico and difflux.icns");
    Ok(())
}
