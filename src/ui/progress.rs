//! Processing (screen 2): dark full screen with a giant frame counter, the step list, a live film
//! strip, live failures, PAUSE & SAVE and OPEN CONTACT SHEET.

use eframe::egui::{self, pos2, vec2, Align2, Key, Rect, Sense, Stroke};

use super::app::{self, BestOfBurstApp};
use super::glim::Mood;
use super::textures::Loaded;
use super::theme::{self, Btn};

fn clock(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let shoot = app.shoot_name();
    let status = app::engine_status(app);
    let mut glim_clicked = false;
    app::chrome_bar(
        ui,
        "progress_header",
        |ui| {
            let _ = app::logo_cell(ui);
            app::text_cell(ui, &shoot, true);
        },
        |ui| {
            app::text_cell(ui, &status, false);
            let (rect, resp) = ui.allocate_exact_size(vec2(64.0, app::HEADER_H), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, "Glim: open Glim's corner"));
            super::glim::paint(ui, Rect::from_center_size(rect.center(), vec2(42.0, 42.0)), Mood::Sleepy, true, app.time(), app.reduced_motion, 1.0);
            glim_clicked = resp.clicked();
            app::cell_rule(ui);
        },
    );
    if glim_clicked {
        app.open_corner();
    }

    let frame = egui::Frame::new().fill(theme::INK).inner_margin(48.0);
    let mut open_sheet = false;
    let mut pause = false;
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        theme::dark_scope(ui);
        let p = &app.prog;
        let finished = p.finished;

        if p.missing_codec > 0 {
            let frame = egui::Frame::new().fill(theme::NUDGE_BG).inner_margin(egui::Margin { left: 18, right: 12, top: 10, bottom: 10 });
            let r = frame
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(theme::mono(format!("{} frames need a Windows codec", fmt(p.missing_codec)), 11.0, theme::SAFELIGHT_SOFT));
                    ui.label(theme::body(
                        "From the Microsoft Store: HEIF Image Extensions + HEVC Video Extensions (HEIC), AV1 Video Extension (AVIF), Raw Image Extension (RAW). Then develop again with failed frames retried.",
                        14.0,
                        theme::TABLE,
                    ));
                })
                .response
                .rect;
            ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())), 0.0, theme::SAFELIGHT);
            ui.add_space(20.0);
        }

        let step = if finished {
            4
        } else if p.scanning {
            0
        } else if p.ranking {
            3
        } else {
            1
        };
        let eyebrow = match step {
            0 => "Scanning the shoot",
            3 => "Grouping and ranking",
            4 => "Developed",
            _ => "Measuring frames",
        };
        ui.label(theme::mono(eyebrow, 11.0, theme::SAFELIGHT));
        ui.add_space(6.0);

        // Giant counter.
        let (count, of) = if finished {
            let picks = p.summary.as_ref().map(|s| s.picks).unwrap_or(0);
            (fmt(picks), format!("keepers circled from {} frames", fmt(p.total_files)))
        } else if p.scanning {
            ("0".to_string(), "frames found so far".to_string())
        } else {
            (fmt(p.done), format!("/ {} new frames · {} already measured", fmt(p.total), fmt(p.total_files.saturating_sub(p.total))))
        };
        let w = ui.available_width();
        let size = theme::fit_size(ui, &count, 148.0, w * 0.9, false);
        ui.label(theme::xp(&count, size, theme::TABLE).line_height(Some(size * 0.86)));
        ui.label(theme::mono(of, 11.0, theme::ASH));
        ui.add_space(18.0);

        // Progress: ruler track with a safelight fill.
        let frac = if finished { 1.0 } else if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 };
        let (bar, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
        let y = bar.center().y;
        let painter = ui.painter();
        painter.line_segment([pos2(bar.min.x, y), pos2(bar.max.x, y)], Stroke::new(2.0, theme::RULE_STRONG));
        for k in 0..=10 {
            let x = bar.min.x + bar.width() * k as f32 / 10.0;
            painter.line_segment([pos2(x, y - 5.0), pos2(x, y)], Stroke::new(1.0, theme::RULE_STRONG));
        }
        painter.rect_filled(Rect::from_min_max(pos2(bar.min.x, y - 3.0), pos2(bar.min.x + bar.width() * frac, y + 3.0)), 0.0, theme::SAFELIGHT);
        ui.add_space(4.0);
        ui.label(theme::mono(
            format!(
                "{:.1} frames/s   ·   ETA {}   ·   {} failed",
                p.per_sec,
                if finished { "done".into() } else { p.eta_s.map(clock).unwrap_or_else(|| "-".into()) },
                fmt(p.failed)
            ),
            10.5,
            theme::ASH,
        ));
        ui.add_space(24.0);

        // Step list: scan, measure, group, rank.
        ui.horizontal(|ui| {
            for (i, name) in ["Scan", "Measure", "Group", "Rank"].iter().enumerate() {
                let (color, tag) = if i < step {
                    (theme::ASH, "done")
                } else if i == step {
                    (theme::SAFELIGHT, "now")
                } else {
                    (theme::GRAPHITE, "next")
                };
                ui.vertical(|ui| {
                    ui.label(theme::mono(format!("0{} · {tag}", i + 1), 10.5, color));
                    ui.label(theme::xp(name, 20.0, color));
                });
                ui.add_space(36.0);
            }
        });
        ui.add_space(24.0);

        // Live film strip of the latest frames.
        ui.label(theme::mono("Latest frames", 10.5, theme::ASH));
        let thumbs: Vec<_> = p.recent_thumbs.iter().cloned().collect();
        let (strip, _) = ui.allocate_exact_size(vec2(w, 120.0), Sense::hover());
        theme::film_well(ui, strip);
        let cell = vec2(150.0, 100.0);
        for (k, t) in thumbs.iter().enumerate() {
            let r = Rect::from_min_size(strip.min + vec2(10.0 + k as f32 * (cell.x + 8.0), 10.0), cell);
            if r.max.x > strip.max.x {
                break;
            }
            if let Loaded::Ready(id, size) = app.textures.thumb(t) {
                ui.painter().image(id, theme::fit(size, r), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
        }
        if let Some(last) = &app.prog.last {
            ui.label(egui::RichText::new(last.display().to_string()).font(theme::f_mono(10.5)).color(theme::EDGE));
        }

        // Live failures.
        let p = &app.prog;
        for w in &p.warnings {
            ui.label(theme::body(w, 13.0, theme::SAFELIGHT_SOFT));
        }
        if !p.failures.is_empty() {
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                for (path, err) in p.failures.iter().rev().take(40) {
                    ui.label(egui::RichText::new(format!("{}  {}", path.display(), err)).font(theme::f_mono(10.0)).color(theme::ASH));
                }
            });
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.visuals_mut().override_text_color = Some(theme::TABLE);
                if theme::button(ui, "Open contact sheet →", Some("enter"), Btn::Primary, 64.0, None, finished).clicked() {
                    open_sheet = true;
                }
                ui.add_space(12.0);
                let label = if p.stopping { "Pausing..." } else { "Pause & save" };
                if theme::button(ui, label, Some("Esc"), Btn::OnDark, 64.0, None, !finished && !p.stopping).clicked() {
                    pause = true;
                }
            });
        });
    });

    let keys_free = !ctx.egui_wants_keyboard_input();
    if keys_free && app.prog.finished && ctx.input(|i| i.key_pressed(Key::Enter)) {
        open_sheet = true;
    }
    if keys_free && !app.prog.finished && ctx.input(|i| i.key_pressed(Key::Escape)) {
        pause = true;
    }
    if open_sheet {
        app.open_contact_sheet();
    } else if pause && !app.prog.stopping {
        app.request_stop();
    }
    let _ = Align2::LEFT_TOP;
}

fn fmt(n: usize) -> String {
    theme::thousands(n)
}
