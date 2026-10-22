//! Grease Pencil design system (DESIGN.md): tokens, bundled fonts, and the shared components
//! (buttons, key caps, segmented choice, ruler bars, table toggle, safelight banner).

use std::sync::Arc;

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Pos2, Rect, Response, RichText, Role, Sense, Shadow, Stroke, StrokeKind, Ui, WidgetInfo,
};

// 2.1 Core palette
pub const TABLE: Color32 = Color32::from_rgb(0xE6, 0xE4, 0xDE);
pub const INK: Color32 = Color32::from_rgb(0x12, 0x12, 0x11);
pub const FILM: Color32 = Color32::from_rgb(0x1B, 0x1A, 0x18);
pub const GRAPHITE: Color32 = Color32::from_rgb(0x5C, 0x5A, 0x55);
pub const RED: Color32 = Color32::from_rgb(0xD4, 0x2A, 0x1C);
pub const RED_HOVER: Color32 = Color32::from_rgb(0xB8, 0x23, 0x16);
pub const SAFELIGHT: Color32 = Color32::from_rgb(0xFF, 0x5A, 0x40);
pub const EDGE: Color32 = Color32::from_rgb(0xD9, 0xA4, 0x41);
pub const CYAN: Color32 = Color32::from_rgb(0x1F, 0x4F, 0xD8);

// 2.2 Supporting neutrals
pub const INK_2: Color32 = Color32::from_rgb(0x3C, 0x3B, 0x37);
pub const ASH: Color32 = Color32::from_rgb(0xA9, 0xA6, 0x9D);
pub const RULE_DAY: Color32 = Color32::from_rgb(0xC4, 0xC1, 0xB8);
pub const RULE_DARK: Color32 = Color32::from_rgb(0x2E, 0x2D, 0x2A);
pub const RULE_NIGHT: Color32 = Color32::from_rgb(0x33, 0x32, 0x2F);
pub const RULE_STRONG: Color32 = Color32::from_rgb(0x3A, 0x39, 0x36);
pub const NIGHT_TABLE: Color32 = Color32::from_rgb(0x1A, 0x19, 0x18);
pub const NUDGE_BG: Color32 = Color32::from_rgb(0x2A, 0x15, 0x12);
pub const SAFELIGHT_SOFT: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x73);
pub const PAPER_WHITE: Color32 = Color32::WHITE;
pub const PAPER_DAY: Color32 = Color32::from_rgb(0xF3, 0xF2, 0xEE);
pub const PAPER_NIGHT: Color32 = Color32::from_rgb(0x2A, 0x29, 0x26);

/// "Set aside" frames drop to 45 % opacity.
pub const ASIDE_TINT: Color32 = Color32::from_rgba_premultiplied(115, 115, 115, 115);

/// 2.3 Semantic surface tokens for the day or night table.
#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub night: bool,
    pub surface: Color32,
    pub on_surface: Color32,
    pub muted: Color32,
    pub rule: Color32,
    pub bar: Color32,
    pub mark: Color32,
    pub chrome: Color32,
    pub on_chrome: Color32,
}

pub fn pal(night: bool) -> Pal {
    if night {
        Pal {
            night,
            surface: NIGHT_TABLE,
            on_surface: TABLE,
            muted: ASH,
            rule: RULE_NIGHT,
            bar: GRAPHITE,
            mark: SAFELIGHT,
            chrome: INK,
            on_chrome: TABLE,
        }
    } else {
        Pal {
            night,
            surface: TABLE,
            on_surface: INK,
            muted: GRAPHITE,
            rule: RULE_DAY,
            bar: Color32::from_rgb(0x8E, 0x8B, 0x82),
            mark: RED,
            chrome: INK,
            on_chrome: TABLE,
        }
    }
}

/// The dark chrome surface (header bars, Home side panel), independent of the table toggle.
pub fn chrome_pal() -> Pal {
    Pal { surface: INK, rule: RULE_DARK, ..pal(true) }
}

