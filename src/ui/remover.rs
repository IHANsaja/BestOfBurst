//! Watermark remover, on its own: watermarked photos on one side, the folder with their
//! originals on the other. Each watermarked photo is matched to its original and rebuilt
//! under the mark (see `clean`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use eframe::egui::{self, pos2, vec2, Align2, Key, Rect, Sense, Stroke, StrokeKind, Ui};

use super::app::{self, BestOfBurstApp, BgMsg, Screen};
use super::glim::{self, Mood};
use super::theme::{self, Btn};
use crate::clean::{self, CleanSummary};
use crate::find;

#[derive(Default)]
pub struct RemoverState {
    /// Watermarked photos: files and folders as chosen.
    pub sent: Vec<PathBuf>,
    pub originals: Option<PathBuf>,
    pub running: bool,
    /// 0 reading the originals, 1 matching, 2 removing.
    pub stage: Arc<AtomicUsize>,
    pub progress: Arc<(AtomicUsize, AtomicUsize)>,
    pub stop: Arc<AtomicBool>,
    pub result: Option<CleanSummary>,
}

/// Where cleaned photos go when nothing else is said: a `watermark removed` folder beside the
/// first watermarked photo.
pub fn default_dir(queries: &[PathBuf]) -> PathBuf {
    queries
        .first()
        .and_then(|q| q.parent())
        .map(|p| p.join("watermark removed"))
        .unwrap_or_else(|| PathBuf::from("watermark removed"))
}

pub fn start(app: &mut BestOfBurstApp, ctx: &egui::Context) {
    let rs = &mut app.remover;
    let Some(originals) = rs.originals.clone() else { return };
    if rs.running || rs.sent.is_empty() {
        return;
    }
    rs.running = true;
    rs.result = None;
    rs.stage = Arc::new(AtomicUsize::new(0));
    rs.progress = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
    rs.stop = Arc::new(AtomicBool::new(false));
    let (sent, stage, progress, stop) = (rs.sent.clone(), rs.stage.clone(), rs.progress.clone(), rs.stop.clone());
    let (tx, ctx) = (app.bg_tx.clone(), ctx.clone());
    std::thread::spawn(move || {
        let tick = |done: usize, total: usize| {
            progress.0.store(done, Ordering::Relaxed);
            progress.1.store(total, Ordering::Relaxed);
            ctx.request_repaint();
        };
        let r = (|| -> anyhow::Result<CleanSummary> {
            let queries = find::collect_queries(&sent);
            anyhow::ensure!(!queries.is_empty(), "There are no photos in what you chose.");
            let index = find::Index::open(&originals, &stop, tick);
            anyhow::ensure!(!index.is_empty(), "No photos could be opened in the originals folder.");
            stage.store(1, Ordering::Relaxed);
            tick(0, queries.len());
            let report = find::find_originals(&index, &queries, None, &stop, tick);
            stage.store(2, Ordering::Relaxed);
            tick(0, queries.len());
            clean::clean_all(&originals, &report.matches, &default_dir(&queries), &stop, tick)
        })();
        let _ = tx.send(BgMsg::Cleaned(r.map_err(|e| format!("{e:#}"))));
        ctx.request_repaint();
    });
}

fn describe(paths: &[PathBuf]) -> String {
    match paths {
        [] => "nothing chosen yet".into(),
        [one] => one.display().to_string(),
        many => format!("{} photos or folders, first: {}", many.len(), many[0].display()),
    }
}

