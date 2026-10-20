//! Plain-language help: short explanations behind each "?" button, and the Help screen (F1).

use eframe::egui::{self, Key, Ui};

use super::app::{self, BestOfBurstApp};
use super::theme::{self, Btn};

pub const KEEP: &str = "How many photos you want at the end. Type a number (like 300) or a percentage of the shoot (like 10%). BestOfBurst circles that many of the best ones and copies them into a \"picks\" folder. You can change it later without starting over.";

pub const SHOT_TYPE: &str = "What kind of shoot this is, so the scoring knows what matters most. General is a good default. Portrait cares more about good light and clean skin tones. Sports cares most about sharpness. Landscape cares more about contrast and colour.";

pub const ENGINE: &str = "Which part of your computer does the heavy work.\n\n• Auto (recommended): uses your graphics card if it works, otherwise the processor.\n• GPU: the graphics card shrinks and measures every photo. It is built for this kind of work and frees the processor to open the next photos.\n• CPU: the processor does everything. Slower, but works on every computer.\n\nOpening (decoding) each photo always happens on the processor, so a faster graphics card helps, but not endlessly.";

pub const THREADS: &str = "How many photos the processor opens at the same time. Cores are the processor's independent workers; this computer has the number shown. More threads finish sooner, but leave less room for other programs while BestOfBurst works. The default leaves one core free so your computer stays responsive. Using all cores together with the GPU is usually the fastest.";

pub const LIVE: &str = "What your computer is doing right now.\n\n• Processor: how busy the whole processor is, and how much of that is BestOfBurst.\n• Workers: photos being opened at this moment, and how many are waiting for or using the graphics card.\n• Graphics card: how busy it is overall and from BestOfBurst. Cores are its small parallel workers; \"about N busy\" is an estimate from how busy it is.\n• Video memory: memory on the graphics card in use.\n• Per photo: average time to open a photo, and to measure it on the GPU or CPU.";

/// Help screen sections: title and text.
pub const SECTIONS: &[(&str, &str)] = &[
    ("What BestOfBurst does", "You give it a folder of photos. It looks at every frame, stacks the ones taken of the same moment, and circles the best frame of each moment. The best ones go into a \"picks\" folder next to your photos. Your original files are never changed, moved or deleted."),
    ("The marks", "A red circle means keeper. A wave underneath means maybe. A cross means set aside (too blurry, too dark, and so on). A \"?\" marks a close call: two frames of the same moment that are nearly equal, so your eyes should decide."),
    ("Keep", KEEP),
    ("Shot type", SHOT_TYPE),
    ("Engine: GPU and CPU", ENGINE),
    ("Threads and cores", THREADS),
    ("The live readings", LIVE),
    ("The loupe and compare", "The loupe shows one moment at a time with the subject at 100 % so you can check eyes and focus. Press Enter to keep the circled frame and go to the next close call, or P to move the circle. Compare shows two frames on top of each other: wipe with the mouse or blink between them."),
    ("Delivering", "Deliver writes the picks folder. By default the files there are hard links: they look like normal files but take no extra space. Deleting one never deletes your original, but an editor that saves over a file in picks changes the original too. Choose \"Copy instead of hard links\" if you will edit the pixels."),
    ("Glim", "Glim is the little lens helper. Every close call you settle earns a treat; spend them in Glim's corner. After 20 minutes of reviewing, Glim suggests a short break for your eyes."),
    ("Keyboard", "F1 help. Arrows move. P keeper, U maybe, X set aside, 0 clear. Space opens the loupe, N the next close call, C compare. Ctrl+Z undo, Ctrl+E deliver. Esc goes back."),
];

/// A small "?" key-cap button that toggles an inline explanation; returns whether it is open.
pub fn toggle(ui: &mut Ui, id: &str, label: &str) -> bool {
    #[allow(deprecated)]
    let key = egui::Id::new(("bob_help", id));
    let mut open = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(key, || false));
    let color = ui.visuals().override_text_color.unwrap_or(theme::INK);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, open, format!("Help: {label}")));
    let fill = if open || resp.hovered() { theme::CYAN } else { egui::Color32::TRANSPARENT };
    let fg = if open || resp.hovered() { theme::PAPER_WHITE } else { color };
    ui.painter().rect_filled(rect, 3.0, fill);
    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.5, if open || resp.hovered() { theme::CYAN } else { color }), egui::StrokeKind::Inside);
    theme::text_at(ui, rect.center(), egui::Align2::CENTER_CENTER, "?", theme::f_mono_semi(12.0), fg, 0.0);
    theme::focus_ring(ui, &resp);
    if resp.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(key, open));
    }
    resp.on_hover_text("What does this mean?");
    open
}

/// The explanation card under a toggled "?".
pub fn card(ui: &mut Ui, text: &str, dark: bool) {
    let (bg, fg) = if dark { (theme::PAPER_NIGHT, theme::TABLE) } else { (theme::PAPER_DAY, theme::INK_2) };
    let r = egui::Frame::new()
        .fill(bg)
        .inner_margin(egui::Margin { left: 14, right: 10, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::body(text, 13.5, fg));
        })
        .response
        .rect;
    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(3.0, r.height())), 0.0, theme::CYAN);
}

/// Label with a "?" toggle; shows the card when open.
pub fn labelled(ui: &mut Ui, label: &str, id: &str, text: &str, dark: bool, color: egui::Color32) {
    let open = ui
        .horizontal(|ui| {
            ui.label(theme::mono(label, 11.0, color));
            ui.add_space(4.0);
            toggle(ui, id, label)
        })
        .inner;
    if open {
        card(ui, text, dark);
    }
}

/// Help screen (F1).
pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let back_to = app.help_return;
    let mut back = false;
    let mut home = false;
    app::chrome_bar(
        ui,
        "help_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, "Help", true);
        },
        |ui| {
            ui.add_space(16.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            back = theme::button(ui, "Back", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked();
            ui.add_space(16.0);
            app::cell_rule(ui);
        },
    );
    if !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(Key::Escape)) {
        back = true;
    }
    if home {
        app.go_home(&ctx);
        return;
    }
    if back {
        app.screen = back_to;
        return;
    }
    let main = egui::Frame::new().fill(theme::TABLE).inner_margin(48.0);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(820.0);
            ui.label(theme::mono("Help · plain words", 11.0, theme::GRAPHITE));
            ui.label(theme::xp("How BestOfBurst works", 34.0, theme::INK));
            ui.add_space(18.0);
            for (title, text) in SECTIONS {
                theme::rule(ui, theme::INK, 1.5);
                ui.add_space(10.0);
                ui.label(theme::xp(*title, 18.0, theme::INK));
                ui.add_space(4.0);
                ui.label(theme::body(*text, 15.0, theme::INK_2));
                ui.add_space(16.0);
            }
        });
    });
}