// 3. Typography: static instances of the OFL fonts, bundled for offline use.
pub const XP_BOLD: &str = "xp-700";
pub const XP: &str = "xp-800";
pub const XP_BLACK: &str = "xp-900";
pub const BODY_MEDIUM: &str = "body-500";
pub const MONO_SEMI: &str = "mono-600";

pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let fallbacks: Vec<String> = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut add = |name: &str, bytes: &'static [u8], family: FontFamily| {
        defs.font_data.insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
        let mut chain = vec![name.to_string()];
        chain.extend(fallbacks.iter().cloned());
        defs.families.insert(family, chain);
    };
    add("Archivo-Regular", include_bytes!("../../assets/fonts/Archivo-Regular.ttf"), FontFamily::Proportional);
    add("Archivo-Medium", include_bytes!("../../assets/fonts/Archivo-Medium.ttf"), FontFamily::Name(BODY_MEDIUM.into()));
    add("Archivo-Expanded-Bold", include_bytes!("../../assets/fonts/Archivo-Expanded-Bold.ttf"), FontFamily::Name(XP_BOLD.into()));
    add("Archivo-Expanded-ExtraBold", include_bytes!("../../assets/fonts/Archivo-Expanded-ExtraBold.ttf"), FontFamily::Name(XP.into()));
    add("Archivo-Expanded-Black", include_bytes!("../../assets/fonts/Archivo-Expanded-Black.ttf"), FontFamily::Name(XP_BLACK.into()));
    add("MartianMono-Condensed-Medium", include_bytes!("../../assets/fonts/MartianMono-Condensed-Medium.ttf"), FontFamily::Monospace);
    add("MartianMono-Condensed-SemiBold", include_bytes!("../../assets/fonts/MartianMono-Condensed-SemiBold.ttf"), FontFamily::Name(MONO_SEMI.into()));
    ctx.set_fonts(defs);
}

pub fn f_xp(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(XP.into()))
}
pub fn f_xp_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(XP_BOLD.into()))
}
pub fn f_xp_black(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(XP_BLACK.into()))
}
pub fn f_body(size: f32) -> FontId {
    FontId::proportional(size)
}
pub fn f_body_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BODY_MEDIUM.into()))
}
pub fn f_mono(size: f32) -> FontId {
    FontId::monospace(size)
}
pub fn f_mono_semi(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MONO_SEMI.into()))
}

/// Expanded uppercase display/label text.
pub fn xp(text: impl AsRef<str>, size: f32, color: Color32) -> RichText {
    RichText::new(text.as_ref().to_uppercase())
        .font(f_xp(size))
        .color(color)
        .extra_letter_spacing(if size >= 40.0 { -0.025 * size } else { 0.03 * size })
}

/// Monospace metadata: uppercase, +0.08em.
pub fn mono(text: impl AsRef<str>, size: f32, color: Color32) -> RichText {
    RichText::new(text.as_ref().to_uppercase())
        .font(f_mono(size))
        .color(color)
        .extra_letter_spacing(0.08 * size)
}

pub fn body(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text.into()).font(f_body(size)).color(color)
}

/// Lay out one line of text with letter spacing for painting.
pub fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, spacing: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::TextFormat { font_id: font, color, extra_letter_spacing: spacing, ..Default::default() });
    ui.painter().layout_job(job)
}

/// Largest size <= `max` at which `text` fits `width` in the expanded display face.
pub fn fit_size(ui: &Ui, text: &str, max: f32, width: f32, black: bool) -> f32 {
    let font = if black { f_xp_black(max) } else { f_xp(max) };
    let w = galley(ui, text, font, INK, -0.03 * max).size().x;
    if w <= width { max } else { (max * width / w).floor().max(12.0) }
}

pub fn eyebrow(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(mono(text, 11.0, color));
}

/// Horizontal rule across the available width.
pub fn rule(ui: &mut Ui, color: Color32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), width), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color);
}

