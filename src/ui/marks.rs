//! 5. The marks: hand-drawn keeper circle, maybe wave, set-aside cross and the close-call "?".
//! Paths come from DESIGN.md; they are stretched to the target rect (preserveAspectRatio="none")
//! and can be drawn partially for the 350-500 ms draw-on at the moment of a decision.

use eframe::egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Shape, Stroke, Ui};

use super::theme;

/// Duration of the draw-on animation.
pub const DRAW_MS: f32 = 450.0;

/// A path: start point, then cubic segments (c1, c2, end), in its own viewBox.
struct Path {
    vb: [f32; 2],
    start: [f32; 2],
    segs: &'static [[f32; 6]],
}

// Keeper circle, thumbnail size (viewBox 0 0 120 100).
const THUMB_CIRCLE: Path = Path {
    vb: [120.0, 100.0],
    start: [22.0, 60.0],
    segs: &[
        [10.0, 18.0, 90.0, 4.0, 100.0, 40.0],
        [108.0, 74.0, 44.0, 98.0, 14.0, 74.0],
        [2.0, 62.0, 12.0, 36.0, 36.0, 26.0],
    ],
};

// Keeper circle, hero size (viewBox 0 0 300 110).
const HERO_CIRCLE: Path = Path {
    vb: [300.0, 110.0],
    start: [40.0, 70.0],
    segs: &[
        [20.0, 20.0, 250.0, 0.0, 285.0, 45.0],
        [310.0, 85.0, 120.0, 112.0, 30.0, 88.0],
        [0.0, 78.0, 18.0, 40.0, 70.0, 30.0],
    ],
};

// Logo circle (24 px): M5 9c-1-6 13-7 14-2s-11 7-13 3, made absolute.
const LOGO_CIRCLE: Path = Path {
    vb: [24.0, 24.0],
    start: [5.0, 9.0],
    segs: &[[4.0, 3.0, 18.0, 2.0, 19.0, 7.0], [20.0, 12.0, 8.0, 14.0, 6.0, 10.0]],
};

// Maybe wave (viewBox 0 0 120 20): one soft wave stroke.
const WAVE: Path = Path {
    vb: [120.0, 20.0],
    start: [4.0, 12.0],
    segs: &[
        [20.0, 2.0, 34.0, 2.0, 46.0, 10.0],
        [58.0, 18.0, 70.0, 20.0, 84.0, 10.0],
        [98.0, 0.0, 106.0, 2.0, 116.0, 8.0],
    ],
};

// Set-aside cross: two slightly bowed diagonals (viewBox 0 0 120 100).
const CROSS_A: Path = Path { vb: [120.0, 100.0], start: [12.0, 10.0], segs: &[[40.0, 22.0, 78.0, 66.0, 108.0, 90.0]] };
const CROSS_B: Path = Path { vb: [120.0, 100.0], start: [106.0, 12.0], segs: &[[76.0, 30.0, 42.0, 72.0, 14.0, 88.0]] };

fn cubic(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    pos2(
        a * p0.x + b * p1.x + c * p2.x + d * p3.x,
        a * p0.y + b * p1.y + c * p2.y + d * p3.y,
    )
}

fn points(path: &Path, rect: Rect) -> Vec<Pos2> {
    let map = |x: f32, y: f32| pos2(rect.min.x + x / path.vb[0] * rect.width(), rect.min.y + y / path.vb[1] * rect.height());
    let mut pts = vec![map(path.start[0], path.start[1])];
    let mut cur = pts[0];
    for s in path.segs {
        let (c1, c2, end) = (map(s[0], s[1]), map(s[2], s[3]), map(s[4], s[5]));
        for k in 1..=28 {
            pts.push(cubic(cur, c1, c2, end, k as f32 / 28.0));
        }
        cur = end;
    }
    pts
}

/// Draw the first `t` (0..1) of a polyline, by length.
fn draw_partial(ui: &Ui, pts: &[Pos2], t: f32, stroke: Stroke) {
    if pts.len() < 2 || t <= 0.0 {
        return;
    }
    let t = t.min(1.0);
    let lens: Vec<f32> = pts.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lens.iter().sum();
    let target = total * t;
    let mut out = vec![pts[0]];
    let mut acc = 0.0;
    for (i, l) in lens.iter().enumerate() {
        if acc + l >= target {
            let f = if *l > 0.0 { (target - acc) / l } else { 0.0 };
            out.push(pts[i] + (pts[i + 1] - pts[i]) * f);
            break;
        }
        acc += l;
        out.push(pts[i + 1]);
    }
    ui.painter().add(Shape::line(out, stroke));
}

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Keeper: an open loop that overshoots its start, hugging `rect`.
pub fn keeper(ui: &Ui, rect: Rect, color: Color32, width: f32, t: f32) {
    draw_partial(ui, &points(&THUMB_CIRCLE, rect), t, Stroke::new(width, color));
}

/// Hero circle around a word or number (once per screen).
pub fn hero(ui: &Ui, rect: Rect, color: Color32, width: f32, t: f32) {
    draw_partial(ui, &points(&HERO_CIRCLE, rect), t, Stroke::new(width, color));
}

/// Maybe: one soft wave under the frame.
pub fn wave(ui: &Ui, rect: Rect, color: Color32, width: f32, t: f32) {
    draw_partial(ui, &points(&WAVE, rect), t, Stroke::new(width, color));
}

/// Set aside: two bowed diagonals (the caller dims the photo to 45 %).
pub fn cross(ui: &Ui, rect: Rect, color: Color32, width: f32, t: f32) {
    let s = Stroke::new(width, color);
    draw_partial(ui, &points(&CROSS_A, rect), (t * 2.0).min(1.0), s);
    draw_partial(ui, &points(&CROSS_B, rect), (t * 2.0 - 1.0).max(0.0), s);
}

/// Close call: "?" in Archivo expanded 900, top-right of the frame.
pub fn question(ui: &Ui, top_right: Pos2, size: f32) {
    theme::text_at(ui, top_right, Align2::RIGHT_TOP, "?", theme::f_xp_black(size), theme::SAFELIGHT, 0.0);
}

/// Logo: the small grease circle.
pub fn logo(ui: &Ui, rect: Rect, color: Color32) {
    draw_partial(ui, &points(&LOGO_CIRCLE, rect), 1.0, Stroke::new(2.4 * rect.width() / 24.0, color));
}

/// Logo mark plus BESTOFBURST wordmark; returns its rect.
pub fn wordmark(ui: &Ui, left_center: Pos2, color: Color32, mark: Color32) -> Rect {
    let logo_rect = Rect::from_center_size(left_center + vec2(14.0, 0.0), vec2(28.0, 28.0));
    let r = theme::text_at(ui, left_center + vec2(34.0, 0.0), Align2::LEFT_CENTER, "BESTOFBURST", theme::f_xp(14.0), color, 0.6);
    logo(ui, logo_rect, mark);
    logo_rect.union(r)
}
