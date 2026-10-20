//! The egui window in the Grease Pencil design (DESIGN.md): Home, Processing, Review, Focus,
//! Compare, Done and Glim's corner, over the same engine as the CLI.

pub mod app;
pub mod compare;
pub mod corner;
pub mod done;
pub mod focus;
pub mod glim;
pub mod home;
pub mod marks;
pub mod progress;
pub mod review;
pub mod textures;
pub mod theme;

use std::path::PathBuf;

/// Open the window, optionally with a folder already chosen.
pub fn run(folder: Option<PathBuf>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("BestOfBurst")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "BestOfBurst",
        options,
        Box::new(move |cc| Ok(Box::new(app::BestOfBurstApp::new(cc, folder)))),
    )
    .map_err(|e| anyhow::anyhow!("window: {e}"))
}
