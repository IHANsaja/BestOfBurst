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
            .with_drag_and_drop(true)
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "BestOfBurst",
        options,
        Box::new(move |cc| Ok(Box::new(app::BestOfBurstApp::new(cc, folder)))),
    )
    .map_err(|e| anyhow::anyhow!("window: {e}"))
}

/// The burst logo for the title bar and taskbar.
fn window_icon() -> std::sync::Arc<eframe::egui::IconData> {
    let png = include_bytes!("../../assets/logo/bestofburst-256.png");
    let icon = match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            eframe::egui::IconData { rgba: rgba.into_raw(), width, height }
        }
        Err(_) => eframe::egui::IconData::default(),
    };
    std::sync::Arc::new(icon)
}
