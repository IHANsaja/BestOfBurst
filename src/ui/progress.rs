//! Progress: count, bar, speed, ETA, last photo, live failures and Stop and save.

use eframe::egui::{self, RichText};

use super::app::{BestOfBurstApp, ACCENT, DIM, MAYBE, REJECT};
use super::home::fmt_n;
use super::textures::Loaded;

fn clock(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(30.0);
        let width = 720.0;
        ui.vertical_centered(|ui| {
            ui.set_max_width(width);
            if let Some(f) = &app.folder {
                ui.label(RichText::new(f.display().to_string()).color(DIM));
            }
            ui.add_space(8.0);

            if app.prog.missing_codec > 0 {
                egui::Frame::new()
                    .fill(MAYBE.linear_multiply(0.18))
                    .stroke(egui::Stroke::new(1.0, MAYBE))
                    .corner_radius(6.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(width - 20.0);
                        ui.colored_label(
                            MAYBE,
                            format!(
                                "{} photos need a Windows codec. From the Microsoft Store install: HEIF Image Extensions + \
                                 HEVC Video Extensions (HEIC), AV1 Video Extension (AVIF), Raw Image Extension (RAW). \
                                 Then run again with failed files retried.",
                                fmt_n(app.prog.missing_codec)
                            ),
                        );
                    });
                ui.add_space(8.0);
            }

            let p = &app.prog;
            if p.scanning {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Scanning folder…").size(18.0));
                });
            } else if p.ranking {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Ranking and writing the picks folder…").size(18.0));
                });
            } else {
                ui.label(
                    RichText::new(format!("{} / {} photos", fmt_n(p.done), fmt_n(p.total)))
                        .size(26.0)
                        .strong(),
                );
                if p.total_files > p.total {
                    ui.label(
                        RichText::new(format!("{} already analyzed", fmt_n(p.total_files - p.total))).color(DIM),
                    );
                }
            }
            ui.add_space(8.0);
            let frac = if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).fill(ACCENT).desired_height(14.0));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(format!("{:.1} photos/s", p.per_sec));
                ui.separator();
                ui.label(format!("ETA {}", p.eta_s.map(clock).unwrap_or_else(|| "–".into())));
                ui.separator();
                ui.label(format!("{} threads", app.threads));
                if p.failed > 0 {
                    ui.separator();
                    ui.colored_label(REJECT, format!("{} failed", fmt_n(p.failed)));
                }
            });

            ui.add_space(16.0);
            let thumb = p.last_thumb.clone();
            let last = p.last.clone();
            if let Some(t) = thumb {
                ui.label(RichText::new("Last analyzed").color(DIM));
                let loaded = app.textures.thumb(&t);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(320.0, 240.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 6.0, super::app::DEEP);
                if let Loaded::Ready(id, size) = loaded {
                    let fit = super::review::fit(size, rect);
                    ui.painter().image(id, fit, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                if let Some(l) = last {
                    ui.label(l.display().to_string());
                }
            }

            ui.add_space(16.0);
            let stopping = app.prog.stopping;
            let label = if stopping { "Stopping…" } else { "Stop and save" };
            if ui.add_enabled(!stopping, egui::Button::new(RichText::new(label).size(15.0)).min_size(egui::vec2(150.0, 30.0))).clicked() {
                app.request_stop();
            }

            for w in &app.prog.warnings {
                ui.colored_label(MAYBE, w);
            }
            if !app.prog.failures.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new("Failures").color(DIM));
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    for (path, err) in app.prog.failures.iter().rev().take(50) {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(REJECT, path.display().to_string());
                            ui.label(RichText::new(err).color(DIM).small());
                        });
                    }
                });
            }
        });
    });
}
