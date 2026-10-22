//! Full-screen viewer: one photo filling the monitor at full size, upright, with zoom, pan and
//! quarter turns. Arrows step through the photos it was opened on; P / U / X still decide.

use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, Key, Pos2, Rect, Sense, Ui, Vec2};

use super::app::{self, BestOfBurstApp, Screen};
use super::review::{self, file_name, run_view, status_word, Nav, Review, ViewerState};
use super::textures::Loaded;
use super::theme::{self, Btn};
use crate::rank::Status;

const ZOOM_MAX: f32 = 16.0;
const ZOOM_STEP: f32 = 1.25;
const BAR_H: f32 = 60.0;

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    run_view(app, ui, view);
}

enum Act {
    Step(isize),
    ZoomBy(f32),
    Fit,
    /// One image pixel per screen pixel, or back to fit when already there.
    Actual,
    Turn(i8),
    Decide(Option<Status>),
    Leave,
}

fn leave(rv: &mut Review) -> Nav {
    Nav::To(rv.viewer.take().map(|v| v.back_to).unwrap_or(Screen::Review))
}

/// Zoom to `zoom`, keeping the image point under `at` in place.
fn zoom_about(v: &mut ViewerState, zoom: f32, at: Pos2, center: Pos2) {
    let zoom = zoom.clamp(1.0, ZOOM_MAX);
    let k = zoom / v.zoom;
    v.pan = (at - center) - (at - center - v.pan) * k;
    v.zoom = zoom;
}

