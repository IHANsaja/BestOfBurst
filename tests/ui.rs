//! The window, driven offscreen with egui_kittest through every Grease Pencil screen: Home,
//! contact sheet, overrides and undo, Focus (loupe), Compare, Done with delivery, Glim's corner,
//! night table and the safelight break. Set BOB_UI_SNAPSHOTS=<dir> to save renders.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bestofburst::engine::{self, RunConfig};
use bestofburst::rank::{Keep, Status};
use bestofburst::ui::app::{BestOfBurstApp, Screen};
use common::*;
use eframe::egui::{self, Key};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

fn shoot(dir: &Path) {
    let mut t = 0u64;
    for shot in 0..5u64 {
        let mut img = scene(1200, 800, 300 + shot);
        let mut rng = Rng::new(shot);
        for v in img.rgb.iter_mut() {
            *v = (*v as f64 + 16.0 * rng.gauss()).round().clamp(0.0, 255.0) as u8;
        }
        for f in 0..3 {
            let frame = if f == 2 { blur(&img, 0.8 + shot as f64 * 0.1) } else { img.clone() };
            save_jpeg(&frame, &dir.join(format!("IMG_{t:04}.jpg")));
            t += 1;
        }
    }
}

fn settle(h: &mut Harness<'_, BestOfBurstApp>, until: impl Fn(&BestOfBurstApp) -> bool) {
    for _ in 0..200 {
        h.step();
        if until(h.state()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // Let background thumbnail/preview decodes and mark draw-ons land.
    for _ in 0..14 {
        std::thread::sleep(Duration::from_millis(45));
        h.step();
    }
}

fn snapshot(h: &mut Harness<'_, BestOfBurstApp>, name: &str) {
    if let Some(dir) = std::env::var_os("BOB_UI_SNAPSHOTS") {
        let img = h.render().expect("render");
        let p = PathBuf::from(dir).join(format!("{name}.png"));
        img.save(&p).expect("save snapshot");
    }
}

fn screen(h: &Harness<'_, BestOfBurstApp>) -> Screen {
    h.state().screen
}

#[test]
fn window_flow() {
    let dir = scratch("ui");
    // Keep recent folders and settings out of the real %APPDATA%.
    let appdata = scratch("ui-appdata");
    std::env::set_var("APPDATA", &appdata);
    shoot(&dir);
    let mut cfg = RunConfig::new(dir.clone());
    cfg.keep = Keep::Count(3);
    cfg.threads = 2;
    let stop = Arc::new(AtomicBool::new(false));
    let o = engine::analyze_folder(&cfg, &stop, |_| {}).unwrap();
    let mut state = o.state;
    engine::rank_and_output(&mut state, &cfg).unwrap();

    let folder = dir.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .wgpu()
        .build_eframe(move |cc| BestOfBurstApp::new(cc, Some(folder)));

    // Home: counts arrive from a background scan; nothing new -> open the contact sheet.
    settle(&mut h, |a| a.folder_info.is_some());
    snapshot(&mut h, "1_home");
    let info = h.state().folder_info.clone().unwrap();
    assert_eq!((info.total, info.analyzed, info.new), (15, 15, 0));
    // Help (F1) and back.
    h.key_press(Key::F1);
    settle(&mut h, |a| a.screen == Screen::Help);
    assert_eq!(screen(&h), Screen::Help);
    snapshot(&mut h, "1a_help");
    h.key_press(Key::Escape);
    settle(&mut h, |a| a.screen == Screen::Home);

    // Develop again: Processing with the live engine panel, then the contact sheet.
    h.get_by_label("Help: Speed").click();
    settle(&mut h, |_| true);
    snapshot(&mut h, "1b_home_engine_help");
    h.get_by_label("Check for new photos").click();
    settle(&mut h, |a| a.prog.finished);
    assert!(h.state().prog.finished);
    std::thread::sleep(Duration::from_millis(900));
    settle(&mut h, |_| true);
    snapshot(&mut h, "1c_processing");
    h.get_by_label("Open contact sheet →").click();
    settle(&mut h, |a| a.screen == Screen::Review);
    assert_eq!(screen(&h), Screen::Review);
    snapshot(&mut h, "2_sheet");

    // Key 2 shows Maybe; P on a maybe moves the circle and demotes the group's old keeper.
    h.key_press(Key::Num2);
    settle(&mut h, |_| true);
    let rv = h.state().review.as_ref().unwrap();
    let sel = rv.selected.expect("a maybe is selected");
    let group = rv.ranking.entries[sel].group;
    let old_pick = rv.ranking.entries.iter().position(|e| e.group == group && e.effective() == Status::Pick);
    h.key_press(Key::P);
    settle(&mut h, |_| true);
    let rv = h.state().review.as_ref().unwrap();
    assert_eq!(rv.ranking.entries[sel].effective(), Status::Pick);
    if let Some(op) = old_pick {
        assert_eq!(rv.ranking.entries[op].effective(), Status::Maybe);
    }
    snapshot(&mut h, "3_after_pick");
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::Z);
    settle(&mut h, |_| true);
    assert!(h.state().review.as_ref().unwrap().state.overrides.is_empty());

    // Night table.
    h.get_by_label("Night table").click();
    h.key_press(Key::Num4);
    settle(&mut h, |a| a.settings.night);
    assert!(h.state().settings.night);
    snapshot(&mut h, "4_sheet_night");

    // N opens the loupe on a close call; Enter keeps and settles it (one treat).
    h.key_press(Key::N);
    settle(&mut h, |a| a.screen == Screen::Focus);
    assert_eq!(screen(&h), Screen::Focus);
    std::thread::sleep(Duration::from_millis(500));
    settle(&mut h, |_| true);
    snapshot(&mut h, "5_focus");
    h.key_press(Key::ArrowRight);
    settle(&mut h, |_| true);
    snapshot(&mut h, "5b_focus_b");
    h.key_press(Key::C);
    settle(&mut h, |a| a.screen == Screen::Compare);
    assert_eq!(screen(&h), Screen::Compare);
    std::thread::sleep(Duration::from_millis(400));
    settle(&mut h, |_| true);
    snapshot(&mut h, "6_compare");
    h.key_press(Key::Num1);
    settle(&mut h, |a| a.screen == Screen::Focus);
    assert_eq!(screen(&h), Screen::Focus);
    let treats_before = h.state().settings.treats;
    assert!(treats_before >= 1, "settling in Compare earns a treat");
    h.key_press(Key::Escape);
    settle(&mut h, |a| a.screen == Screen::Review);
    assert_eq!(screen(&h), Screen::Review);

    // Safelight break after 20 minutes of review.
    h.state_mut().review_clock = Instant::now().checked_sub(Duration::from_secs(21 * 60));
    settle(&mut h, |_| true);
    snapshot(&mut h, "7_break");
    h.get_by_label("Done").click();
    settle(&mut h, |_| true);
    assert!(h.state().break_due().is_none());

    // Changing Keep re-ranks in memory.
    {
        let rv = h.state_mut().review.as_mut().unwrap();
        rv.keep = Keep::Count(5);
        rv.rerank();
    }
    settle(&mut h, |_| true);
    assert_eq!(h.state().review.as_ref().unwrap().ranking.summary.picks, 5);

    // Done and delivery.
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::E);
    settle(&mut h, |a| a.screen == Screen::Done);
    assert_eq!(screen(&h), Screen::Done);
    h.get_by_label("Deliver 5").click();
    settle(&mut h, |a| a.last_delivery.is_some());
    let d = h.state().last_delivery.clone().expect("delivered");
    assert_eq!(d.written, 5);
    snapshot(&mut h, "8_done");

    // Glim's corner.
    h.state_mut().open_corner();
    settle(&mut h, |a| a.screen == Screen::Corner);
    snapshot(&mut h, "9_corner");
    h.get_by_label("Pet").click();
    settle(&mut h, |_| true);
    assert!(h.state().settings.joy > 60.0);
    h.key_press(Key::Escape);
    settle(&mut h, |a| a.screen == Screen::Done);
    assert_eq!(screen(&h), Screen::Done);
}
