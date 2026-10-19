//! Home: choose a folder, set Keep and Preset, press Start (or Open results).

use eframe::egui::{self, Align, Layout, RichText};

use super::app::{BestOfBurstApp, ACCENT, DIM, REJECT};
use crate::rank::PRESETS;

pub fn show(app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();

    // Drop a folder (or a file inside it) on the window.
    let dropped: Vec<std::path::PathBuf> =
        ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
    if let Some(p) = dropped.into_iter().next() {
        let folder = if p.is_dir() { p } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) };
        app.choose_folder(folder, &ctx);
    }
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(36.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("BestOfBurst").size(34.0).strong());
            ui.label(RichText::new("Turn a folder of photos into a picks folder of the best shots.").color(DIM));
        });
        ui.add_space(28.0);

        let width = 620.0;
        ui.vertical_centered(|ui| {
            ui.set_max_width(width);
            egui::Frame::new()
                .fill(if hovering { ACCENT.linear_multiply(0.15) } else { super::app::CARD })
                .stroke(egui::Stroke::new(1.0, if hovering { ACCENT } else { egui::Color32::from_gray(60) }))
                .corner_radius(8.0)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.set_width(width - 40.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("Choose folder…").size(16.0))).clicked() {
                            if let Some(f) = rfd::FileDialog::new().set_title("Choose a photo folder").pick_folder() {
                                app.choose_folder(f, &ctx);
                            }
                        }
                        ui.label(RichText::new("or drop a folder on the window").color(DIM));
                    });
                    ui.add_space(10.0);
                    match &app.folder {
                        Some(f) => {
                            ui.label(RichText::new(f.display().to_string()).size(15.0).strong());
                        }
                        None => {
                            ui.label(RichText::new("No folder chosen").color(DIM));
                        }
                    }
                    if app.checking_folder {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Counting photos…");
                        });
                    } else if let Some(info) = &app.folder_info {
                        if let Some(e) = &info.error {
                            ui.colored_label(REJECT, e);
                        } else {
                            let mut text = format!(
                                "Found {} photos: {} analyzed, {} new",
                                fmt_n(info.total),
                                fmt_n(info.analyzed),
                                fmt_n(info.new)
                            );
                            if info.failed > 0 {
                                text.push_str(&format!(", {} failed earlier", fmt_n(info.failed)));
                            }
                            ui.label(text);
                        }
                    }

                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        let keep_label = ui.label("Keep");
                        let keep_ok = app.keep().is_some();
                        let edit = egui::TextEdit::singleline(&mut app.keep_text).desired_width(80.0);
                        let edit = if keep_ok { edit } else { edit.text_color(REJECT) };
                        ui.add(edit)
                            .labelled_by(keep_label.id)
                            .on_hover_text("A number of photos, or a percentage like 10%");
                        ui.add_space(16.0);
                        let preset_label = ui.label("Preset");
                        egui::ComboBox::from_id_salt("home_preset")
                            .selected_text(app.preset.label())
                            .show_ui(ui, |ui| {
                                for p in PRESETS {
                                    ui.selectable_value(&mut app.preset, p, p.label());
                                }
                            })
                            .response
                            .labelled_by(preset_label.id);
                        ui.add_space(16.0);
                        let threads_label = ui.label("Threads");
                        ui.add(egui::DragValue::new(&mut app.threads).range(1..=256)).labelled_by(threads_label.id);
                    });

                    ui.add_space(16.0);
                    let info = app.folder_info.clone();
                    let can_start = app.folder.is_some() && !app.checking_folder && info.as_ref().map(|i| i.error.is_none()).unwrap_or(false);
                    if info.as_ref().map(|i| i.failed > 0).unwrap_or(false) {
                        ui.checkbox(&mut app.retry_failed, "Retry files that failed before (after installing a codec)");
                    }
                    let open_results = !app.retry_failed && info.as_ref().map(|i| i.new == 0 && i.has_rank).unwrap_or(false);
                    ui.horizontal(|ui| {
                        let label = if open_results { "Open results" } else { "Start" };
                        let btn = egui::Button::new(RichText::new(label).size(17.0).strong().color(egui::Color32::WHITE))
                            .fill(ACCENT)
                            .min_size(egui::vec2(160.0, 36.0));
                        if ui.add_enabled(can_start, btn).clicked() {
                            if open_results {
                                app.open_results(&ctx);
                            } else {
                                app.start(&ctx);
                            }
                        }
                        if open_results {
                            if ui.button("Re-run analysis check").on_hover_text("Scan again and analyze anything new").clicked() {
                                app.start(&ctx);
                            }
                        } else if info.as_ref().map(|i| i.has_rank).unwrap_or(false)
                            && ui.button("Open previous results").clicked()
                        {
                            app.open_results(&ctx);
                        }
                    });
                });

            if let Some((text, error, _)) = &app.message {
                ui.add_space(10.0);
                ui.colored_label(if *error { REJECT } else { ACCENT }, text);
            }

            if !app.recent.is_empty() {
                ui.add_space(24.0);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.set_max_width(width);
                    ui.label(RichText::new("Recent folders").color(DIM));
                    let recent = app.recent.clone();
                    for r in recent {
                        if ui.link(r.display().to_string()).clicked() {
                            app.choose_folder(r, &ctx);
                        }
                    }
                });
            }
        });
    });
}

pub fn fmt_n(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
