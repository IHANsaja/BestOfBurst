//! The activation gate: the very first screen until a valid license key is entered. Nothing
//! else in the app is reachable without one (the invariant is re-checked every frame in
//! `app.rs`). The key is verified against the embedded public key — see `crate::license`.

use eframe::egui::{self, vec2, Key, Sense};

use super::app::{self, BestOfBurstApp, Screen};
use super::marks;
use super::theme::{self, Btn};
use crate::license;

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    app::chrome_bar(
        ui,
        "activate_header",
        |ui| {
            let _ = app::logo_cell(ui);
            app::text_cell(ui, "Activate", true);
        },
        |_| {},
    );

    let main = egui::Frame::new().fill(theme::TABLE).inner_margin(48.0);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            marks::burst(ui, r, false);
            ui.add_space(6.0);
            ui.label(theme::mono("BestOfBurst · licensed software", 11.0, theme::GRAPHITE));
        });
        ui.add_space(14.0);

        let w = ui.available_width().min(760.0);
        let size = theme::fit_size(ui, "ENTER YOUR LICENSE KEY", 56.0, w, false);
        ui.label(theme::xp("Enter your license key", size, theme::INK));
        ui.add_space(10.0);
        ui.label(theme::body(
            "BestOfBurst is a licensed, commercial product. Paste the license key you were given to unlock it. Your key is checked on this computer — nothing is sent anywhere.",
            15.0,
            theme::GRAPHITE,
        ));
        ui.add_space(26.0);

        // The key field.
        let field = egui::TextEdit::singleline(&mut app.activate_input)
            .font(theme::f_mono_semi(15.0))
            .hint_text("XXXXX-XXXXX-XXXXX-…")
            .desired_width(w);
        let resp = ui.add(field);
        let submitted = resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter));
        ui.add_space(16.0);

        let mut go = false;
        ui.horizontal(|ui| {
            if theme::button(ui, "Activate", Some("↵"), Btn::Primary, 52.0, Some(220.0), !app.activate_input.trim().is_empty()).clicked() {
                go = true;
            }
            ui.add_space(14.0);
            if let Some(err) = &app.activate_error {
                ui.label(theme::body(err.clone(), 14.0, theme::RED));
            }
        });

        if go || submitted {
            attempt(app);
        }

        ui.add_space(40.0);
        theme::rule(ui, theme::ASH, 1.0);
        ui.add_space(16.0);

        // The machine code lets you issue a key locked to this computer if you choose to.
        ui.label(theme::mono("NO KEY YET?", 11.0, theme::GRAPHITE));
        ui.add_space(6.0);
        ui.label(theme::body(
            "Contact the publisher to purchase a license. If your key is tied to this computer, give them the code below.",
            14.0,
            theme::GRAPHITE,
        ));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(theme::mono("THIS COMPUTER", 11.0, theme::GRAPHITE));
            ui.add_space(10.0);
            let code = license::machine_code();
            let r = ui.label(theme::mono(&code, 13.0, theme::INK));
            ui.add_space(8.0);
            if theme::button(ui, "Copy", None, Btn::MonoSmall, 28.0, None, true).clicked() {
                ctx.copy_text(code);
            }
            let _ = r;
        });
    });
}

fn attempt(app: &mut BestOfBurstApp) {
    let input = app.activate_input.trim().to_string();
    match license::activate(&input) {
        Ok(license) => {
            app.license = Some(license);
            app.activate_error = None;
            app.activate_input.clear();
            app.flash(format!("Activated — {} edition. Welcome.", license.edition.label()), false);
            // Hand off to the normal first screen.
            app.screen = Screen::Start;
        }
        Err(e) => {
            app.activate_error = Some(format!("{e}. Check the key and try again."));
        }
    }
}

// Re-export egui::Ui for the signature above without pulling the whole prelude.
use eframe::egui::Ui;