pub fn vrule(ui: &Ui, x: f32, top: f32, bottom: f32, color: Color32) {
    ui.painter().line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(1.0, color));
}

/// Map the semantic tokens onto egui's visuals: square corners, no shadows, cyan focus.
pub fn apply_visuals(ctx: &egui::Context, p: Pal) {
    let mut v = if p.night { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.surface;
    v.window_fill = p.surface;
    v.extreme_bg_color = if p.night { FILM } else { PAPER_DAY };
    v.faint_bg_color = p.surface;
    v.override_text_color = Some(p.on_surface);
    v.hyperlink_color = CYAN;
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_stroke = Stroke::new(1.5, p.on_surface);
    v.selection.bg_fill = CYAN.linear_multiply(0.35);
    v.selection.stroke = Stroke::new(2.5, CYAN);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.rule);
    v.widgets.inactive.bg_fill = p.surface;
    v.widgets.inactive.weak_bg_fill = p.surface;
    v.widgets.inactive.bg_stroke = Stroke::new(1.5, p.rule);
    v.widgets.hovered.bg_fill = p.surface;
    v.widgets.hovered.weak_bg_fill = p.surface;
    v.widgets.hovered.bg_stroke = Stroke::new(1.5, p.on_surface);
    v.widgets.active.bg_stroke = Stroke::new(2.0, p.on_surface);
    v.text_cursor.stroke = Stroke::new(2.0, CYAN);
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 32.0;
        s.text_styles.insert(egui::TextStyle::Body, f_body(14.0));
        s.text_styles.insert(egui::TextStyle::Button, f_xp(13.0));
        s.text_styles.insert(egui::TextStyle::Small, f_mono(10.5));
        s.text_styles.insert(egui::TextStyle::Monospace, f_mono(11.0));
        s.text_styles.insert(egui::TextStyle::Heading, f_xp(26.0));
    });
}

/// Visuals for a dark region (chrome bar, Home side panel) inside a lighter screen.
pub fn dark_scope(ui: &mut Ui) {
    let v = ui.visuals_mut();
    v.override_text_color = Some(TABLE);
    v.extreme_bg_color = FILM;
    v.widgets.inactive.bg_fill = INK;
    v.widgets.inactive.weak_bg_fill = INK;
    v.widgets.inactive.bg_stroke = Stroke::new(1.5, RULE_STRONG);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TABLE);
    v.widgets.hovered.bg_fill = INK;
    v.widgets.hovered.weak_bg_fill = INK;
    v.widgets.hovered.bg_stroke = Stroke::new(1.5, TABLE);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TABLE);
    v.widgets.active.fg_stroke = Stroke::new(1.0, TABLE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TABLE);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, RULE_DARK);
}

/// Cyanotype 2.5 px focus ring, offset 2 px.
pub fn focus_ring(ui: &Ui, resp: &Response) {
    if resp.has_focus() {
        ui.painter()
            .rect_stroke(resp.rect.expand(2.0), 0.0, Stroke::new(2.5, CYAN), StrokeKind::Outside);
    }
}

/// 6.2 Key cap. Draws "enter" as a vector arrow, since neither font has U+21B5.
pub fn keycap(ui: &Ui, right_center: Pos2, key: &str, color: Color32) -> Rect {
    let enter = key == "enter";
    let text_w = if enter {
        12.0
    } else {
        galley(ui, key, f_mono_semi(11.0), color, 0.0).size().x
    };
    let w = (text_w + 12.0).max(26.0);
    let rect = Rect::from_min_size(pos2(right_center.x - w, right_center.y - 13.0), vec2(w, 26.0));
    let p = ui.painter();
    p.rect_stroke(rect, 3.0, Stroke::new(1.5, color), StrokeKind::Inside);
    if enter {
        let c = rect.center();
        let s = Stroke::new(1.6, color);
        p.line_segment([pos2(c.x + 5.0, c.y - 5.0), pos2(c.x + 5.0, c.y + 2.0)], s);
        p.line_segment([pos2(c.x + 5.0, c.y + 2.0), pos2(c.x - 5.0, c.y + 2.0)], s);
        p.line_segment([pos2(c.x - 5.0, c.y + 2.0), pos2(c.x - 2.0, c.y - 1.0)], s);
        p.line_segment([pos2(c.x - 5.0, c.y + 2.0), pos2(c.x - 2.0, c.y + 5.0)], s);
    } else {
        let g = galley(ui, key, f_mono_semi(11.0), color, 0.0);
        p.galley(rect.center() - g.size() / 2.0, g, color);
    }
    rect
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    /// Grease red fill, white label.
    Primary,
    /// 2 px table-coloured outline on dark.
    OnDark,
    /// Ink fill on the day table.
    OnDay,
    /// Table fill on dark chrome.
    Inverse,
    /// Safelight outline, soft label.
    Alert,
    /// 40 px, 1.5 px outline, mono label.
    MonoSmall,
}

