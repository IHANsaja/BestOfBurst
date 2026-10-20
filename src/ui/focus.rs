//! Focus (screen 4): one same-shot group on the night table. Large preview, a floating loupe at
//! 100 % on the subject, the group's film strip, and the decision panel.

use std::time::Instant;

use eframe::egui::{self, pos2, vec2, Align2, Color32, Key, Rect, Stroke, Ui};

use super::app::{self, BestOfBurstApp, Screen};
use super::glim::{self, Mood};
use super::review::{self, file_name, frame_card, run_view, status_word, CardOpts, Nav, Review, FULL_UV};
use super::textures::Loaded;
use super::theme::{self, Btn};
use crate::rank::Status;

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    run_view(app, ui, view);
}

fn letter(k: usize) -> char {
    (b'A' + (k.min(25)) as u8) as char
}

/// Leave Focus for the next close call, or back to the sheet when there is none.
fn keep_and_next(rv: &mut Review) -> Nav {
    let Some(f) = &rv.focus else { return Nav::To(Screen::Review) };
    let g = f.group;
    rv.settle(g);
    match rv.next_close_call(Some(g)) {
        Some(n) if !rv.settled.contains(&n) => {
            rv.open_focus(n, None);
            Nav::Stay
        }
        _ => {
            rv.focus = None;
            Nav::To(Screen::Review)
        }
    }
}

