//! Done (screen 6): the light table. The keeper count circled in red, a stat grid, a cheering
//! Glim and the delivery checklist with DELIVER.

use std::time::Instant;

use eframe::egui::{self, pos2, vec2, Align2, Key, Rect, Sense, Stroke, Ui};

use super::app::{self, BestOfBurstApp, Screen};
use super::glim::{self, Mood};
use super::marks;
use super::review::{self, run_view, Nav, Review};
use super::theme::{self, Btn};

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    run_view(app, ui, view);
}

#[allow(deprecated)]
fn done_id() -> egui::Id {
    egui::Id::new("bob_done_opened")
}

/// When Done opened, for the cheer circle draw-on.
fn opened(ctx: &egui::Context) -> f64 {
    let id = done_id();
    let now = ctx.input(|i| i.time);
    let t = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    now - t
}

fn view(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui) -> Nav {
    let ctx = ui.ctx().clone();
    let p = theme::pal(false);
    let mut nav = Nav::Stay;
    let mut deliver = false;
    if !ctx.egui_wants_keyboard_input() {
        let (keys, cmd) = review::pressed_keys(&ctx);
        for k in keys {
            if cmd && k == Key::E {
                deliver = true;
            }
            if k == Key::Escape {
                nav = Nav::To(Screen::Review);
            }
        }
    }
    if !matches!(nav, Nav::Stay) {
        ctx.data_mut(|d| d.remove::<f64>(done_id()));
        return nav;
    }
    let since = opened(&ctx);

    let shoot = app.shoot_name();
    let mut home = false;
    let mut back = false;
    app::chrome_bar(
        ui,
        "done_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, &shoot, true);
        },
        |ui| {
            ui.add_space(16.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            back = theme::button(ui, "Back to sheet", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked();
            ui.add_space(16.0);
            app::cell_rule(ui);
        },
    );
    if home || back {
        ctx.data_mut(|d| d.remove::<f64>(done_id()));
        return if home { Nav::Home } else { Nav::To(Screen::Review) };
    }

    let s = rv.ranking.summary.clone();
    let total = rv.ranking.entries.len();

    // Delivery checklist.
    let side = egui::Frame::new().fill(theme::PAPER_DAY).inner_margin(28.0);
    egui::Panel::right("done_side").exact_size(460.0).resizable(false).frame(side).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.min.x - 28.0, r.min.y - 28.0), pos2(r.min.x - 28.0, r.max.y + 28.0)], Stroke::new(1.5, theme::INK));
        ui.label(theme::mono("Delivery", 11.0, theme::GRAPHITE));
        ui.label(theme::xp("What you get", 22.0, theme::INK));
        ui.add_space(12.0);
        let picks_dir = rv.out.join("picks").display().to_string();
        let mut always = true;
        theme::checklist_row(ui, &mut always, true, "Picks folder", "Your keepers, numbered best first (0001, 0002...).", Some(&picks_dir));
        let o = &mut rv.write_opts;
        theme::checklist_row(ui, &mut o.maybe, false, "Maybe folder", "Good photos that didn't make the cut, in case you want a second look.", None);
        theme::checklist_row(ui, &mut o.rejects, false, "Set-aside folder", "Out-of-focus, too dark or too bright photos, sorted by reason.", None);
        theme::checklist_row(ui, &mut o.copy, false, "Make full copies", "Normally the folders hold shortcuts that look and open like the real photos but take no extra disk space. Turn this on if you'll edit the photos in those folders, so your originals stay untouched.", None);
        theme::checklist_row(ui, &mut o.keep_names, false, "Keep original file names", "Don't number the picks.", None);
        let mut report = true;
        theme::checklist_row(ui, &mut report, true, "Spreadsheet of results", "Every photo with its quality and decision (report.csv, opens in Excel).", None);
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let w = ui.available_width();
            if let Some(d) = &app.last_delivery {
                ui.horizontal(|ui| {
                    if theme::button(ui, "Show folder", None, Btn::OnDay, 44.0, None, true).clicked() {
                        let _ = std::process::Command::new("explorer").arg(&d.picks_dir).spawn();
                    }
                });
            }
            app::message_line(app, ui, p);
            let label = if rv.writing { "Saving...".to_string() } else { format!("Deliver {}", theme::thousands(s.picks)) };
            if theme::button(ui, &label, Some("Ctrl E"), Btn::Primary, 68.0, Some(w), !rv.writing).clicked() {
                deliver = true;
            }
        });
    });

    let main = egui::Frame::new().fill(theme::TABLE).inner_margin(48.0);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        ui.label(theme::mono("Ready to deliver", 11.0, theme::GRAPHITE));
        let count = theme::thousands(s.picks);
        let w = ui.available_width();
        let size = theme::fit_size(ui, &count, 220.0, w * 0.8, true);
        let g = theme::galley(ui, &count, theme::f_xp_black(size), theme::INK, -0.04 * size);
        let (rect, _) = ui.allocate_exact_size(vec2(w, g.size().y * 0.9 + 20.0), Sense::hover());
        let text_pos = rect.min + vec2(size * 0.18, 10.0);
        let gs = g.size();
        ui.painter().galley(text_pos, g, theme::INK);
        let t = if app.reduced_motion { 1.0 } else { marks::ease_out((since as f32 * 1000.0) / marks::DRAW_MS) };
        marks::hero(ui, Rect::from_min_size(text_pos - vec2(size * 0.2, size * 0.02), vec2(gs.x + size * 0.4, gs.y * 0.95)), theme::RED, (size / 40.0).clamp(3.0, 6.0), t);
        ui.label(theme::xp(format!("keepers, circled from {} photos", theme::thousands(total)), 26.0, theme::INK));
        ui.add_space(28.0);

        // 3-column stat grid.
        let reasons = {
            let mut v: Vec<(String, usize)> = s.reject_reasons.iter().map(|(k, n)| (super::review::reason_words(k).to_lowercase(), *n)).collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            v.iter().take(3).map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(" · ")
        };
        let closes = rv.close_call_groups().len();
        let stats = [
            ("Moments", theme::thousands(s.groups), format!("{} photos checked", theme::thousands(s.analyzed))),
            ("Close calls settled", format!("{} / {}", rv.settled.len(), closes), format!("{} for Glim", theme::plural(app.settings.treats as usize, "treat"))),
            ("Set aside", theme::thousands(s.rejects + s.failed), if reasons.is_empty() { "nothing".into() } else { reasons }),
        ];
        let cw = (w - 32.0) / 3.0;
        ui.horizontal_top(|ui| {
            for (title, big, sub) in stats {
                ui.allocate_ui_with_layout(vec2(cw, 150.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(cw);
                    theme::rule(ui, theme::INK, 2.0);
                    ui.add_space(8.0);
                    ui.label(theme::mono(title, 11.0, theme::GRAPHITE));
                    ui.label(theme::xp(big, 44.0, theme::INK));
                    ui.label(theme::body(sub, 14.0, theme::INK_2));
                });
                ui.add_space(16.0);
            }
        });
        ui.add_space(24.0);
        let msg = format!("Circled and ready! You've got {} to spend in my corner.", theme::plural(app.settings.treats as usize, "treat"));
        ui.allocate_ui(vec2(w.min(520.0), 120.0), |ui| {
            if glim::card(ui, Mood::Cheer, false, &msg, 84.0, app.time(), app.reduced_motion).clicked() {
                nav = Nav::Corner;
            }
        });
        let _ = Align2::LEFT_TOP;
    });

    if deliver {
        rv.deliver(app.bg_tx.clone(), &ctx);
        app.last_delivery = None;
        app.review_clock = Some(Instant::now());
    }
    if !matches!(nav, Nav::Stay) {
        ctx.data_mut(|d| d.remove::<f64>(done_id()));
    }
    nav
}