/// 6.1 Buttons: square, uppercase expanded label, shortcut right-aligned.
pub fn button(ui: &mut Ui, label: &str, key: Option<&str>, kind: Btn, height: f32, width: Option<f32>, enabled: bool) -> Response {
    let font = if kind == Btn::MonoSmall { f_mono_semi(11.0) } else { f_xp(if height >= 52.0 { 15.0 } else { 13.0 }) };
    let spacing = if kind == Btn::MonoSmall { 0.88 } else { 0.45 };
    let text = label.to_uppercase();
    let g = galley(ui, &text, font.clone(), INK, spacing);
    let key_w = key.map(|k| if k == "enter" { 40.0 } else { 40.0 + 7.0 * k.len() as f32 }).unwrap_or(0.0);
    let mut pad: f32 = if kind == Btn::MonoSmall { 14.0 } else { 22.0 };
    if let Some(w) = width {
        pad = pad.min(((w - g.size().x - key_w) / 2.0).floor()).max(8.0);
    }
    let w = width.unwrap_or(g.size().x + key_w + pad * 2.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, height), sense);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let hovered = enabled && resp.hovered();
    let pressed = enabled && resp.is_pointer_button_down_on();
    let r = if pressed { rect.translate(vec2(0.0, 1.0)) } else { rect };
    let (fill, stroke, fg) = match kind {
        Btn::Primary => (if hovered { RED_HOVER } else { RED }, None, PAPER_WHITE),
        Btn::OnDark => {
            if hovered { (TABLE, None, INK) } else { (Color32::TRANSPARENT, Some(Stroke::new(2.0, TABLE)), TABLE) }
        }
        Btn::OnDay => (if hovered { INK_2 } else { INK }, None, TABLE),
        Btn::Inverse => (if hovered { PAPER_DAY } else { TABLE }, None, INK),
        Btn::Alert => {
            if hovered { (SAFELIGHT, None, INK) } else { (Color32::TRANSPARENT, Some(Stroke::new(2.0, SAFELIGHT)), SAFELIGHT_SOFT) }
        }
        Btn::MonoSmall => {
            let c = ui.visuals().override_text_color.unwrap_or(TABLE);
            if hovered { (c, None, invert_of(c)) } else { (Color32::TRANSPARENT, Some(Stroke::new(1.5, c)), c) }
        }
    };
    let alpha = if enabled { 1.0 } else { 0.4 };
    let p = ui.painter();
    p.rect_filled(r, 0.0, fill.gamma_multiply(alpha));
    if let Some(s) = stroke {
        p.rect_stroke(r, 0.0, Stroke::new(s.width, s.color.gamma_multiply(alpha)), StrokeKind::Inside);
    }
    let fg = fg.gamma_multiply(alpha);
    let g = galley(ui, &text, font, fg, spacing);
    let tx = if key.is_some() || width.is_some() { r.min.x + pad } else { r.center().x - g.size().x / 2.0 };
    ui.painter().galley(pos2(tx, r.center().y - g.size().y / 2.0), g, fg);
    if let Some(k) = key {
        keycap(ui, pos2(r.max.x - pad + 4.0, r.center().y), k, fg);
    }
    focus_ring(ui, &resp);
    resp
}