/// A numbered step box with its current choice; returns its rectangle's bottom for layout.
fn step(ui: &mut Ui, no: &str, title: &str, value: &str, set: bool, hovering: bool) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 92.0), Sense::hover());
    if hovering {
        ui.painter().rect_filled(rect, 0.0, theme::RED.gamma_multiply(0.08));
    }
    if set {
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(2.0, theme::INK), StrokeKind::Inside);
    } else {
        let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
        for seg in pts.windows(2) {
            ui.painter().extend(egui::Shape::dashed_line(&[seg[0], seg[1]], Stroke::new(2.0, theme::INK), 10.0, 6.0));
        }
    }
    theme::text_at(ui, pos2(rect.min.x + 22.0, rect.min.y + 24.0), Align2::LEFT_CENTER, no, theme::f_mono_semi(11.0), theme::GRAPHITE, 0.9);
    theme::text_at(ui, pos2(rect.min.x + 22.0, rect.min.y + 48.0), Align2::LEFT_CENTER, &title.to_uppercase(), theme::f_xp(18.0), theme::INK, 0.4);
    theme::text_at(ui, pos2(rect.min.x + 22.0, rect.min.y + 72.0), Align2::LEFT_CENTER, value, theme::f_mono(10.5), if set { theme::INK_2 } else { theme::GRAPHITE }, 0.3);
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let p = theme::pal(false);
    let (mut home, mut back) = (false, false);
    app::chrome_bar(
        ui,
        "remover_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, "Watermark remover", true);
        },
        |ui| {
            ui.add_space(16.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            back = theme::button(ui, "Back", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked();
            ui.add_space(16.0);
            app::cell_rule(ui);
        },
    );
    if !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(Key::Escape)) {
        back = true;
    }
    if home || back {
        app.screen = Screen::Start;
        return;
    }

    // Dropped watermarked photos (or, when those are set and a single folder comes, the originals).
    let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if !dropped.is_empty() && !app.remover.running {
        if !app.remover.sent.is_empty() && app.remover.originals.is_none() && dropped.len() == 1 && dropped[0].is_dir() {
            app.remover.originals = Some(dropped[0].clone());
        } else {
            app.remover.sent = dropped;
            app.remover.result = None;
        }
    }

    let side = egui::Frame::new().fill(theme::PAPER_DAY).inner_margin(28.0);
    let mut go = false;
    egui::Panel::right("remover_side").exact_size(420.0).resizable(false).frame(side).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.min.x - 28.0, r.min.y - 28.0), pos2(r.min.x - 28.0, r.max.y + 28.0)], Stroke::new(1.5, theme::INK));
        let rs = &app.remover;
        let msg = if rs.running {
            "Working through them. Big photos take a moment each.".to_string()
        } else if let Some(s) = &rs.result {
            if s.written > 0 { format!("{} are clean. Have a look.", theme::plural(s.written, "photo")) } else { "I couldn't pair these with originals in that folder.".to_string() }
        } else {
            "Give me the watermarked photos and the folder with the real ones.".to_string()
        };
        let mood = if rs.result.as_ref().is_some_and(|s| s.written > 0) { Mood::Cheer } else { Mood::Idle };
        let _ = glim::card(ui, mood, false, &msg, 60.0, app.time(), app.reduced_motion);
        ui.add_space(14.0);
        ui.label(theme::mono("How it works", 11.0, theme::GRAPHITE));
        ui.label(theme::body(
            "Each watermarked photo is matched to its original by what is in the picture (the file name helps, but isn't needed). The part under the watermark is rebuilt from the original, in the preview's own colours and size. The rest of the preview is left exactly as it is.",
            14.0,
            theme::INK_2,
        ));
        ui.add_space(10.0);
        ui.label(theme::body("It needs the originals: that is what makes the result the real picture and not a guess.", 14.0, theme::INK_2));
        if let Some(s) = &rs.result {
            ui.add_space(12.0);
            theme::rule(ui, theme::INK, 1.5);
            ui.add_space(8.0);
            for (n, label) in [(s.written, "cleaned"), (s.unmatched, "without a sure original"), (s.errors.len(), "couldn't be cleaned")] {
                if n > 0 || label == "cleaned" {
                    ui.horizontal(|ui| {
                        ui.label(theme::xp(theme::thousands(n), 28.0, theme::INK));
                        ui.label(theme::mono(label, 11.0, theme::GRAPHITE));
                    });
                }
            }
            for (name, e) in s.errors.iter().take(3) {
                ui.label(theme::body(format!("{name}: {e}"), 12.5, theme::RED));
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let w = ui.available_width();
            let ready = !rs.sent.is_empty() && rs.originals.is_some() && !rs.running;
            let label = if rs.running { "Working..." } else { "Remove watermarks" };
            if theme::button(ui, label, Some("enter"), Btn::Primary, 64.0, Some(w), ready).clicked() {
                go = true;
            }
            if let Some(s) = &rs.result {
                ui.add_space(8.0);
                if s.written > 0 && theme::button(ui, "Show folder", None, Btn::OnDay, 44.0, None, true).clicked() {
                    let _ = std::process::Command::new("explorer").arg(&s.dir).spawn();
                }
            }
        });
    });

    let main = egui::Frame::new().fill(theme::TABLE).inner_margin(egui::Margin { left: 48, right: 32, top: 32, bottom: 16 });
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        ui.label(theme::mono("On its own", 11.0, theme::GRAPHITE));
        ui.label(theme::xp("Remove watermarks", 34.0, theme::INK));
        ui.add_space(14.0);
        let sent = describe(&app.remover.sent);
        step(ui, "STEP 1", "The watermarked photos", &sent, !app.remover.sent.is_empty(), hovering);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if theme::button(ui, "Choose photos", None, Btn::OnDay, 44.0, None, !app.remover.running).clicked() {
                if let Some(f) = rfd::FileDialog::new().set_title("Choose the watermarked photos").pick_files() {
                    app.remover.sent = f;
                    app.remover.result = None;
                }
            }
            if theme::button(ui, "Choose a folder", None, Btn::OnDay, 44.0, None, !app.remover.running).clicked() {
                if let Some(f) = rfd::FileDialog::new().set_title("Choose the folder of watermarked photos").pick_folder() {
                    app.remover.sent = vec![f];
                    app.remover.result = None;
                }
            }
        });
        ui.add_space(22.0);
        let orig = app.remover.originals.as_ref().map(|o| o.display().to_string()).unwrap_or_else(|| "nothing chosen yet".into());
        step(ui, "STEP 2", "The folder with the originals", &orig, app.remover.originals.is_some(), false);
        ui.add_space(8.0);
        if theme::button(ui, "Choose the originals folder", None, Btn::OnDay, 44.0, None, !app.remover.running).clicked() {
            if let Some(f) = rfd::FileDialog::new().set_title("Choose the folder with the originals").pick_folder() {
                app.remover.originals = Some(f);
                app.remover.result = None;
            }
        }
        ui.add_space(22.0);
        if app.remover.running {
            let (done, total) = (app.remover.progress.0.load(Ordering::Relaxed), app.remover.progress.1.load(Ordering::Relaxed));
            let what = match app.remover.stage.load(Ordering::Relaxed) {
                0 => "Reading the originals",
                1 => "Pairing each photo with its original",
                _ => "Removing watermarks",
            };
            ui.label(theme::xp(format!("{what}  {} / {}", theme::thousands(done), theme::thousands(total)), 20.0, theme::INK));
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width().min(560.0), 6.0), Sense::hover());
            ui.painter().rect_filled(bar, 0.0, theme::RULE_DAY);
            if total > 0 {
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * (done as f32 / total as f32).clamp(0.0, 1.0), 6.0)), 0.0, theme::RED);
            }
            ui.add_space(10.0);
            if theme::button(ui, "Stop", None, Btn::OnDay, 40.0, None, true).clicked() {
                app.remover.stop.store(true, Ordering::Relaxed);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        } else if !app.remover.sent.is_empty() {
            let dir = default_dir(&find_first(&app.remover.sent));
            ui.label(theme::body(format!("The cleaned photos go to: {}", dir.display()), 14.0, theme::GRAPHITE));
        }
        let _ = p;
    });

    let enter = !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(Key::Enter));
    if go || enter {
        start(app, &ctx);
    }
}

/// The first watermarked photo's own path (a chosen folder stands for a photo inside it).
fn find_first(sent: &[PathBuf]) -> Vec<PathBuf> {
    match sent.first() {
        Some(p) if p.is_dir() => vec![Path::new(p).join("photo")],
        Some(p) => vec![p.clone()],
        None => Vec::new(),
    }
}
