//! Home (screen 1): hero, drop zone and recent shoots on the table; a dark side panel with Glim,
//! the Keep target, the shot type, the engine and DEVELOP.

use eframe::egui::{self, pos2, vec2, Align2, Key, Rect, Role, Sense, Stroke, StrokeKind, WidgetInfo};

use super::app::{self, BestOfBurstApp};
use super::glim::{self, Mood};
use super::theme::{self, Btn};
use super::marks;
use crate::rank::PRESETS;

const SHOT_TYPES: [(&str, &str); 4] = [
    ("General", "Balanced"),
    ("Portrait", "Exposure & skin"),
    ("Sports", "Sharpness first"),
    ("Landscape", "Contrast & colour"),
];

pub fn fmt_n(n: usize) -> String {
    theme::thousands(n)
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let p = app.pal();

    // Drop a folder (or a file inside it) on the window.
    let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
        i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
    });
    if let Some(path) = dropped.into_iter().next() {
        let folder = if path.is_dir() { path } else { path.parent().map(|x| x.to_path_buf()).unwrap_or(path) };
        app.choose_folder(folder, &ctx);
    }
    let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

    let info = app.folder_info.clone();
    let can_start = app.folder.is_some() && !app.checking_folder && info.as_ref().map(|i| i.error.is_none()).unwrap_or(false);
    let open_results = !app.retry_failed && info.as_ref().map(|i| i.new == 0 && i.has_rank).unwrap_or(false);

    let shoot = app.shoot_name();
    let mut go_home = false;
    let mut night = app.settings.night;
    app::chrome_bar(
        ui,
        "home_header",
        |ui| {
            go_home = app::logo_cell(ui);
            app::text_cell(ui, &shoot, true);
        },
        |ui| {
            ui.add_space(16.0);
            theme::table_toggle(ui, &mut night);
            ui.add_space(16.0);
            app::cell_rule(ui);
            app::text_cell(ui, &app::engine_status(app), false);
        },
    );
    app.settings.night = night;
    let _ = go_home;

    // Side panel: always dark chrome.
    let side = egui::Frame::new().fill(theme::INK).inner_margin(24.0);
    let mut primary = false;
    let mut rerun = false;
    egui::Panel::right("home_side").exact_size(400.0).resizable(false).frame(side).show(ui, |ui| {
        theme::dark_scope(ui);
        ui.spacing_mut().item_spacing.y = 10.0;
        let msg = if app.checking_folder {
            "Counting frames on the table...".to_string()
        } else if let Some(i) = &info {
            if let Some(e) = &i.error {
                e.clone()
            } else if i.new == 0 && i.has_rank {
                format!("All {} frames are measured. Open the contact sheet whenever you like.", fmt_n(i.total))
            } else {
                format!("Found {} frames: {} measured, {} new. Ready when you are.", fmt_n(i.total), fmt_n(i.analyzed), fmt_n(i.new))
            }
        } else {
            "Drop a shoot on the table and I'll circle the best frames.".to_string()
        };
        if glim::card(ui, Mood::Happy, true, &msg, 64.0, app.time(), app.reduced_motion).clicked() {
            app.open_corner();
        }
        ui.add_space(14.0);

        let keep_label = ui.label(theme::mono("Keep", 11.0, theme::ASH));
        let keep_ok = app.keep().is_some();
        let edit = egui::TextEdit::singleline(&mut app.keep_text)
            .font(theme::f_xp_black(64.0))
            .text_color(if keep_ok { theme::TABLE } else { theme::SAFELIGHT })
            .frame(egui::Frame::new())
            .desired_width(f32::INFINITY);
        ui.add(edit).labelled_by(keep_label.id).on_hover_text("A number of frames, or a percentage like 10%");
        let target_hint = match (app.keep(), &info) {
            (Some(k), Some(i)) if i.total > 0 => format!("About {} of {} frames", fmt_n(k.target(i.total)), fmt_n(i.total)),
            _ => "A number, or a percentage like 10%".into(),
        };
        ui.label(theme::mono(target_hint, 10.5, theme::ASH));
        ui.add_space(12.0);

        ui.label(theme::mono("Shot type", 11.0, theme::ASH));
        let mut sel = PRESETS.iter().position(|x| *x == app.preset).unwrap_or(0);
        if theme::segmented(ui, &SHOT_TYPES, &mut sel, 2, true) {
            app.preset = PRESETS[sel];
        }
        ui.add_space(12.0);

        ui.label(theme::mono("Engine", 11.0, theme::ASH));
        ui.horizontal(|ui| {
            ui.label(theme::xp(format!("CPU × {}", app.threads), 15.0, theme::TABLE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.visuals_mut().override_text_color = Some(theme::TABLE);
                if theme::button(ui, "+", None, Btn::MonoSmall, 40.0, Some(40.0), app.threads < 256).clicked() {
                    app.threads += 1;
                }
                if theme::button(ui, "-", None, Btn::MonoSmall, 40.0, Some(40.0), app.threads > 1).clicked() {
                    app.threads -= 1;
                }
            });
        });
        ui.label(theme::body("Decoding runs on the CPU. The GPU only draws this window.", 13.0, theme::ASH));
        if info.as_ref().map(|i| i.failed > 0).unwrap_or(false) {
            ui.add_space(6.0);
            ui.checkbox(&mut app.retry_failed, theme::body("Retry frames that failed before (after installing a codec)", 13.0, theme::TABLE));
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if let Some((text, err, _)) = &app.message {
                ui.label(theme::body(text, 13.0, if *err { theme::SAFELIGHT_SOFT } else { theme::TABLE }));
            }
            if open_results {
                ui.visuals_mut().override_text_color = Some(theme::TABLE);
                if theme::button(ui, "Develop again", None, Btn::OnDark, 40.0, Some(ui.available_width()), can_start).clicked() {
                    rerun = true;
                }
            }
            let label = if open_results { "Open contact sheet" } else { "Develop" };
            if theme::button(ui, label, Some("enter"), Btn::Primary, 64.0, Some(ui.available_width()), can_start).clicked() {
                primary = true;
            }
        });
    });

    // Main: the light (or night) table.
    let main = egui::Frame::new().fill(p.surface).inner_margin(48.0);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            marks::burst(ui, r, p.night);
            ui.add_space(6.0);
            ui.label(theme::mono("BestOfBurst · offline photo culler", 11.0, p.muted));
        });
        ui.add_space(10.0);
        hero(ui, p);
        ui.add_space(14.0);
        ui.label(theme::body(
            "Drop a shoot. BestOfBurst measures every frame, stacks the bursts and circles the best one of each moment. You only look where it's a close call.",
            16.0,
            p.muted,
        ));
        ui.add_space(28.0);

        // Drop zone.
        let w = ui.available_width();
        let (zone, zresp) = ui.allocate_exact_size(vec2(w, 150.0), Sense::click());
        zresp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Choose a shoot folder"));
        let border = if hovering { p.mark } else { p.on_surface };
        if hovering {
            ui.painter().rect_filled(zone, 0.0, p.mark.gamma_multiply(0.08));
            ui.painter().rect_stroke(zone, 0.0, Stroke::new(2.0, border), StrokeKind::Inside);
        } else {
            dashed_rect(ui, zone, border);
        }
        theme::text_at(ui, zone.center() - vec2(0.0, 22.0), Align2::CENTER_CENTER, "DROP A SHOOT FOLDER", theme::f_xp(22.0), p.on_surface, 0.6);
        let sub = match &app.folder {
            Some(f) => f.display().to_string(),
            None => "or click to choose one".into(),
        };
        theme::text_at(ui, zone.center() + vec2(0.0, 16.0), Align2::CENTER_CENTER, &sub, theme::f_mono(11.0), p.muted, 0.4);
        if app.checking_folder {
            theme::text_at(ui, zone.center() + vec2(0.0, 40.0), Align2::CENTER_CENTER, "COUNTING FRAMES...", theme::f_mono(10.5), p.mark, 0.8);
        }
        theme::focus_ring(ui, &zresp);
        if zresp.clicked() {
            if let Some(f) = rfd::FileDialog::new().set_title("Choose a shoot folder").pick_folder() {
                app.choose_folder(f, &ctx);
            }
        }
        ui.add_space(36.0);

        if !app.recent.is_empty() {
            ui.label(theme::mono("Recent shoots", 11.0, p.muted));
            ui.add_space(4.0);
            let recent = app.recent.clone();
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                for r in recent {
                    if recent_row(ui, &r, p, app.folder.as_ref() == Some(&r)) {
                        app.choose_folder(r, &ctx);
                    }
                }
            });
        }
    });

    let enter = !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(Key::Enter));
    if (primary || enter) && can_start {
        if open_results { app.open_results(&ctx) } else { app.start(&ctx) }
    } else if rerun {
        app.start(&ctx);
    }
}

