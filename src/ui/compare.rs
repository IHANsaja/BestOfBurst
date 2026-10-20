//! Compare (screen 5): wipe or blink between the keeper (A) and the runner-up (B).
//! 1 / 2 choose; W wipe, B blink, Space flips manually; arrows pick another B; Z 100 % crop.

use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Align2, Color32, Key, Rect, Sense, Stroke, Ui};

use super::app::{self, BestOfBurstApp, Screen};
use super::review::{self, file_name, run_view, CmpMode, Nav, Review};
use super::textures::Loaded;
use super::theme::{self, Btn};
use crate::rank::Status;

const BLINK: Duration = Duration::from_millis(450);

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    run_view(app, ui, view);
}

fn back(rv: &mut Review) -> Nav {
    rv.cmp = None;
    if rv.focus.is_some() { Nav::To(Screen::Focus) } else { Nav::To(Screen::Review) }
}

fn view(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui) -> Nav {
    let ctx = ui.ctx().clone();
    let p = theme::pal(true);
    let Some(c) = &rv.cmp else { return back(rv) };
    let (group, a, mut b) = (c.group, c.a, c.b);
    let members = rv.members(group);
    let mut choose: Option<usize> = None;
    let mut nav = Nav::Stay;

    if !ctx.egui_wants_keyboard_input() {
        let (keys, _) = review::pressed_keys(&ctx);
        for key in keys {
            let Some(c) = &mut rv.cmp else { break };
            match key {
                Key::Num1 => choose = Some(a),
                Key::Num2 => choose = Some(b),
                Key::W => c.mode = CmpMode::Wipe,
                Key::B => {
                    c.mode = CmpMode::Blink;
                    c.auto_blink = !app.reduced_motion;
                }
                Key::Space => {
                    c.mode = CmpMode::Blink;
                    c.auto_blink = false;
                    c.show_b = !c.show_b;
                }
                Key::Z => c.crop = !c.crop,
                Key::ArrowRight | Key::ArrowLeft => {
                    let others: Vec<usize> = members.iter().copied().filter(|&m| m != a).collect();
                    if let Some(k) = others.iter().position(|&m| m == b) {
                        let n = others.len();
                        b = others[if key == Key::ArrowRight { (k + 1) % n } else { (k + n - 1) % n }];
                        c.b = b;
                    }
                }
                Key::Escape => nav = back(rv),
                _ => {}
            }
        }
    }
    if !matches!(nav, Nav::Stay) {
        return nav;
    }

    let mut home = false;
    let mut esc = false;
    app::chrome_bar(
        ui,
        "compare_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, "Compare · A (circled) and B", true);
        },
        |ui| {
            ui.add_space(16.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            esc = theme::button(ui, "Back", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked();
            ui.add_space(16.0);
            app::cell_rule(ui);
        },
    );
    if home {
        rv.cmp = None;
        return Nav::Home;
    }
    if esc {
        return back(rv);
    }

    let frame = egui::Frame::new().fill(p.surface).inner_margin(20.0);
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        theme::dark_scope(ui);
        let Some(c) = &mut rv.cmp else { return };
        ui.horizontal(|ui| {
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            let wipe = c.mode == CmpMode::Wipe;
            if theme::button(ui, if wipe { "Slider ·" } else { "Slider" }, Some("W"), if wipe { Btn::Inverse } else { Btn::OnDark }, 44.0, None, true).clicked() {
                c.mode = CmpMode::Wipe;
            }
            if theme::button(ui, if !wipe { "Flip ·" } else { "Flip" }, Some("B"), if !wipe { Btn::Inverse } else { Btn::OnDark }, 44.0, None, true).clicked() {
                c.mode = CmpMode::Blink;
                c.auto_blink = !app.reduced_motion;
            }
            ui.add_space(12.0);
            if theme::button(ui, if c.crop { "Zoom to 100% ·" } else { "Zoom to 100%" }, Some("Z"), Btn::OnDark, 44.0, None, true).clicked() {
                c.crop = !c.crop;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button(ui, "Keep B", Some("2"), Btn::Alert, 44.0, None, true).clicked() {
                    choose = Some(c.b);
                }
                if theme::button(ui, "Keep A", Some("1"), Btn::Primary, 44.0, None, true).clicked() {
                    choose = Some(c.a);
                }
            });
        });
        ui.add_space(10.0);
        let area = Rect::from_min_max(ui.cursor().min, ui.max_rect().max - vec2(0.0, 22.0));
        let resp = ui.allocate_rect(area, Sense::click_and_drag());
        if resp.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                c.zoom = (c.zoom * (1.0 + scroll * 0.002)).clamp(1.0, 12.0);
            }
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            c.center -= vec2(d.x / area.width(), d.y / area.height()) / c.zoom;
        }
        if resp.double_clicked() {
            c.zoom = 1.0;
            c.center = vec2(0.5, 0.5);
        }
        let half = 0.5 / c.zoom;
        c.center.x = c.center.x.clamp(half, 1.0 - half);
        c.center.y = c.center.y.clamp(half, 1.0 - half);
        let uv = Rect::from_center_size(pos2(c.center.x, c.center.y), vec2(1.0 / c.zoom, 1.0 / c.zoom));

        if c.mode == CmpMode::Blink && c.auto_blink && c.last_flip.elapsed() >= BLINK {
            c.show_b = !c.show_b;
            c.last_flip = Instant::now();
        }
        if c.mode == CmpMode::Blink && c.auto_blink {
            ctx.request_repaint_after(BLINK);
        }
        let (ia, ib, mode, show_b, crop) = (c.a, c.b, c.mode, c.show_b, c.crop);
        let tex = |app: &mut BestOfBurstApp, i: usize| {
            let path = rv.abs_path(i);
            let fmt = rv.formats[i];
            if crop { app.textures.crop(&path, fmt, rv.center_of(i)) } else { app.textures.preview(&path, fmt) }
        };
        let la = tex(app, ia);
        let lb = tex(app, ib);
        let paint = |ui: &Ui, l: &Loaded, clip: Rect| {
            if let Loaded::Ready(id, size) = l {
                let visible = vec2(size.x * uv.width(), size.y * uv.height());
                let target = theme::fit(visible, area);
                ui.painter().with_clip_rect(clip).image(*id, target, uv, Color32::WHITE);
            }
        };
        ui.painter().rect_filled(area, 0.0, theme::FILM);
        let label = |ui: &Ui, pos: egui::Pos2, align: Align2, i: usize, letter: &str| {
            let e = &rv.ranking.entries[i];
            let circ = if e.effective() == Status::Pick { " · circled" } else { "" };
            theme::text_at(
                ui,
                pos,
                align,
                &format!("{letter} · {} · best in burst {:.0}{circ}", file_name(&e.path), e.group_score).to_uppercase(),
                theme::f_mono_semi(10.5),
                theme::EDGE,
                0.6,
            );
        };
        match mode {
            CmpMode::Wipe => {
                let split = resp.hover_pos().map(|p| p.x).unwrap_or(area.center().x).clamp(area.min.x, area.max.x);
                paint(ui, &la, Rect::from_min_max(area.min, pos2(split, area.max.y)));
                paint(ui, &lb, Rect::from_min_max(pos2(split, area.min.y), area.max));
                ui.painter().line_segment([pos2(split, area.min.y), pos2(split, area.max.y)], Stroke::new(2.0, theme::SAFELIGHT));
                label(ui, area.left_top() + vec2(10.0, 10.0), Align2::LEFT_TOP, ia, "A");
                label(ui, area.right_top() + vec2(-10.0, 10.0), Align2::RIGHT_TOP, ib, "B");
            }
            CmpMode::Blink => {
                let (l, i, name) = if show_b { (&lb, ib, "B") } else { (&la, ia, "A") };
                paint(ui, l, area);
                label(ui, area.left_top() + vec2(10.0, 10.0), Align2::LEFT_TOP, i, name);
                theme::text_at(ui, area.right_top() + vec2(-12.0, 8.0), Align2::RIGHT_TOP, name, theme::f_xp_black(40.0), theme::SAFELIGHT, 0.0);
            }
        }
        if matches!(la, Loaded::Loading) || matches!(lb, Loaded::Loading) {
            app::center_text(ui, area, "LOADING...", p);
        }
        ui.allocate_space(vec2(0.0, 4.0));
        theme::text_at(
            ui,
            pos2(area.min.x, area.max.y + 6.0),
            Align2::LEFT_TOP,
            "1 KEEP A · 2 KEEP B · W SLIDER · B FLIP · SPACE SWAP · ARROWS ANOTHER PHOTO · SCROLL TO ZOOM, DRAG TO MOVE · ESC BACK",
            theme::f_mono(10.0),
            theme::ASH,
            0.6,
        );
    });

    if let Some(i) = choose {
        rv.set_status(i, Some(Status::Pick));
        rv.settle(group);
        return back(rv);
    }
    nav
}
