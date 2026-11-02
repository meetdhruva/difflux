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

/// App icon (rendered from `assets/icon/difflux.svg` by `cargo xtask icons`).
pub(crate) const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon/png/difflux-256.png");

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
    let options = match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(icon) => eframe::NativeOptions { viewport: options.viewport.with_icon(icon), ..options },
        Err(e) => {
            tracing::warn!("app icon could not be decoded: {e}");
            options
        }
    };
    eframe::run_native("difflux", options, Box::new(move |cc| Ok(Box::new(DiffluxApp::new(cc, left, right)))))
}
