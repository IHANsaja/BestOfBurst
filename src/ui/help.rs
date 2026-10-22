//! Plain-language help: short explanations behind each "?" button, and the Help screen (F1).

use eframe::egui::{self, Key, Ui};

use super::app::{self, BestOfBurstApp};
use super::theme::{self, Btn};

pub const KEEP: &str = "How many photos you want to end up with. Click the box and type a number (like 300 for a wedding album) or a share of the shoot (like 10%), or use the quick buttons. BestOfBurst circles that many of the best and puts them in a \"picks\" folder. Every pose you shot three or more times always gets its best frame circled too, even if that means a few more than you asked for, so no moment goes missing. You can change it later without starting over.";

pub const SHOT_TYPE: &str = "What kind of shoot this is, so BestOfBurst knows what matters most. General suits weddings, events and everyday shoots. Portrait cares more about good light and clean skin. Sports cares most about sharp, frozen action. Landscape cares more about contrast and colour.";

pub const ENGINE: &str = "Which part of your computer does the heavy lifting.\n\n• Automatic (recommended): uses the graphics card if your computer has a suitable one, otherwise the processor.\n• Graphics: the graphics card checks every photo. It's built for picture work, so culling finishes sooner.\n• Processor: the main processor does everything. A little slower, but works on every computer.\n\nOpening each photo file is always done by the processor, so a better graphics card speeds things up, but only so far.";

pub const THREADS: &str = "How many photos BestOfBurst opens at the same time. Your processor has a number of cores, like several pairs of hands. Using more at once finishes sooner but leaves less for other programs, so editing or email may feel slower meanwhile. The default leaves one core free. Using all of them with the graphics card is the fastest.";

pub const LIVE: &str = "What your computer is doing right now.\n\n• Processor: how busy your computer's processor is, and how much of that is BestOfBurst.\n• Photos in progress: how many photos are being worked on at this moment.\n• Graphics card: how busy it is. Its cores are hundreds of tiny workers; \"about N busy\" is an estimate.\n• Graphics memory: how much of the card's own memory is in use.\n• Time per photo: on average, how long it takes to open each photo and to check it.";

/// Help screen sections: title and text.
pub const SECTIONS: &[(&str, &str)] = &[
    ("What BestOfBurst does", "You give it a folder from a shoot. It checks every photo for focus, exposure and noise, stacks the bursts (photos of the same moment), and circles the best one of each moment. Your keepers go into a \"picks\" folder. Your original files are never changed, moved or deleted."),
    ("The marks", "A red circle means keeper. A wave underneath means maybe. A cross means set aside (out of focus, too dark, and so on). A \"?\" marks a close call: two photos of the same moment that are nearly as good as each other, so your eye should decide."),
    ("Photos to keep", KEEP),
    ("Type of shoot", SHOT_TYPE),
    ("Speed: graphics card or processor", ENGINE),
    ("Photos at once", THREADS),
    ("Your computer now", LIVE),
    ("The loupe and compare", "The loupe shows one moment at a time with the subject at 100 % so you can check eyes and focus. Press Enter to keep the circled frame and go to the next close call, or P to move the circle. Compare shows two frames on top of each other: wipe with the mouse or blink between them. Press F in compare to fill the screen."),
    ("Full screen and turning photos", "Click the selected photo (or press F) to see it full size on the whole screen. Scroll or press + and - to zoom, drag to move around, double-click or Z for 100 %, and the arrows for the next photo. Photos are shown the right way up, the same as in File Explorer. If one is still on its side, press R or L (or the Turn buttons) to turn it. Turning only changes how BestOfBurst shows the photo; your files are never changed."),
    ("Delivering", "Deliver puts your keepers in a \"picks\" folder inside your shoot folder. Normally these are shortcuts: they look and open like the real photos but use no extra disk space, and deleting one never deletes your original. But if you edit and save a photo inside picks, the original changes too. Tick \"Make full copies\" if you plan to edit them there."),
    ("Glim", "Glim is the little lens helper. Every close call you settle earns a treat; spend them in Glim's corner. After 20 minutes of reviewing, Glim suggests a short break for your eyes."),
    ("Keyboard", "F1 help. Arrows move. P keeper, U maybe, X set aside, 0 clear. Space opens the loupe, N the next close call, C compare, F full screen. R / L turn a photo. + / - zoom. Ctrl+Z undo, Ctrl+E deliver. Esc goes back."),
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