fn invert_of(c: Color32) -> Color32 {
    if c.r() as u32 + c.g() as u32 + c.b() as u32 > 380 { INK } else { TABLE }
}

/// 6.3 Segmented choice: square cells separated by 1 px rules; selected cell is inverted.
pub fn segmented(ui: &mut Ui, options: &[(&str, &str)], selected: &mut usize, cols: usize, dark: bool) -> bool {
    let cols = cols.max(1);
    let rows = options.len().div_ceil(cols);
    let w = ui.available_width();
    let cell = vec2(w / cols as f32, 62.0);
    let (area, _) = ui.allocate_exact_size(vec2(w, cell.y * rows as f32), Sense::hover());
    let (rule_c, sel_bg, sel_fg, fg, sub) = if dark {
        (RULE_STRONG, TABLE, INK, TABLE, ASH)
    } else {
        (RULE_DAY, INK, TABLE, INK, GRAPHITE)
    };
    let mut changed = false;
    for (i, (title, desc)) in options.iter().enumerate() {
        let r = Rect::from_min_size(area.min + vec2((i % cols) as f32 * cell.x, (i / cols) as f32 * cell.y), cell);
        let id = ui.make_persistent_id(("seg", title));
        let resp = ui.interact(r, id, Sense::click());
        let on = *selected == i;
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, on, *title));
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
        let p = ui.painter();
        if on {
            p.rect_filled(r, 0.0, sel_bg);
        } else if resp.hovered() {
            p.rect_filled(r, 0.0, rule_c.gamma_multiply(0.5));
        }
        p.rect_stroke(r, 0.0, Stroke::new(1.0, rule_c), StrokeKind::Inside);
        let tc = if on { sel_fg } else { fg };
        let sc = if on { sel_fg } else { sub };
        // Titles shrink to fit narrow cells.
        let mut size = 15.0;
        let mut g = galley(ui, &title.to_uppercase(), f_xp(size), tc, 0.4);
        while g.size().x > cell.x - 24.0 && size > 10.0 {
            size -= 0.5;
            g = galley(ui, &title.to_uppercase(), f_xp(size), tc, 0.3);
        }
        ui.painter().galley(r.min + vec2(12.0, 12.0 + (15.0 - size) * 0.5), g, tc);
        let g = galley(ui, &desc.to_uppercase(), f_mono(if cols >= 3 { 9.0 } else { 10.0 }), sc, if cols >= 3 { 0.3 } else { 0.8 });
        ui.painter().galley(r.min + vec2(12.0, 36.0), g, sc);
        focus_ring(ui, &resp);
    }
    changed
}

/// 6.5 Ruler bar: 2 px track with 10 % ticks, keeper value as a solid bar, runner-up as a tick.
pub fn ruler(ui: &mut Ui, p: Pal, title: &str, value: f32, runner: Option<f32>, delta: &str) {
    ui.horizontal(|ui| {
        ui.label(body(title, 14.0, p.on_surface));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(mono(delta, 11.0, p.muted));
        });
    });
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    let pt = ui.painter();
    let y = rect.center().y;
    pt.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], Stroke::new(2.0, p.rule));
    for k in 0..=10 {
        let x = rect.min.x + rect.width() * k as f32 / 10.0;
        pt.line_segment([pos2(x, y - 4.0), pos2(x, y)], Stroke::new(1.0, p.rule));
    }
    let v = value.clamp(0.0, 1.0);
    pt.rect_filled(Rect::from_min_max(pos2(rect.min.x, y - 3.0), pos2(rect.min.x + rect.width() * v, y + 3.0)), 0.0, p.mark);
    if let Some(r) = runner {
        let x = rect.min.x + rect.width() * r.clamp(0.0, 1.0);
        pt.line_segment([pos2(x, y - 7.0), pos2(x, y + 7.0)], Stroke::new(3.0, p.on_surface));
    }
}

