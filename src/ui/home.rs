//! Home (screen 1): hero, drop zone and recent shoots on the table; a dark side panel with Glim,
//! the Keep target, the shot type, the engine and DEVELOP.

use eframe::egui::{self, pos2, vec2, Align2, Key, Rect, Role, Sense, Stroke, StrokeKind, WidgetInfo};

use super::app::{self, BestOfBurstApp};
use super::help;
use crate::engine::EngineChoice;
use super::glim::{self, Mood};
use super::theme::{self, Btn};
use super::marks;
use crate::rank::PRESETS;

const ENGINES: [(&str, &str); 3] = [("Automatic", "Best for this PC"), ("Graphics", "Fastest"), ("Processor", "Any computer")];

/// One hardware line: label, name, details; a safelight dot when it is in use.
fn hw_row(ui: &mut egui::Ui, kind: &str, name: &str, detail: &str, used: bool) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(48.0, 18.0), Sense::hover());
        theme::text_at(ui, r.left_center(), Align2::LEFT_CENTER, kind, theme::f_mono_semi(10.5), if used { theme::SAFELIGHT } else { theme::ASH }, 0.8);
        ui.vertical(|ui| {
            ui.label(theme::body(name, 14.0, theme::TABLE));
            if !detail.is_empty() {
                ui.label(egui::RichText::new(detail).font(theme::f_mono(10.0)).color(theme::ASH));
            }
        });
    });
}

/// The Keep input: a clearly editable box with a pencil, plus quick picks.
fn keep_field(ui: &mut egui::Ui, text: &mut String) {
    let ok = crate::rank::Keep::parse(text).is_some();
    let w = ui.available_width();
    let edit_id = ui.make_persistent_id("keep_edit");
    let row = ui.horizontal(|ui| {
        ui.add_space(14.0);
        let edit = egui::TextEdit::singleline(text)
            .id(edit_id)
            .font(theme::f_xp_black(48.0))
            .text_color(if ok { theme::TABLE } else { theme::SAFELIGHT })
            .frame(egui::Frame::new())
            .margin(egui::Margin::symmetric(0, 6))
            .desired_width(w - 76.0);
        let resp = ui.add(edit).on_hover_text("Click to change how many photos to keep");
        let (pen, pen_resp) = ui.allocate_exact_size(vec2(44.0, 56.0), Sense::click());
        pen_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Change photos to keep"));
        if pen_resp.clicked() {
            resp.request_focus();
        }
        (resp, pen, pen_resp.hovered())
    });
    let (resp, pen, pen_hover) = row.inner;
    let rect = row.response.rect.with_min_x(row.response.rect.min.x).expand2(vec2(0.0, 2.0));
    let focused = resp.has_focus();
    let hover = resp.hovered() || pen_hover;
    let border = if focused { theme::CYAN } else if hover { theme::TABLE } else { theme::RULE_STRONG };
    let width = if focused { 2.5 } else { 2.0 };
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(width, border), StrokeKind::Inside);
    // Pencil: a grease pencil, drawn.
    let c = pen.center();
    let col = if focused || hover { theme::TABLE } else { theme::ASH };
    let s = Stroke::new(2.0, col);
    let a = c + vec2(-7.0, 7.0);
    let b = c + vec2(6.0, -6.0);
    ui.painter().line_segment([a, b], Stroke::new(5.0, col));
    ui.painter().line_segment([b, b + vec2(2.5, -2.5)], Stroke::new(5.0, theme::SAFELIGHT));
    ui.painter().line_segment([a + vec2(-3.0, 3.0), a], s);
    if !focused {
        theme::text_at(ui, rect.right_bottom() + vec2(-8.0, -6.0), Align2::RIGHT_BOTTOM, "CLICK TO CHANGE", theme::f_mono(9.5), col, 0.6);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.visuals_mut().override_text_color = Some(theme::TABLE);
        for pick in ["5%", "10%", "20%", "100", "300"] {
            let label = if pick.ends_with('%') { pick.to_string() } else { format!("{pick}") };
            if theme::button(ui, &label, None, theme::Btn::MonoSmall, 32.0, Some(56.0), true).clicked() {
                *text = pick.to_string();
            }
        }
    });
}

