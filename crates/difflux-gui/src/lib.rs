//! difflux desktop app, built on egui.
//!
//! Rendering is custom-painted and virtualized (only visible rows are laid out),
//! the UI thread never touches the disk, and every row carries an AccessKit
//! label so screen readers can follow along.

mod app;
mod folder_pane;
mod format;
mod keymap;
mod palette;
mod text_pane;
mod theme;

use std::path::PathBuf;

pub use app::{DiffluxApp, Settings};

/// Opens the main window, optionally comparing two paths right away.
pub fn run(left: Option<PathBuf>, right: Option<PathBuf>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("difflux")
            .with_app_id("difflux")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 400.0])
            .with_drag_and_drop(true),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native("difflux", options, Box::new(move |cc| Ok(Box::new(DiffluxApp::new(cc, left, right)))))
}
