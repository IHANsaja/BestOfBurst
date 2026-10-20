//! 8. Glim: a small lens-hood body with a camera-aperture eye, drawn as ink on paper.
//! Glim reacts and never blocks: no modals, no sound, no focus stealing.

use std::f32::consts::TAU;

use eframe::egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Response, Role, Sense, Shape, Stroke, Ui, WidgetInfo};

use super::{marks, theme};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Idle,
    Happy,
    Sleepy,
    Cheer,
}

struct Ink {
    paper: Color32,
    ink: Color32,
    red: Color32,
}

fn ink(night: bool) -> Ink {
    if night {
        Ink { paper: theme::PAPER_NIGHT, ink: theme::TABLE, red: theme::SAFELIGHT }
    } else {
        Ink { paper: theme::PAPER_DAY, ink: theme::INK, red: theme::RED }
    }
}

fn cubic(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            let u = 1.0 - t;
            pos2(
                u * u * u * p0.x + 3.0 * u * u * t * p1.x + 3.0 * u * t * t * p2.x + t * t * t * p3.x,
                u * u * u * p0.y + 3.0 * u * u * t * p1.y + 3.0 * u * t * t * p2.y + t * t * t * p3.y,
            )
        })
        .collect()
}

/// Paint Glim into a square `rect`. `time` drives the bob, blink and z's; `cheer_t` draws the
/// grease circle around Glim in the Cheer mood.
pub fn paint(ui: &Ui, rect: Rect, mood: Mood, night: bool, time: f64, reduced: bool, cheer_t: f32) {
    let c = ink(night);
    let s = rect.width() / 100.0;
    let bob = if reduced { 0.0 } else { (time as f32 * TAU / 3.0).sin() * 2.0 };
    let o = rect.min + vec2(0.0, bob * s * 0.6);
    let p = |x: f32, y: f32| o + vec2(x * s, y * s);
    let w = (2.2 * s).clamp(1.2, 5.0);
    let line = Stroke::new(w, c.ink);
    let painter = ui.painter();

    // Lens-hood body: petal top edge, flared sides, soft bottom.
    let mut body = cubic(p(18.0, 34.0), p(30.0, 25.0), p(40.0, 25.0), p(50.0, 31.0), 10);
    body.extend(cubic(p(50.0, 31.0), p(60.0, 25.0), p(70.0, 25.0), p(82.0, 34.0), 10));
    body.extend(cubic(p(82.0, 34.0), p(80.0, 52.0), p(77.0, 72.0), p(74.0, 86.0), 10));
    body.extend(cubic(p(74.0, 86.0), p(60.0, 89.0), p(40.0, 89.0), p(26.0, 86.0), 10));
    body.extend(cubic(p(26.0, 86.0), p(22.0, 70.0), p(19.0, 50.0), p(19.5, 36.0), 10));
    painter.add(Shape::convex_polygon(body.clone(), c.paper, Stroke::NONE));
    painter.add(Shape::line(body, line));
    // Front ring of the hood.
    painter.add(Shape::line(cubic(p(23.0, 41.0), p(40.0, 47.0), p(60.0, 47.0), p(78.0, 41.0), 16), Stroke::new(w * 0.7, c.ink)));

    // Arms and feet.
    let (al, ar) = if mood == Mood::Cheer {
        ((p(20.0, 52.0), p(8.0, 36.0)), (p(80.0, 52.0), p(92.0, 36.0)))
    } else {
        ((p(20.0, 58.0), p(10.0, 67.0)), (p(80.0, 58.0), p(90.0, 67.0)))
    };
    painter.line_segment([al.0, al.1], line);
    painter.line_segment([ar.0, ar.1], line);
    painter.line_segment([p(37.0, 88.0), p(35.0, 95.0)], line);
    painter.line_segment([p(63.0, 88.0), p(65.0, 95.0)], line);

    // Aperture eye: wobbly outer ring, six blades, hexagonal pupil with a red dot.
    let eye = p(50.0, 62.0);
    let r = 15.0 * s;
    let blink = !reduced && (time % 4.2) < 0.14;
    if blink || mood == Mood::Sleepy && !reduced && (time % 6.0) < 3.0 {
        painter.add(Shape::line(cubic(eye - vec2(r, 0.0), eye + vec2(-r * 0.4, r * 0.35), eye + vec2(r * 0.4, r * 0.35), eye + vec2(r, 0.0), 12), line));
    } else {
        let ring: Vec<Pos2> = (0..=40)
            .map(|k| {
                let a = k as f32 / 40.0 * TAU;
                let rr = r * (1.0 + 0.035 * (3.0 * a + 0.7).sin());
                eye + vec2(a.cos(), a.sin()) * rr
            })
            .collect();
        painter.add(Shape::line(ring, line));
        let pupil = match mood {
            Mood::Happy | Mood::Cheer => 7.5,
            Mood::Idle => 6.0,
            Mood::Sleepy => 3.0,
        } * s;
        let hex: Vec<Pos2> = (0..6)
            .map(|k| {
                let a = k as f32 / 6.0 * TAU + 0.3;
                eye + vec2(a.cos(), a.sin()) * pupil
            })
            .collect();
        for k in 0..6 {
            let a = k as f32 / 6.0 * TAU + 0.3 + 0.75;
            let outer = eye + vec2(a.cos(), a.sin()) * r * 0.97;
            painter.line_segment([hex[k], outer], Stroke::new(w * 0.55, c.ink));
        }
        painter.add(Shape::closed_line(hex, Stroke::new(w * 0.8, c.ink)));
        painter.circle_filled(eye, (1.8 * s).max(1.5), c.red);
        if mood == Mood::Sleepy {
            // Heavy lid.
            painter.add(Shape::line(cubic(eye + vec2(-r, -2.0 * s), eye + vec2(-r * 0.4, -r * 0.2), eye + vec2(r * 0.4, -r * 0.2), eye + vec2(r, -2.0 * s), 12), line));
        }
    }

    // Mouth.
    match mood {
        Mood::Happy | Mood::Cheer => {
            painter.add(Shape::line(cubic(p(42.0, 80.0), p(46.0, 84.5), p(54.0, 84.5), p(58.0, 80.0), 10), line));
        }
        Mood::Idle => {
            painter.line_segment([p(45.0, 82.0), p(55.0, 82.0)], line);
        }
        Mood::Sleepy => {
            painter.circle_stroke(p(50.0, 82.0), 1.6 * s, Stroke::new(w * 0.7, c.ink));
        }
    }

    if mood == Mood::Sleepy {
        let drift = if reduced { 0.0 } else { ((time as f32) % 3.0) / 3.0 };
        for (k, (x, y)) in [(80.0, 26.0), (88.0, 14.0)].iter().enumerate() {
            let pos = p(*x, *y - drift * 4.0);
            let alpha = if reduced { 1.0 } else { 1.0 - (drift - k as f32 * 0.3).abs().min(1.0) * 0.6 };
            theme::text_at(ui, pos, Align2::CENTER_CENTER, "z", theme::f_mono_semi((8.0 + 3.0 * k as f32) * s.max(0.5)), c.ink.gamma_multiply(alpha), 0.0);
        }
    }
    if mood == Mood::Cheer && cheer_t > 0.0 {
        marks::hero(ui, rect.expand(6.0 * s), c.red, (3.0 * s).clamp(2.0, 6.0), cheer_t);
    }
}

