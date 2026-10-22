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

    live_panel(app, ui);

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
                    ui.label(theme::mono(format!("{} photos need a free Microsoft add-on to open", fmt(p.missing_codec)), 11.0, theme::SAFELIGHT_SOFT));
                    ui.label(theme::body(
                        "Get them free from the Microsoft Store: HEIF Image Extensions and HEVC Video Extensions (iPhone HEIC photos), AV1 Video Extension (AVIF), Raw Image Extension (some RAW files). Then start again and tick \"Try again on photos that couldn't be opened\".",
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
            0 => "Finding your photos",
            3 => "Picking the best of each moment",
            4 => "All done",
            _ => "Checking every photo",
        };
        ui.label(theme::mono(eyebrow, 11.0, theme::SAFELIGHT));
        ui.add_space(6.0);

        // Giant counter.
        let (count, of) = if finished {
            let picks = p.summary.as_ref().map(|s| s.picks).unwrap_or(0);
            (fmt(picks), format!("keepers circled from {} photos", fmt(p.total_files)))
        } else if p.scanning {
            ("0".to_string(), "looking through the folder".to_string())
        } else {
            (fmt(p.done), format!("of {} new photos checked · {} done before", fmt(p.total), fmt(p.total_files.saturating_sub(p.total))))
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
                "{:.1} photos a second   ·   {}   ·   {} couldn't be opened",
                p.per_sec,
                if finished { "finished".into() } else { p.eta_s.map(|s| format!("about {} left", clock(s))).unwrap_or_else(|| "working out the time left".into()) },
                fmt(p.failed)
            ),
            10.5,
            theme::ASH,
        ));
        ui.add_space(24.0);

        // Step list: scan, measure, group, rank.
        ui.horizontal(|ui| {
            for (i, name) in ["Find photos", "Check quality", "Stack bursts", "Pick the best"].iter().enumerate() {
                let (color, tag) = if i < step {
                    (theme::ASH, "done")
                } else if i == step {
                    (theme::SAFELIGHT, "now")
                } else {
                    (theme::GRAPHITE, "next")
                };
                ui.vertical(|ui| {
                    ui.label(theme::mono(format!("0{} · {tag}", i + 1), 10.5, color));
                    ui.label(theme::xp(name, 16.0, color));
                });
                ui.add_space(36.0);
            }
        });
        ui.add_space(24.0);

        // Live film strip of the latest frames.
        ui.label(theme::mono("Just checked", 10.5, theme::ASH));
        let thumbs: Vec<_> = p.recent_thumbs.iter().cloned().collect();
        let (strip, _) = ui.allocate_exact_size(vec2(w, 120.0), Sense::hover());
        theme::film_well(ui, strip);
        let cell = vec2(150.0, 100.0);
        for (k, t) in thumbs.iter().enumerate() {
            let r = Rect::from_min_size(strip.min + vec2(10.0 + k as f32 * (cell.x + 8.0), 10.0), cell);
            if r.max.x > strip.max.x {
                break;
            }
            if let Loaded::Ready(id, size) = app.textures.thumb(t, None) {
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

/// "Engine live": what the processor and graphics card are doing right now.
fn live_panel(app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    let snap = app.sys();
    let st = crate::stats::STATS.view();
    let p = theme::pal(true);
    let frame = egui::Frame::new().fill(theme::FILM).inner_margin(24.0);
    egui::Panel::right("engine_live").exact_size(380.0).resizable(false).frame(frame).show(ui, |ui| {
        theme::dark_scope(ui);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width() - 14.0);
            let gpu_on = st.engine == 2;
            ui.horizontal(|ui| {
                ui.label(theme::xp("Your computer now", 18.0, theme::TABLE));
                ui.add_space(6.0);
                let _ = super::help::toggle(ui, "live", "Your computer now");
            });
            if ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(help_id(), || false)) {
                super::help::card(ui, super::help::LIVE, true);
            }
            ui.label(theme::mono(if gpu_on { "Graphics card + processor" } else { "Processor only" }, 10.5, theme::SAFELIGHT));
            ui.add_space(10.0);

            // Processor.
            ui.label(theme::mono("Processor", 11.0, theme::ASH));
            theme::ruler(ui, p, "Whole computer", snap.cpu_total / 100.0, None, &format!("{:.0}%", snap.cpu_total));
            theme::ruler(ui, p, "BestOfBurst", snap.cpu_app / 100.0, None, &format!("{:.0}%", snap.cpu_app));
            stat(ui, "Cores", &format!("{}", snap.physical));
            stat(ui, "Photos in progress", &format!("{} of {}", st.busy, st.threads.max(app.threads)));
            stat(ui, "  being opened", &format!("{}", st.decoding));
            if gpu_on {
                stat(ui, "  being checked on the graphics card", &format!("{}", st.on_gpu));
            }
            ui.add_space(12.0);

            // Graphics card.
            ui.label(theme::mono("Graphics card", 11.0, theme::ASH));
            let (gname, gdetail) = app::gpu_line(&snap);
            ui.label(theme::body(&gname, 14.0, theme::TABLE));
            if !gdetail.is_empty() {
                ui.label(egui::RichText::new(&gdetail).font(theme::f_mono(10.0)).color(theme::ASH));
            }
            if let Some(u) = snap.gpu_total {
                theme::ruler(ui, p, "Whole card", u / 100.0, None, &format!("{u:.0}%"));
            }
            if let Some(u) = snap.gpu_app {
                theme::ruler(ui, p, "BestOfBurst", u / 100.0, None, &format!("{u:.0}%"));
            }
            let cores = snap.gpu_cores.or_else(|| match crate::gpu::status() {
                Some(Ok(i)) => crate::sysmon::known_cores(&i.name),
                _ => None,
            });
            if let Some(c) = cores {
                let busy = snap.gpu_app.or(snap.gpu_total).unwrap_or(0.0) / 100.0 * c as f32;
                stat(ui, "Cores", &format!("{} · about {} busy", theme::thousands(c as usize), theme::thousands(busy.round() as usize)));
            } else {
                stat(ui, "Cores", "not shown by this card");
            }
            if let (Some(u), Some(t)) = (snap.gpu_mem_used_mb, snap.gpu_mem_total_mb) {
                theme::ruler(ui, p, "Graphics memory", u as f32 / t.max(1) as f32, None, &format!("{} / {} MB", theme::thousands(u as usize), theme::thousands(t as usize)));
            }

            ui.add_space(12.0);

            // Per photo.
            ui.label(theme::mono("Time per photo", 11.0, theme::ASH));
            stat(ui, "Opening it", &format!("{:.2} s", st.decode_ms_avg / 1000.0));
            if st.gpu_photos > 0 {
                stat(ui, "Checking it (graphics card)", &format!("{:.2} s", st.gpu_ms_avg / 1000.0));
            }
            if st.cpu_photos > 0 {
                stat(ui, "Checking it (processor)", &format!("{:.2} s", st.cpu_ms_avg / 1000.0));
            }
            stat(ui, "Checked by", &format!("graphics {} · processor {}", theme::thousands(st.gpu_photos as usize), theme::thousands(st.cpu_photos as usize)));
            if st.gpu_fallbacks > 0 {
                stat(ui, "Handed to the processor", &format!("{}", st.gpu_fallbacks));
            }
        });
    });
}

fn help_id() -> egui::Id {
    #[allow(deprecated)]
    egui::Id::new(("bob_help", "live"))
}

fn stat(ui: &mut egui::Ui, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(theme::body(k, 13.0, theme::ASH));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(v).font(theme::f_mono(10.5)).color(theme::TABLE));
        });
    });
}
