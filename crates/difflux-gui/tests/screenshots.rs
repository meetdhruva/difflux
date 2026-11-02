//! Renders key screens to PNG for design review and docs:
//! `cargo test -p difflux-gui --test screenshots -- --ignored`
//! Images land in `target/screenshots/`. Needs a GPU or a software
//! Vulkan driver (lavapipe), so it is not part of the default test run.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use difflux_app::Command;
use difflux_gui::{DiffluxApp, Settings};
use egui::Key;
use egui_kittest::Harness;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn out_dir() -> std::io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn harness(app: DiffluxApp) -> Harness<'static, DiffluxApp> {
    Harness::builder().with_size([1280.0, 760.0]).wgpu().build_ui_state(|ui, app: &mut DiffluxApp| app.show(ui), app)
}

fn settle(h: &mut Harness<'_, DiffluxApp>) {
    for _ in 0..1000 {
        h.step();
        if !h.state().is_busy() {
            h.run_steps(4);
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn save(h: &mut Harness<'_, DiffluxApp>, name: &str) -> TestResult {
    let img = h.render()?;
    img.save(out_dir()?.join(format!("{name}.png")))?;
    Ok(())
}

fn light(h: &mut Harness<'_, DiffluxApp>) {
    let ctx = h.ctx.clone();
    h.state_mut().run_command(&ctx, Command::ToggleTheme);
    h.run_steps(3);
}

fn sample_tree(root: &Path) -> std::io::Result<(PathBuf, PathBuf)> {
    let (l, r) = (root.join("before"), root.join("after"));
    for side in [&l, &r] {
        fs::create_dir_all(side.join("src/engine"))?;
        fs::create_dir_all(side.join("docs"))?;
        fs::write(side.join("README.md"), "# Project\n")?;
        fs::write(side.join("src/engine/cache.rs"), "pub struct Cache;\n")?;
        fs::write(side.join("docs/guide.md"), "Guide\n")?;
    }
    let left_main = "use std::collections::HashMap;\n\nfn main() {\n    let config = load_config(\"app.toml\");\n    let total = compute(&config, 1);\n    println!(\"total = {total}\");\n}\n\nfn load_config(path: &str) -> Config {\n    Config::from_file(path).expect(\"config\")\n}\n\nfn compute(config: &Config, factor: u32) -> u32 {\n    config.base * factor\n}\n";
    let right_main = "use std::collections::BTreeMap;\n\nfn main() {\n    tracing::info!(\"starting\");\n    let config = load_config(\"app.toml\");\n    let total = compute(&config, 2);\n    println!(\"total = {total}\");\n}\n\nfn load_config(path: &str) -> Config {\n    Config::from_file(path).unwrap_or_default()\n}\n\nfn compute(config: &Config, factor: u32) -> u32 {\n    config.base.saturating_mul(factor)\n}\n";
    fs::write(l.join("src/main.rs"), left_main)?;
    fs::write(r.join("src/main.rs"), right_main)?;
    fs::write(l.join("src/legacy.rs"), "// old\n")?;
    fs::write(r.join("src/telemetry.rs"), "// new\n")?;
    fs::write(l.join("docs/guide.md"), "Guide v1\n")?;
    fs::write(r.join("docs/guide.md"), "Guide version 2\n")?;
    Ok((l, r))
}

#[test]
#[ignore = "needs a GPU or lavapipe; run explicitly"]
fn render_key_screens() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (l, r) = sample_tree(dir.path())?;

    let mut h = harness(DiffluxApp::with_settings(Settings::default(), None, None));
    h.run_steps(3);
    save(&mut h, "01-home-dark")?;

    let mut h = harness(DiffluxApp::with_settings(Settings::default(), Some(l.clone()), Some(r.clone())));
    settle(&mut h);
    save(&mut h, "02-folder-dark")?;
    light(&mut h);
    save(&mut h, "03-folder-light")?;

    let mut h = harness(DiffluxApp::with_settings(Settings::default(), Some(l.join("src/main.rs")), Some(r.join("src/main.rs"))));
    settle(&mut h);
    save(&mut h, "04-text-dark")?;
    h.key_press(Key::F8);
    h.run_steps(3);
    light(&mut h);
    save(&mut h, "05-text-light")?;

    h.key_press(Key::F1);
    h.run_steps(3);
    save(&mut h, "06-palette")?;
    Ok(())
}