fn view(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui) -> Nav {
    let ctx = ui.ctx().clone();
    let p = theme::pal(true);
    let Some(focus) = &rv.focus else { return Nav::To(Screen::Review) };
    let group = focus.group;
    let members = rv.members(group);
    if members.is_empty() {
        rv.focus = None;
        return Nav::To(Screen::Review);
    }
    let mut view_idx = focus.view;
    let mut big = focus.big_loupe;
    let keeper = rv.keeper_of(group).unwrap_or(members[0]);
    let pos = members.iter().position(|&m| m == view_idx).unwrap_or(0);
    let mut nav = Nav::Stay;

    // Keys.
    if !ctx.egui_wants_keyboard_input() {
        let (keys, cmd) = review::pressed_keys(&ctx);
        for key in keys {
            if cmd {
                match key {
                    Key::Z => rv.undo(),
                    Key::Y => rv.redo(),
                    _ => {}
                }
                continue;
            }
            match key {
                Key::ArrowLeft | Key::ArrowUp => view_idx = members[pos.saturating_sub(1)],
                Key::ArrowRight | Key::ArrowDown => view_idx = members[(pos + 1).min(members.len() - 1)],
                Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 => {
                    let k = match key { Key::Num1 => 0, Key::Num2 => 1, Key::Num3 => 2, _ => 3 };
                    if let Some(&m) = members.get(k) {
                        view_idx = m;
                    }
                }
                Key::P => {
                    // Move the circle to the frame on screen, or to the runner-up from the keeper.
                    let target = if view_idx == keeper { rv.runner_up(group, keeper) } else { Some(view_idx) };
                    if let Some(t) = target {
                        rv.set_status(t, Some(Status::Pick));
                        rv.settle(group);
                        view_idx = t;
                    }
                }
                Key::U => rv.set_status(view_idx, Some(Status::Maybe)),
                Key::X => rv.set_status(view_idx, Some(Status::Reject)),
                Key::Num0 => rv.set_status(view_idx, None),
                Key::Z => big = !big,
                Key::Enter => nav = keep_and_next(rv),
                Key::N => {
                    if let Some(n) = rv.next_close_call(Some(group)) {
                        rv.open_focus(n, None);
                    }
                }
                Key::C => {
                    if rv.open_compare(group, (view_idx != keeper).then_some(view_idx)) {
                        nav = Nav::To(Screen::Compare);
                    }
                }
                Key::Escape | Key::Space => {
                    rv.selected = Some(view_idx);
                    rv.focus = None;
                    nav = Nav::To(Screen::Review);
                }
                _ => {}
            }
        }
    }
    if !matches!(nav, Nav::Stay) {
        return nav;
    }
    // Keep&next may have moved to another group.
    let Some(focus) = &mut rv.focus else { return Nav::To(Screen::Review) };
    if focus.group == group {
        focus.view = view_idx;
        focus.big_loupe = big;
    }
    let group = focus.group;
    let view_idx = focus.view;
    let big = focus.big_loupe;
    let members = rv.members(group);
    let keeper = rv.keeper_of(group).unwrap_or(members[0]);
    rv.selected = Some(view_idx);

    // Header: group counter.
    let closes = rv.close_call_groups();
    let counter = match closes.iter().position(|g| *g == group) {
        Some(k) => format!("Close call {} of {}", k + 1, closes.len()),
        None => format!("Burst · {} photos", members.len()),
    };
    let mut home = false;
    let mut back = false;
    app::chrome_bar(
        ui,
        "focus_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, &counter, true);
        },
        |ui| {
            ui.add_space(16.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            back = theme::button(ui, "Back", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked();
            ui.add_space(16.0);
            app::cell_rule(ui);
        },
    );
    if home {
        rv.focus = None;
        return Nav::Home;
    }
    if back {
        rv.focus = None;
        return Nav::To(Screen::Review);
    }

    if let Some(min) = app.break_due() {
        let mut action = theme::BannerAction::None;
        egui::Panel::top("focus_break").frame(egui::Frame::new()).show(ui, |ui| {
            action = theme::safelight_banner(ui, min);
        });
        match action {
            theme::BannerAction::Done => app.review_clock = Some(Instant::now()),
            theme::BannerAction::Play => {
                app.review_clock = Some(Instant::now());
                return Nav::Corner;
            }
            theme::BannerAction::None => {}
        }
    }

    // Decision panel.
    let view_letter = letter(members.iter().position(|&m| m == view_idx).unwrap_or(0));
    let keeper_letter = letter(members.iter().position(|&m| m == keeper).unwrap_or(0));
    let other = if view_idx == keeper { rv.runner_up(group, keeper) } else { Some(view_idx) };
    let other_letter = other.and_then(|o| members.iter().position(|&m| m == o)).map(letter).unwrap_or('B');
    let side = egui::Frame::new().fill(p.surface).inner_margin(24.0);
    egui::Panel::right("focus_side").exact_size(400.0).resizable(false).frame(side).show(ui, |ui| {
        theme::dark_scope(ui);
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.min.x - 24.0, r.min.y - 24.0), pos2(r.min.x - 24.0, r.max.y + 24.0)], Stroke::new(1.0, p.rule));
        let msg = if rv.ranking.entries[keeper].close_call {
            format!("It's close. I circled {keeper_letter}. Check the eyes in the loupe, then keep it or move the circle.")
        } else {
            format!("{keeper_letter} is the clear keeper here. Have a look anyway.")
        };
        if glim::card(ui, Mood::Happy, true, &msg, 60.0, app.time(), app.reduced_motion).clicked() {
            nav = Nav::Corner;
        }
        ui.add_space(14.0);
        let e = &rv.ranking.entries[view_idx];
        ui.label(theme::mono(
            format!("Frame {view_letter} · {}{}", status_word(e.effective()), if view_idx == keeper { " · circled" } else { "" }),
            11.0,
            theme::SAFELIGHT,
        ));
        ui.label(theme::xp(file_name(&e.path), 18.0, p.on_surface));
        ui.label(theme::mono(format!("best in burst {:.0}  ·  quality {:.0}", e.group_score, e.score), 10.5, p.muted));
        ui.add_space(10.0);
        let (base, cmp_to, name) = if view_idx == keeper { (keeper, other, other_letter.to_string()) } else { (keeper, Some(view_idx), view_letter.to_string()) };
        ui.label(theme::mono(format!("Keeper {keeper_letter} vs {name}"), 10.5, p.muted));
        for (title, v, o, d) in rv.component_rows(base, cmp_to, &name) {
            theme::ruler(ui, p, title, v, o, &d);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let w = ui.available_width();
            if theme::button(ui, "Keep & next", Some("enter"), Btn::Primary, 64.0, Some(w), true).clicked() {
                nav = keep_and_next(rv);
            }
            ui.add_space(4.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            ui.horizontal(|ui| {
                let hw = ((w - 8.0) / 2.0).floor();
                if theme::button(ui, "Compare", Some("C"), Btn::OnDark, 44.0, Some(hw), members.len() > 1).clicked()
                    && rv.open_compare(group, (view_idx != keeper).then_some(view_idx))
                {
                    nav = Nav::To(Screen::Compare);
                }
                if theme::button(ui, "Set aside", Some("X"), Btn::OnDark, 44.0, Some(hw), true).clicked() {
                    rv.set_status(view_idx, Some(Status::Reject));
                }
            });
            ui.add_space(4.0);
            let target = if view_idx == keeper { other } else { Some(view_idx) };
            let target_letter = if view_idx == keeper { other_letter } else { view_letter };
            let move_label = format!("Move circle to {target_letter}");
            if theme::button(ui, &move_label, Some("P"), Btn::Alert, 52.0, Some(w), target.is_some()).clicked() {
                if let Some(t) = target {
                    rv.set_status(t, Some(Status::Pick));
                    rv.settle(group);
                    if let Some(f) = &mut rv.focus {
                        f.view = t;
                    }
                }
            }
        });
    });
    if !matches!(nav, Nav::Stay) {
        return nav;
    }

    // Film strip of the group.
    let strip = egui::Frame::new().fill(theme::FILM).inner_margin(egui::Margin { left: 20, right: 20, top: 8, bottom: 8 });
    egui::Panel::bottom("focus_strip").exact_size(148.0).frame(strip).show(ui, |ui| {
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (k, &m) in members.iter().enumerate() {
                    let resp = frame_card(ui, rv, app, m, vec2(170.0, 128.0), CardOpts { count: 1, selected: m == view_idx, letter: Some(letter(k)), show_score: false, plain: false });
                    if resp.clicked() {
                        if let Some(f) = &mut rv.focus {
                            f.view = m;
                        }
                    }
                }
            });
        });
    });

    // Large preview with the floating loupe.
    let main = egui::Frame::new().fill(p.surface).inner_margin(20.0);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        let area = ui.max_rect();
        let fmt = rv.formats[view_idx];
        let path = rv.abs_path(view_idx);
        let aside = matches!(rv.ranking.entries[view_idx].effective(), Status::Reject | Status::Failed);
        let tint = if aside { theme::ASIDE_TINT } else { Color32::WHITE };
        let mut img_rect = area;
        match app.textures.preview(&path, fmt) {
            Loaded::Ready(id, size) => {
                img_rect = theme::fit(size, area);
                ui.painter().image(id, img_rect, FULL_UV, tint);
            }
            Loaded::Failed(e) => app::center_text(ui, area, &e, p),
            Loaded::Loading => {
                if let Some(t) = rv.thumb_path(view_idx) {
                    if let Loaded::Ready(id, size) = app.textures.thumb(&t) {
                        img_rect = theme::fit(size, area);
                        ui.painter().image(id, img_rect, FULL_UV, tint);
                    }
                }
            }
        }
        let t = rv.mark_t(view_idx, app.reduced_motion);
        match rv.ranking.entries[view_idx].effective() {
            Status::Pick => super::marks::keeper(ui, img_rect.shrink(img_rect.width() * 0.02), theme::SAFELIGHT, 6.0, t),
            Status::Reject | Status::Failed => super::marks::cross(ui, img_rect.shrink(img_rect.width() * 0.08), theme::TABLE, 5.0, t),
            Status::Maybe => {}
        }
        theme::text_at(ui, area.left_top(), Align2::LEFT_TOP, &format!("{view_letter}"), theme::f_xp_black(40.0), theme::SAFELIGHT, 0.0);

        // Loupe: the subject at 100 %, the only element with a shadow.
        let ls = if big { 460.0 } else { 300.0 };
        let lens = Rect::from_min_size(pos2(area.max.x - ls - 8.0, area.max.y - ls - 8.0), vec2(ls, ls));
        theme::loupe_shadow(ui, lens);
        ui.painter().rect_filled(lens, 0.0, theme::FILM);
        let center = rv.center_of(view_idx);
        match app.textures.crop(&path, fmt, center) {
            Loaded::Ready(id, size) => {
                // Show a square window of the crop at 1 image pixel per point.
                let side = size.x.min(size.y).min(ls);
                let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(side / size.x, side / size.y));
                ui.painter().image(id, lens.shrink(2.0), uv, tint);
            }
            Loaded::Loading => app::center_text(ui, lens, "DEVELOPING LOUPE...", p),
            Loaded::Failed(_) => app::center_text(ui, lens, "NO LOUPE", p),
        }
        ui.painter().rect_stroke(lens, 0.0, Stroke::new(2.0, theme::INK), egui::StrokeKind::Inside);
        let tag = Rect::from_min_size(lens.left_top() + vec2(2.0, 2.0), vec2(112.0, 20.0));
        ui.painter().rect_filled(tag, 0.0, theme::INK);
        theme::text_at(ui, tag.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, "LOUPE · 100%", theme::f_mono_semi(10.0), theme::EDGE, 0.8);
        let tag = Rect::from_min_size(lens.left_bottom() + vec2(2.0, -22.0), vec2(64.0, 20.0));
        ui.painter().rect_filled(tag, 0.0, theme::INK);
        theme::text_at(ui, tag.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, "Z BIGGER", theme::f_mono(10.0), theme::ASH, 0.8);
    });
    nav
}