/// "TEN THOUSAND FRAMES. / CIRCLE THE BEST." with the hero circle around "CIRCLE".
fn hero(ui: &mut egui::Ui, p: theme::Pal) {
    let w = ui.available_width();
    let size = theme::fit_size(ui, "TEN THOUSAND FRAMES.", 84.0, w, false);
    let sp = -0.02 * size;
    let l1 = theme::galley(ui, "TEN THOUSAND FRAMES.", theme::f_xp(size), p.on_surface, sp);
    let a = theme::galley(ui, "CIRCLE", theme::f_xp(size), p.on_surface, sp);
    let b = theme::galley(ui, " THE BEST.", theme::f_xp(size), p.on_surface, sp);
    let line_h = size * 0.95;
    let (rect, _) = ui.allocate_exact_size(vec2(w, line_h * 2.0 + 10.0), Sense::hover());
    let painter = ui.painter();
    painter.galley(rect.min, l1, p.on_surface);
    let y2 = rect.min.y + line_h;
    let x0 = rect.min.x + size * 0.3;
    let aw = a.size();
    painter.galley(pos2(x0, y2), a, p.on_surface);
    painter.galley(pos2(x0 + aw.x, y2), b, p.on_surface);
    let rw = aw.x * 1.4;
    let rh = aw.y * 1.55;
    let circle = Rect::from_min_size(pos2(x0 - rw * 0.16, y2 - rh * 0.24), vec2(rw, rh));
    marks::hero(ui, circle, p.mark, (size / 17.0).clamp(2.5, 5.0), 1.0);
}