/// Glim on its paper card with a speech line (role=status). Clicking opens Glim's corner.
pub fn card(ui: &mut Ui, mood: Mood, night: bool, message: &str, size: f32, time: f64, reduced: bool) -> Response {
    let c = ink(night);
    let w = ui.available_width();
    let h = size + 16.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Link, true, "Glim: open Glim's corner"));
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, c.paper);
    painter.rect_stroke(rect, 0.0, Stroke::new(1.5, if resp.hovered() { c.ink } else { c.ink.gamma_multiply(0.25) }), eframe::egui::StrokeKind::Inside);
    let glim_rect = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(size, size));
    let inner = if mood == Mood::Cheer { glim_rect.shrink(size * 0.12) } else { glim_rect };
    paint(ui, inner, mood, night, time, reduced, 1.0);
    let text_x = glim_rect.max.x + 12.0;
    theme::text_at(ui, pos2(text_x, rect.min.y + 12.0), Align2::LEFT_TOP, "GLIM", theme::f_mono_semi(10.5), c.red, 0.84);
    let wrap = (rect.max.x - text_x - 10.0).max(60.0);
    let job = eframe::egui::text::LayoutJob::simple(message.to_string(), theme::f_body(14.0), c.ink, wrap);
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(text_x, rect.min.y + 30.0), g, c.ink);
    theme::focus_ring(ui, &resp);
    resp
}