fn view(app: &mut BestOfBurstApp, rv: &mut Review, ui: &mut Ui) -> Nav {
    let ctx = ui.ctx().clone();
    let p = theme::pal(true);
    let Some(v) = &rv.viewer else { return Nav::To(Screen::Review) };
    let idx = v.idx;
    let pos = v.list.iter().position(|&i| i == idx).unwrap_or(0);
    let count = v.list.len();
    let zoom = v.zoom;
    let mut acts: Vec<Act> = Vec::new();

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
            acts.push(match key {
                Key::Escape | Key::F => Act::Leave,
                Key::ArrowLeft | Key::ArrowUp => Act::Step(-1),
                Key::ArrowRight | Key::ArrowDown => Act::Step(1),
                Key::Plus | Key::Equals => Act::ZoomBy(ZOOM_STEP),
                Key::Minus => Act::ZoomBy(1.0 / ZOOM_STEP),
                Key::Z => Act::Actual,
                Key::R => Act::Turn(1),
                Key::L => Act::Turn(-1),
                Key::P => Act::Decide(Some(Status::Pick)),
                Key::U => Act::Decide(Some(Status::Maybe)),
                Key::X => Act::Decide(Some(Status::Reject)),
                Key::Num0 => Act::Decide(None),
                _ => continue,
            });
        }
    }

    let e = &rv.ranking.entries[idx];
    let name = file_name(&e.path).to_string();
    let mut info = format!("{}  ·  {}", name, status_word(e.effective()));
    if count > 1 {
        info.push_str(&format!("  ·  {} of {}", pos + 1, count));
    }

    // The image: full size when ready, else the preview or the thumbnail meanwhile.
    let path = rv.abs_path(idx);
    let fmt = rv.formats[idx];
    let turns = rv.rotation(idx);
    let mut error = None;
    let full = match app.textures.full(&path, fmt) {
        Loaded::Ready(id, size) => Some((id, size)),
        Loaded::Failed(err) => {
            error = Some(err);
            None
        }
        Loaded::Loading => None,
    };
    let shown = match full {
        Some(t) => Some(t),
        None => app.textures.preview_if_ready(&path).or_else(|| match rv.thumb(&mut app.textures, idx) {
            Some(Loaded::Ready(id, size)) => Some((id, size)),
            _ => None,
        }),
    };

    // Toolbar.
    #[allow(deprecated)]
    let zoom_slot = egui::Id::new("bob_viewer_zoom_slot");
    let bar = egui::Frame::new().fill(theme::INK).inner_margin(egui::Margin { left: 20, right: 16, top: 10, bottom: 10 });
    egui::Panel::bottom("viewer_bar").exact_size(BAR_H).frame(bar).show(ui, |ui| {
        theme::dark_scope(ui);
        ui.horizontal_centered(|ui| {
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            ui.label(theme::mono(&info, 11.0, theme::EDGE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button(ui, "Exit", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked() {
                    acts.push(Act::Leave);
                }
                ui.add_space(12.0);
                if theme::button(ui, "Turn right", Some("R"), Btn::MonoSmall, 40.0, None, true).clicked() {
                    acts.push(Act::Turn(1));
                }
                if theme::button(ui, "Turn left", Some("L"), Btn::MonoSmall, 40.0, None, true).clicked() {
                    acts.push(Act::Turn(-1));
                }
                ui.add_space(12.0);
                if theme::button(ui, "Zoom in", Some("+"), Btn::MonoSmall, 40.0, None, zoom < ZOOM_MAX).clicked() {
                    acts.push(Act::ZoomBy(ZOOM_STEP));
                }
                if theme::button(ui, "100%", Some("Z"), Btn::MonoSmall, 40.0, None, full.is_some()).clicked() {
                    acts.push(Act::Actual);
                }
                if theme::button(ui, "Fit", None, Btn::MonoSmall, 40.0, None, zoom > 1.0).clicked() {
                    acts.push(Act::Fit);
                }
                if theme::button(ui, "Zoom out", Some("-"), Btn::MonoSmall, 40.0, None, zoom > 1.0).clicked() {
                    acts.push(Act::ZoomBy(1.0 / ZOOM_STEP));
                }
                // Filled in below, once the image's on-screen scale is known.
                ui.add_space(8.0);
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 40.0), Sense::hover());
                ui.ctx().data_mut(|d| d.insert_temp(zoom_slot, r));
                ui.add_space(12.0);
                if theme::button(ui, "Next", None, Btn::MonoSmall, 40.0, None, pos + 1 < count).clicked() {
                    acts.push(Act::Step(1));
                }
                if theme::button(ui, "Previous", None, Btn::MonoSmall, 40.0, None, pos > 0).clicked() {
                    acts.push(Act::Step(-1));
                }
            });
        });
    });

    let mut area = Rect::NOTHING;
    let mut fit_size = Vec2::ZERO;
    let main = egui::Frame::new().fill(theme::FILM);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        area = ui.max_rect();
        let resp = ui.allocate_rect(area, Sense::click_and_drag());
        let Some(v) = &mut rv.viewer else { return };
        match shown {
            Some((id, size)) => {
                fit_size = theme::fit(theme::turned(size, turns), area).size();
                // Scroll zooms about the pointer, drag moves, double-click toggles fit / 100 %.
                if let Some(at) = resp.hover_pos() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        zoom_about(v, v.zoom * (1.0 + scroll * 0.002), at, area.center());
                    }
                }
                if resp.dragged() {
                    v.pan += resp.drag_delta();
                }
                if resp.double_clicked() {
                    acts.push(Act::Actual);
                }
                if v.zoom > 1.0 {
                    ui.ctx().set_cursor_icon(if resp.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
                }
                let shown_size = fit_size * v.zoom;
                let max = ((shown_size - area.size()) / 2.0).max(Vec2::ZERO);
                v.pan = v.pan.clamp(-max, max);
                let target = Rect::from_center_size(area.center() + v.pan, shown_size);
                theme::paint_turned(ui.painter(), id, target, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), turns, Color32::WHITE);
                if full.is_none() && error.is_none() {
                    theme::text_at(ui, area.left_top() + vec2(16.0, 14.0), Align2::LEFT_TOP, "LOADING FULL SIZE...", theme::f_mono_semi(10.5), theme::EDGE, 0.6);
                }
            }
            None => app::center_text(ui, area, error.as_deref().unwrap_or("DEVELOPING..."), p),
        }
        if let Some(err) = &error {
            theme::text_at(ui, area.left_top() + vec2(16.0, 14.0), Align2::LEFT_TOP, &format!("FULL SIZE FAILED: {err}").to_uppercase(), theme::f_mono_semi(10.5), theme::SAFELIGHT, 0.6);
        }
        let hint = "SCROLL OR + / - ZOOM · DRAG TO MOVE · DOUBLE-CLICK 100% · ARROWS NEXT · R / L TURN · P KEEP · ESC EXIT";
        theme::text_at(ui, area.left_bottom() + vec2(16.0, -12.0), Align2::LEFT_BOTTOM, hint, theme::f_mono(10.0), theme::ASH, 0.6);
    });

    // Zoom as image pixels per screen pixel, once the full-size image is in.
    let ppp = ctx.pixels_per_point();
    let actual = full.map(|(_, size)| {
        let texels = theme::turned(size, turns);
        if fit_size.x > 0.0 { texels.x / (fit_size.x * ppp) } else { 1.0 }
    });
    let zoom_now = rv.viewer.as_ref().map(|v| v.zoom).unwrap_or(1.0);
    let zoom_label = match actual {
        Some(a) => format!("{:.0}%", zoom_now / a * 100.0),
        None => format!("FIT ×{zoom_now:.1}"),
    };
    if let Some(r) = ctx.data(|d| d.get_temp::<Rect>(zoom_slot)) {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, zoom_slot));
        let g = painter.layout_no_wrap(zoom_label, theme::f_mono_semi(11.0), theme::EDGE);
        painter.galley(Align2::CENTER_CENTER.anchor_size(r.center(), g.size()).min, g, theme::EDGE);
    }

    for act in acts {
        let Some(v) = &mut rv.viewer else { break };
        match act {
            Act::Leave => return leave(rv),
            Act::Step(d) => {
                let next = (pos as isize + d).clamp(0, count as isize - 1) as usize;
                if next != pos {
                    v.idx = v.list[next];
                    v.zoom = 1.0;
                    v.pan = Vec2::ZERO;
                }
                // Only the first step counts; the next frame recomputes the position.
                break;
            }
            Act::ZoomBy(k) => {
                let z = v.zoom * k;
                zoom_about(v, z, area.center(), area.center());
            }
            Act::Fit => {
                v.zoom = 1.0;
                v.pan = Vec2::ZERO;
            }
            Act::Actual => match actual {
                Some(a) if (v.zoom - a).abs() > 0.01 && a > 1.0 => {
                    let at = ctx.pointer_hover_pos().filter(|p| area.contains(*p)).unwrap_or(area.center());
                    zoom_about(v, a, at, area.center());
                }
                _ => {
                    v.zoom = 1.0;
                    v.pan = Vec2::ZERO;
                }
            },
            Act::Turn(q) => {
                v.zoom = 1.0;
                v.pan = Vec2::ZERO;
                rv.rotate(idx, q);
            }
            Act::Decide(s) => rv.set_status(idx, s),
        }
    }
    Nav::Stay
}