fn dashed_rect(ui: &egui::Ui, r: Rect, color: egui::Color32) {
    let s = Stroke::new(2.0, color);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        ui.painter().extend(egui::Shape::dashed_line(&[w[0], w[1]], s, 10.0, 6.0));
    }
}

fn recent_row(ui: &mut egui::Ui, path: &std::path::Path, p: theme::Pal, current: bool) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
    resp.widget_info(|| WidgetInfo::labeled(Role::Link, true, format!("Resume {name}")));
    let painter = ui.painter();
    painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, p.rule));
    if resp.hovered() {
        painter.rect_filled(rect.shrink2(vec2(0.0, 1.0)), 0.0, p.rule.gamma_multiply(0.35));
    }
    theme::text_at(ui, rect.left_top() + vec2(0.0, 12.0), Align2::LEFT_TOP, &name.to_uppercase(), theme::f_xp(18.0), p.on_surface, 0.4);
    theme::text_at(ui, rect.left_top() + vec2(0.0, 40.0), Align2::LEFT_TOP, &path.display().to_string(), theme::f_mono(10.5), p.muted, 0.3);
    let label = if current { "SELECTED" } else { "RESUME →" };
    theme::text_at(ui, rect.right_center(), Align2::RIGHT_CENTER, label, theme::f_xp(13.0), p.mark, 0.5);
    theme::focus_ring(ui, &resp);
    resp.clicked()
}
