//! Drives the real UI headlessly through the accessibility tree.
//! If a test cannot find a widget by its label, a screen reader can't either.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::Duration;

use difflux_gui::{DiffluxApp, Settings};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn harness(app: DiffluxApp) -> Harness<'static, DiffluxApp> {
    Harness::builder().with_size([1200.0, 800.0]).build_ui_state(|ui, app: &mut DiffluxApp| app.show(ui), app)
}

/// Steps frames until the background compare finishes.
fn settle(h: &mut Harness<'_, DiffluxApp>) {
    for _ in 0..1000 {
        h.step();
        if !h.state().is_busy() {
            h.run_steps(3);
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("compare did not finish");
}

fn app_for(left: &Path, right: &Path) -> DiffluxApp {
    DiffluxApp::with_settings(Settings::default(), Some(left.to_path_buf()), Some(right.to_path_buf()))
}

#[test]
fn home_screen_is_labelled() {
    let mut h = harness(DiffluxApp::with_settings(Settings::default(), None, None));
    h.run();
    h.get_by_label("Compare");
    assert!(h.query_by_label("Differences only").is_none(), "folder controls only appear for folders");
    assert!(h.query_by_label_contains("Drop two files or folders").is_some());
}

#[test]
fn text_compare_rows_are_described_for_screen_readers() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (a, b) = (dir.path().join("a.rs"), dir.path().join("b.rs"));
    fs::write(&a, "fn main() {\n    let total = compute(1);\n}\n")?;
    fs::write(&b, "fn main() {\n    let total = compute(2);\n}\n")?;
    let mut h = harness(app_for(&a, &b));
    settle(&mut h);

    assert_eq!(h.state().error(), None);
    h.get_by_label_contains("Changed. Left line 2:");
    h.get_by_label("Unchanged line 1: fn main() {");
    assert!(h.query_by_label_contains("Difference 1 of 1").is_some());
    Ok(())
}

#[test]
fn keyboard_navigation_between_differences() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    let left: String = (0..200).fold(String::new(), |mut s, i| {
        let _ = writeln!(s, "line {i}");
        s
    });
    let right = left.replace("line 20\n", "LINE 20\n").replace("line 150\n", "LINE 150\n");
    fs::write(&a, left)?;
    fs::write(&b, right)?;
    let mut h = harness(app_for(&a, &b));
    settle(&mut h);

    assert!(h.query_by_label_contains("Difference 1 of 2").is_some());
    h.key_press(Key::F8);
    h.run();
    assert!(h.query_by_label_contains("Difference 2 of 2").is_some());
    h.key_press_modifiers(Modifiers::SHIFT, Key::F8);
    h.run();
    assert!(h.query_by_label_contains("Difference 1 of 2").is_some());

    // Ignoring case removes both differences.
    h.key_press_modifiers(Modifiers::ALT, Key::C);
    settle(&mut h);
    assert!(h.query_by_label_contains("No differences").is_some());
    assert!(h.state().settings().options.text.ignore_case);
    Ok(())
}

#[test]
fn folder_compare_open_file_and_go_back() -> TestResult {
    let (l, r) = (tempfile::tempdir()?, tempfile::tempdir()?);
    for root in [l.path(), r.path()] {
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("same.txt"), "same")?;
    }
    fs::write(l.path().join("src/main.rs"), "fn main() {}\n")?;
    fs::write(r.path().join("src/main.rs"), "fn main() { run(); }\n")?;
    fs::write(l.path().join("old.txt"), "gone")?;

    let mut h = harness(app_for(l.path(), r.path()));
    settle(&mut h);
    assert_eq!(h.state().depth(), 1);
    h.get_by_label_contains("src/main.rs file, different");
    h.get_by_label_contains("old.txt file, only on left");

    // Next difference selects src/main.rs; Enter opens it.
    h.key_press(Key::F8);
    h.run();
    h.key_press(Key::Enter);
    settle(&mut h);
    assert_eq!(h.state().depth(), 2);
    h.get_by_label_contains("Changed. Left line 1: fn main() {}");

    h.key_press(Key::Backspace);
    h.run();
    assert_eq!(h.state().depth(), 1);
    Ok(())
}

#[test]
fn command_palette_runs_commands() {
    let mut settings = Settings::default();
    settings.left = "/tmp/left".into();
    settings.right = "/tmp/right".into();
    let mut h = harness(DiffluxApp::with_settings(settings, None, None));
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    h.run();
    // The palette lists commands; typing filters and Enter runs the best match.
    assert!(h.query_all_by_label("Swap left and right").next().is_some());
    h.event(egui::Event::Text("swap".into()));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().settings().left, "/tmp/right");
    assert_eq!(h.state().settings().right, "/tmp/left");
}

#[test]
fn missing_paths_show_an_error_instead_of_crashing() {
    let mut h = harness(app_for(Path::new("/nope/left"), Path::new("/nope/right")));
    settle(&mut h);
    assert!(h.state().error().is_some_and(|e| e.contains("does not exist")));
    h.get_by_label("Dismiss");
}