const SHOT_TYPES: [(&str, &str); 4] = [
    ("General", "Everyday mix"),
    ("Portrait", "Faces & skin"),
    ("Sports", "Fast action"),
    ("Landscape", "Scenery & colour"),
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
        ui.spacing_mut().item_spacing.y = 7.0;

        // Actions stay pinned to the bottom; the settings above scroll.
        egui::Panel::bottom("home_actions").frame(egui::Frame::new().fill(theme::INK)).show(ui, |ui| {
            theme::dark_scope(ui);
            ui.add_space(10.0);
            let label = if open_results { "Open contact sheet" } else { "Start culling" };
            if theme::button(ui, label, Some("enter"), Btn::Primary, 64.0, Some(ui.available_width()), can_start).clicked() {
                primary = true;
            }
            if open_results {
                ui.visuals_mut().override_text_color = Some(theme::TABLE);
                if theme::button(ui, "Check for new photos", None, Btn::OnDark, 40.0, Some(ui.available_width()), can_start).clicked() {
                    rerun = true;
                }
            }
            if let Some((text, err, _)) = &app.message {
                ui.label(theme::body(text, 13.0, if *err { theme::SAFELIGHT_SOFT } else { theme::TABLE }));
            }
        });

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(352.0);
            let msg = if app.checking_folder {
                "Counting the photos in this folder...".to_string()
            } else if let Some(i) = &info {
                if let Some(e) = &i.error {
                    e.clone()
                } else if i.new == 0 && i.has_rank {
                    format!("All {} photos are checked. Open the contact sheet whenever you like.", fmt_n(i.total))
                } else {
                    format!("Found {} photos: {} already checked, {} new. Ready when you are.", fmt_n(i.total), fmt_n(i.analyzed), fmt_n(i.new))
                }
            } else {
                "Drop a shoot here and I'll circle the best photos.".to_string()
            };
            if glim::card(ui, Mood::Happy, true, &msg, 48.0, app.time(), app.reduced_motion).clicked() {
                app.open_corner();
            }
            ui.add_space(10.0);

            help::labelled(ui, "Photos to keep", "keep", help::KEEP, true, theme::ASH);
            keep_field(ui, &mut app.keep_text);
            let target_hint = match (app.keep(), &info) {
                (Some(k), Some(i)) if i.total > 0 => {
                    format!("That's about {} of your {} photos", fmt_n(k.target(i.total)), fmt_n(i.total))
                }
                (None, _) => "Type a number (like 300) or a share (like 10%)".into(),
                _ => "A number (like 300) or a share of the shoot (like 10%)".into(),
            };
            ui.label(theme::body(target_hint, 13.0, if app.keep().is_some() { theme::ASH } else { theme::SAFELIGHT_SOFT }));
            ui.add_space(8.0);

            help::labelled(ui, "Type of shoot", "shot", help::SHOT_TYPE, true, theme::ASH);
            let mut sel = PRESETS.iter().position(|x| *x == app.preset).unwrap_or(0);
            if theme::segmented(ui, &SHOT_TYPES, &mut sel, 2, true) {
                app.preset = PRESETS[sel];
            }
            ui.add_space(8.0);

            help::labelled(ui, "Speed", "engine", help::ENGINE, true, theme::ASH);
            let engines = [EngineChoice::Auto, EngineChoice::Gpu, EngineChoice::Cpu];
            let mut esel = engines.iter().position(|e| *e == app.settings.engine).unwrap_or(0);
            if theme::segmented(ui, &ENGINES, &mut esel, 3, true) {
                app.settings.engine = engines[esel];
            }
            let snap = app.sys();
            let (gname, gdetail) = app::gpu_line(&snap);
            let gpu_used = app.settings.engine != EngineChoice::Cpu && matches!(crate::gpu::status(), Some(Ok(_)));
            hw_row(ui, "GPU", &gname, &gdetail, gpu_used);
            let cpu_detail = format!("{} cores · opens {} at once", snap.physical, theme::plural(app.threads, "photo"));
            hw_row(ui, "CPU", "Your computer's processor", &cpu_detail, true);

            let open = ui
                .horizontal(|ui| {
                    ui.label(theme::mono("Photos at once", 11.0, theme::ASH));
                    ui.add_space(4.0);
                    let open = help::toggle(ui, "threads", "Photos at once");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.visuals_mut().override_text_color = Some(theme::TABLE);
                        let max = snap.logical.max(1);
                        if theme::button(ui, "+", None, Btn::MonoSmall, 40.0, Some(40.0), app.threads < max).clicked() {
                            app.threads += 1;
                        }
                        ui.label(theme::xp(format!("{} / {}", app.threads, max), 18.0, theme::TABLE));
                        if theme::button(ui, "-", None, Btn::MonoSmall, 40.0, Some(40.0), app.threads > 1).clicked() {
                            app.threads -= 1;
                        }
                    });
                    open
                })
                .inner;
            // What the selected Speed option allows on this computer.
            let max = snap.logical.max(1);
            let gpu_ready = matches!(crate::gpu::status(), Some(Ok(_)));
            let with_gpu = match app.settings.engine {
                EngineChoice::Cpu => false,
                EngineChoice::Gpu => true,
                EngineChoice::Auto => gpu_ready,
            };
            let gpu_cores = if gpu_ready {
                snap.gpu_cores.or_else(|| match crate::gpu::status() {
                    Some(Ok(i)) => crate::sysmon::known_cores(&i.name),
                    _ => None,
                })
            } else {
                None
            };
            let limits = if with_gpu {
                let cores = gpu_cores.map(|c| format!("its {} cores", theme::thousands(c as usize))).unwrap_or_else(|| "all its cores".into());
                format!(
                    "Up to {max} at once on this computer: the processor opens up to {max} photos together, and the graphics card checks each one with {cores}. Fastest here: {max}."
                )
            } else {
                format!(
                    "Up to {max} at once on this processor ({} cores). {} leaves one free so your computer stays responsive.",
                    snap.physical,
                    (max.saturating_sub(1)).max(1)
                )
            };
            ui.label(theme::body(limits, 12.5, theme::ASH));
            if open {
                help::card(ui, help::THREADS, true);
            }
            if info.as_ref().map(|i| i.failed > 0).unwrap_or(false) {
                ui.add_space(6.0);
                ui.checkbox(&mut app.retry_failed, theme::body("Try again on photos that couldn't be opened (after installing a free add-on)", 13.0, theme::TABLE));
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
            ui.label(theme::mono("BestOfBurst · photo culling, on your own computer", 11.0, p.muted));
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
            None => "or click here to choose one".into(),
        };
        theme::text_at(ui, zone.center() + vec2(0.0, 16.0), Align2::CENTER_CENTER, &sub, theme::f_mono(11.0), p.muted, 0.4);
        if app.checking_folder {
            theme::text_at(ui, zone.center() + vec2(0.0, 40.0), Align2::CENTER_CENTER, "COUNTING PHOTOS...", theme::f_mono(10.5), p.mark, 0.8);
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
