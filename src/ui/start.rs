//! The first page: two tools, one choice. Choose the best shots (the culler, with finding
//! originals and watermark removal inside it), or the watermark remover on its own.

use eframe::egui::{self, pos2, vec2, Align2, Key, Rect, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo};

use super::app::{self, BestOfBurstApp, Screen};
use super::marks;
use super::theme;

/// One tool card: number, title, what it does, and a call to action. Returns true when chosen.
// Each argument is one piece of what the card shows; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
fn card(ui: &mut Ui, rect: Rect, no: &str, title: &str, text: &str, action: &str, key: &str, primary: bool) -> bool {
    let resp = ui.interact(rect, ui.scope_id().with(no), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, title));
    let hover = resp.hovered();
    let (bg, fg, sub) = if hover { (theme::INK, theme::TABLE, theme::ASH) } else { (theme::PAPER_DAY, theme::INK, theme::INK_2) };
    let p = ui.painter();
    p.rect_filled(rect, 0.0, bg);
    p.rect_stroke(rect, 0.0, Stroke::new(2.0, theme::INK), StrokeKind::Inside);
    let x = rect.min.x + 28.0;
    theme::text_at(
        ui,
        pos2(x, rect.min.y + 30.0),
        Align2::LEFT_CENTER,
        no,
        theme::f_mono_semi(11.0),
        if hover { theme::EDGE } else { theme::GRAPHITE },
        0.9,
    );
    let size = theme::fit_size(ui, title, 34.0, rect.width() - 56.0, false);
    theme::text_at(ui, pos2(x, rect.min.y + 74.0), Align2::LEFT_CENTER, title, theme::f_xp(size), fg, 0.4);
    let g = ui.painter().layout(text.to_string(), theme::f_body(15.0), sub, rect.width() - 56.0);
    ui.painter().galley(pos2(x, rect.min.y + 108.0), g, sub);
    // Action bar along the bottom.
    let bar = Rect::from_min_max(pos2(rect.min.x, rect.max.y - 56.0), rect.max);
    let fill = if primary {
        if hover {
            theme::RED_HOVER
        } else {
            theme::RED
        }
    } else if hover {
        theme::TABLE
    } else {
        theme::INK
    };
    let on = if primary {
        theme::PAPER_WHITE
    } else if hover {
        theme::INK
    } else {
        theme::TABLE
    };
    ui.painter().rect_filled(bar, 0.0, fill);
    theme::text_at(ui, pos2(bar.min.x + 28.0, bar.center().y), Align2::LEFT_CENTER, &action.to_uppercase(), theme::f_xp(14.0), on, 0.5);
    theme::keycap(ui, pos2(bar.max.x - 22.0, bar.center().y), key, on);
    theme::focus_ring(ui, &resp);
    resp.clicked()
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    app::chrome_bar(
        ui,
        "start_header",
        |ui| {
            let _ = app::logo_cell(ui);
            app::text_cell(ui, "Start", true);
        },
        |_| {},
    );
    let mut go: Option<Screen> = None;
    if !ctx.egui_wants_keyboard_input() {
        ctx.input(|i| {
            if i.key_pressed(Key::Num1) {
                go = Some(Screen::Home);
            }
            if i.key_pressed(Key::Num2) {
                go = Some(Screen::Remover);
            }
        });
    }
    let main = egui::Frame::new().fill(theme::TABLE).inner_margin(48.0);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            marks::burst(ui, r, false);
            ui.add_space(6.0);
            ui.label(theme::mono("BestOfBurst · on your own computer, nothing uploaded", 11.0, theme::GRAPHITE));
        });
        ui.add_space(14.0);
        let w = ui.available_width();
        let size = theme::fit_size(ui, "WHAT ARE WE DOING TODAY?", 64.0, w, false);
        ui.label(theme::xp("What are we doing today?", size, theme::INK));
        ui.add_space(26.0);
        let gap = 24.0;
        let cw = ((w - gap) / 2.0).floor();
        let (area, _) = ui.allocate_exact_size(vec2(w, 330.0), Sense::hover());
        let a = Rect::from_min_size(area.min, vec2(cw, area.height()));
        let b = Rect::from_min_size(pos2(area.min.x + cw + gap, area.min.y), vec2(cw, area.height()));
        if card(
            ui,
            a,
            "01 · AFTER A SHOOT",
            "CHOOSE THE BEST SHOTS",
            "Drop a shoot of thousands of photos. Bursts are stacked, blinks and out-of-focus frames set aside, and the best photo of each moment circled. When your client replies with the previews they chose, find the originals, remove the watermark and save the list of names, all from the same place.",
            "Open a shoot",
            "1",
            true,
        ) {
            go = Some(Screen::Home);
        }
        if card(
            ui,
            b,
            "02 · ON ITS OWN",
            "WATERMARK REMOVER",
            "Have previews with a watermark on them? Drop them in. The mark is learned from the photos themselves and lifted off each one; add the originals folder and the part under the mark is rebuilt from the real photo instead. No culling needed.",
            "Remove watermarks",
            "2",
            false,
        ) {
            go = Some(Screen::Remover);
        }
        ui.add_space(20.0);
        ui.label(theme::body("Your photos are only ever read. Everything BestOfBurst makes goes into new folders.", 14.0, theme::GRAPHITE));
    });
    match go {
        Some(Screen::Home) => {
            app.screen = Screen::Home;
            if let Some(f) = app.folder.clone() {
                app.choose_folder(f, &ctx);
            }
        }
        Some(s) => app.screen = s,
        None => {}
    }
}