/// "keeper +4" / "even" / "B +3".
pub fn delta_label(keeper: f32, other: Option<f32>, other_name: &str) -> String {
    match other {
        None => format!("{:.0}", keeper * 100.0),
        Some(o) => {
            let d = (keeper - o) * 100.0;
            if d.abs() < 1.0 {
                "even".into()
            } else if d > 0.0 {
                format!("keeper +{d:.0}")
            } else {
                format!("{other_name} +{:.0}", -d)
            }
        }
    }
}

/// 6.7 Day/night table toggle on the chrome bar.
pub fn table_toggle(ui: &mut Ui, night: &mut bool) -> bool {
    let label = if *night { "NIGHT TABLE" } else { "DAY TABLE" };
    let g = galley(ui, label, f_mono_semi(10.5), TABLE, 0.84);
    let w = g.size().x + 12.0 + 40.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
    let on = *night;
    resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, on, "Night table"));
    let p = ui.painter();
    p.galley(pos2(rect.min.x, rect.center().y - g.size().y / 2.0), g, TABLE);
    let track = Rect::from_min_size(pos2(rect.max.x - 34.0, rect.center().y - 9.0), vec2(34.0, 18.0));
    p.rect_stroke(track, 0.0, Stroke::new(1.5, TABLE), StrokeKind::Inside);
    let knob_x = if on { track.max.x - 16.0 } else { track.min.x + 2.0 };
    p.rect_filled(Rect::from_min_size(pos2(knob_x, track.min.y + 2.0), vec2(14.0, 14.0)), 0.0, if on { SAFELIGHT } else { TABLE });
    focus_ring(ui, &resp);
    if resp.clicked() {
        *night = !*night;
        return true;
    }
    false
}

#[derive(PartialEq, Eq)]
pub enum BannerAction {
    None,
    Play,
    Done,
}

/// 6.8 Safelight break banner (never blocks input).
pub fn safelight_banner(ui: &mut Ui, minutes: u64) -> BannerAction {
    let mut action = BannerAction::None;
    let frame = egui::Frame::new().fill(NUDGE_BG).inner_margin(egui::Margin { left: 18, right: 12, top: 8, bottom: 8 });
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(mono("Safelight break", 11.0, SAFELIGHT_SOFT));
            ui.add_space(10.0);
            ui.label(body(
                format!("{minutes} min in. Look at something far away for 20 seconds."),
                14.0,
                TABLE,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.visuals_mut().override_text_color = Some(SAFELIGHT_SOFT);
                if button(ui, "Done", None, Btn::MonoSmall, 40.0, None, true).clicked() {
                    action = BannerAction::Done;
                }
                if button(ui, "Play with Glim", None, Btn::MonoSmall, 40.0, None, true).clicked() {
                    action = BannerAction::Play;
                }
            });
        });
    });
    let r = inner.response.rect;
    ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())), 0.0, SAFELIGHT);
    inner.response.widget_info(|| WidgetInfo::labeled(Role::Status, true, "Safelight break"));
    action
}

/// 6.10 Checklist row with a square checkbox.
pub fn checklist_row(ui: &mut Ui, checked: &mut bool, locked: bool, title: &str, desc: &str, path: Option<&str>) -> bool {
    rule(ui, RULE_DAY, 1.0);
    ui.add_space(10.0);
    let mut changed = false;
    ui.horizontal_top(|ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), if locked { Sense::hover() } else { Sense::click() });
        let on = *checked;
        resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, !locked, on, title));
        let p = ui.painter();
        if on {
            p.rect_filled(rect, 0.0, if locked { GRAPHITE } else { RED });
            let s = Stroke::new(2.2, PAPER_WHITE);
            p.line_segment([pos2(rect.min.x + 4.5, rect.center().y), pos2(rect.min.x + 8.5, rect.max.y - 5.0)], s);
            p.line_segment([pos2(rect.min.x + 8.5, rect.max.y - 5.0), pos2(rect.max.x - 4.0, rect.min.y + 5.0)], s);
        } else {
            p.rect_stroke(rect, 0.0, Stroke::new(1.5, INK), StrokeKind::Inside);
        }
        focus_ring(ui, &resp);
        if resp.clicked() && !locked {
            *checked = !*checked;
            changed = true;
        }
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.label(xp(title, 15.0, INK));
            ui.label(body(desc, 14.0, INK_2));
            if let Some(path) = path {
                ui.label(RichText::new(path).font(f_mono(10.5)).color(GRAPHITE));
            }
        });
    });
    ui.add_space(10.0);
    changed
}

