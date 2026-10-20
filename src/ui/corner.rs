//! Glim's corner (screen 7): Glim on the light table, Joy and Energy rulers, Pet / Treat / Nap,
//! and "Sharp Eye": circle the one sharp frame in a strip from your own shoot.

use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, vec2, Key, Rect, Sense, Ui};

use super::app::{self, BestOfBurstApp};
use super::glim::{self, Mood};
use super::marks;
use super::review::{frame_card, CardOpts};
use super::theme::{self, Btn};
use crate::rank;

#[derive(Default)]
pub struct CornerState {
    pub round: Option<Round>,
    pub score: u32,
    pub rounds: u32,
    mood: Option<(Mood, Instant)>,
    seed: u64,
}

pub struct Round {
    pub frames: Vec<usize>,
    /// The frame that is sharpest on the subject.
    pub answer: usize,
    pub picked: Option<usize>,
}

fn next_rand(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// A strip of 3-4 frames from one group; the answer is the sharpest on the group's subject.
fn new_round(app: &mut BestOfBurstApp) -> Option<Round> {
    let rv = app.review.as_ref()?;
    let mut groups: Vec<&Vec<usize>> = rv.groups.values().filter(|m| m.len() >= 3).collect();
    if groups.is_empty() {
        groups = rv.groups.values().filter(|m| m.len() >= 2).collect();
    }
    if groups.is_empty() {
        return None;
    }
    groups.sort_by_key(|m| m[0]);
    if app.corner.seed == 0 {
        app.corner.seed = (app.time() * 1e6) as u64 | 1;
    }
    let g = groups[(next_rand(&mut app.corner.seed) % groups.len() as u64) as usize];
    let mut frames: Vec<usize> = g.iter().copied().take(4).collect();
    let refs: Vec<&rank::Entry> = frames.iter().map(|&i| &rv.ranking.entries[i]).collect();
    let ss = rank::subject_sharpness(&refs);
    let best = ss.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(k, _)| frames[k])?;
    // Shuffle so the answer is not always first.
    for k in (1..frames.len()).rev() {
        let j = (next_rand(&mut app.corner.seed) % (k as u64 + 1)) as usize;
        frames.swap(k, j);
    }
    Some(Round { frames, answer: best, picked: None })
}