/// Film well: film-base background with sprocket dashes along the top and bottom.
pub fn film_well(ui: &Ui, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, FILM);
    let mut x = rect.min.x + 6.0;
    let s = Stroke::new(2.0, RULE_STRONG);
    while x + 6.0 < rect.max.x {
        p.line_segment([pos2(x, rect.min.y + 4.0), pos2(x + 5.0, rect.min.y + 4.0)], s);
        p.line_segment([pos2(x, rect.max.y - 4.0), pos2(x + 5.0, rect.max.y - 4.0)], s);
        x += 12.0;
    }
}

/// Edge-print triangle (U+25B8 is not in the bundled fonts).
pub fn edge_arrow(ui: &Ui, left_center: Pos2, color: Color32) {
    let c = left_center;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(c.x, c.y - 3.5), pos2(c.x + 5.0, c.y), pos2(c.x, c.y + 3.5)],
        color,
        Stroke::NONE,
    ));
}

/// The loupe's lens shadow: the only shadow in the system.
pub fn loupe_shadow(ui: &Ui, rect: Rect) {
    let sh = Shadow { offset: [0, 18], blur: 40, spread: 0, color: Color32::from_black_alpha(90) };
    ui.painter().add(sh.as_shape(rect, CornerRadius::ZERO));
}

pub fn fit(size: egui::Vec2, rect: Rect) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let s = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * s)
}

/// On-screen size of `size` after `turns` quarter turns.
pub fn turned(size: egui::Vec2, turns: u8) -> egui::Vec2 {
    if turns % 2 == 1 { vec2(size.y, size.x) } else { size }
}

/// Paint the `uv` window of a texture turned `turns` quarter turns clockwise, filling `target`
/// (already sized with `turned`).
pub fn paint_turned(painter: &egui::Painter, id: egui::TextureId, target: Rect, uv: Rect, turns: u8, tint: Color32) {
    let screen = [target.left_top(), target.right_top(), target.right_bottom(), target.left_bottom()];
    let tex = [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()];
    let mut mesh = egui::Mesh::with_texture(id);
    for (k, pos) in screen.into_iter().enumerate() {
        let uv = tex[(k + 4 - (turns % 4) as usize) % 4];
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Fit a whole texture turned `turns` quarter turns into `area` and paint it; returns where.
pub fn image_turned(ui: &Ui, id: egui::TextureId, size: egui::Vec2, area: Rect, turns: u8, tint: Color32) -> Rect {
    let target = fit(turned(size, turns), area);
    paint_turned(ui.painter(), id, target, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), turns, tint);
    target
}

pub fn text_at(ui: &Ui, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32, spacing: f32) -> Rect {
    let g = galley(ui, text, font, color, spacing);
    let rect = align.anchor_size(pos, g.size());
    ui.painter().galley(rect.min, g, color);
    rect
}

/// "1 treat" / "3 treats".
pub fn plural(n: usize, word: &str) -> String {
    format!("{} {}{}", thousands(n), word, if n == 1 { "" } else { "s" })
}

pub fn thousands(n: usize) -> String {
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

/// Windows "Show animations" off means reduced motion.
pub fn reduced_motion() -> bool {
    use windows::core::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    let mut on = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL into the provided pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut BOOL as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && !on.as_bool()
}