pub fn show(app: &mut BestOfBurstApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let p = theme::pal(false);
    let back_to = app.corner_return;
    let mut back = false;
    let mut home = false;
    app::chrome_bar(
        ui,
        "corner_header",
        |ui| {
            home = app::logo_cell(ui);
            app::text_cell(ui, "Glim's corner", true);
        },
        |ui| {
            ui.add_space(16.0);
            ui.visuals_mut().override_text_color = Some(theme::TABLE);
            back = theme::button(ui, "Back", Some("Esc"), Btn::MonoSmall, 40.0, None, true).clicked();
            ui.add_space(16.0);
            app::cell_rule(ui);
            app::text_cell(ui, &theme::plural(app.settings.treats as usize, "treat"), false);
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

    let mood = match app.corner.mood {
        Some((m, at)) if at.elapsed() < Duration::from_secs(3) => m,
        _ => {
            if app.settings.energy < 25.0 { Mood::Sleepy } else { Mood::Idle }
        }
    };

    // Sharp Eye.
    let side = egui::Frame::new().fill(theme::PAPER_DAY).inner_margin(28.0);
    egui::Panel::right("corner_side").exact_size(560.0).resizable(false).frame(side).show(ui, |ui| {
        ui.label(theme::mono("Mini-game", 11.0, theme::GRAPHITE));
        ui.label(theme::xp("Sharp Eye", 26.0, theme::INK));
        ui.label(theme::body("Circle the one frame that's sharp on the subject. Frames come from your own shoot.", 14.0, theme::INK_2));
        ui.add_space(12.0);
        if app.corner.round.is_none() && app.review.is_some() {
            app.corner.round = new_round(app);
        }
        let Some(round) = app.corner.round.as_ref().map(|r| (r.frames.clone(), r.answer, r.picked)) else {
            ui.label(theme::body("Open a shoot's contact sheet first, then come back to play.", 14.0, theme::GRAPHITE));
            return;
        };
        let (frames, answer, picked) = round;
        let cw = ((ui.available_width() - 8.0) / 2.0).floor();
        let mut clicked = None;
        if let Some(rv) = app.review.take() {
            for chunk in frames.chunks(2) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for &f in chunk {
                        let r = frame_card(ui, &rv, app, f, vec2(cw, cw * 0.75), CardOpts { count: 1, selected: picked == Some(f), letter: None, show_score: false, plain: true });
                        if picked.is_some() && f == answer {
                            marks::keeper(ui, r.rect.shrink(10.0), theme::RED, 4.0, 1.0);
                        }
                        if r.clicked() && picked.is_none() {
                            clicked = Some(f);
                        }
                    }
                });
            }
            app.review = Some(rv);
        }
        if let Some(f) = clicked {
            if let Some(r) = &mut app.corner.round {
                r.picked = Some(f);
            }
            app.corner.rounds += 1;
            app.settings.energy = (app.settings.energy - 4.0).max(0.0);
            if f == answer {
                app.corner.score += 1;
                app.settings.joy = (app.settings.joy + 6.0).min(100.0);
                app.settings.sharp_eye_best = app.settings.sharp_eye_best.max(app.corner.score);
                app.corner.mood = Some((Mood::Cheer, Instant::now()));
            } else {
                app.corner.mood = Some((Mood::Idle, Instant::now()));
            }
        }
        ui.add_space(10.0);
        let verdict = match picked {
            None => "Pick one.".to_string(),
            Some(f) if f == answer => "Sharp eye! That's the one.".into(),
            Some(_) => "Close. The circled one is sharper on the subject.".into(),
        };
        ui.label(theme::xp(verdict, 16.0, theme::INK));
        ui.label(theme::mono(format!("Score {} / {} · best {}", app.corner.score, app.corner.rounds, app.settings.sharp_eye_best), 10.5, theme::GRAPHITE));
        ui.add_space(8.0);
        if theme::button(ui, "Next strip", Some("enter"), Btn::OnDay, 52.0, None, picked.is_some()).clicked()
            || (picked.is_some() && ctx.input(|i| i.key_pressed(Key::Enter)))
        {
            app.corner.round = new_round(app);
        }
    });

    // Habitat.
    let main = egui::Frame::new().fill(theme::TABLE).inner_margin(48.0);
    egui::CentralPanel::default().frame(main).show(ui, |ui| {
        ui.label(theme::mono("Habitat", 11.0, theme::GRAPHITE));
        let w = ui.available_width();
        let size = 290.0f32.min(w * 0.6);
        let (rect, resp) = ui.allocate_exact_size(vec2(w, size + 24.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Glim"));
        let paper = Rect::from_min_size(pos2(rect.min.x, rect.min.y + 8.0), vec2(size + 40.0, size + 16.0));
        ui.painter().rect_filled(paper, 0.0, theme::PAPER_DAY);
        glim::paint(ui, Rect::from_min_size(paper.min + vec2(20.0, 8.0), vec2(size, size)), mood, false, app.time(), app.reduced_motion, 1.0);
        let say = match mood {
            Mood::Cheer => "Yes!",
            Mood::Happy => "That's nice.",
            Mood::Sleepy => "Zz... a nap would help.",
            Mood::Idle => "Hi. Settle close calls to earn treats.",
        };
        theme::text_at(ui, paper.right_top() + vec2(24.0, 24.0), egui::Align2::LEFT_TOP, say, theme::f_body(16.0), theme::INK, 0.0);
        ui.add_space(12.0);
        theme::ruler(ui, p, "Joy", app.settings.joy / 100.0, None, &format!("{:.0}", app.settings.joy));
        theme::ruler(ui, p, "Energy", app.settings.energy / 100.0, None, &format!("{:.0}", app.settings.energy));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if theme::button(ui, "Pet", None, Btn::OnDay, 52.0, None, true).clicked() {
                app.settings.joy = (app.settings.joy + 5.0).min(100.0);
                app.corner.mood = Some((Mood::Happy, Instant::now()));
            }
            let treats = app.settings.treats;
            if theme::button(ui, &format!("Treat ({treats})"), None, Btn::Primary, 52.0, None, treats > 0).clicked() {
                app.settings.treats -= 1;
                app.settings.joy = (app.settings.joy + 12.0).min(100.0);
                app.settings.energy = (app.settings.energy + 5.0).min(100.0);
                app.corner.mood = Some((Mood::Cheer, Instant::now()));
            }
            if theme::button(ui, "Nap", None, Btn::OnDay, 52.0, None, true).clicked() {
                app.settings.energy = (app.settings.energy + 30.0).min(100.0);
                app.corner.mood = Some((Mood::Sleepy, Instant::now()));
            }
        });
        ui.add_space(10.0);
        ui.label(theme::body(format!("{} settled so far. Each one earns a treat.", theme::plural(app.settings.settled as usize, "close call")), 14.0, theme::INK_2));
    });
}
