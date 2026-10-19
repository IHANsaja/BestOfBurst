//! The window, driven offscreen with egui_kittest: Home -> Open results -> Review, keyboard
//! overrides, undo, Compare and full preview. Set BOB_UI_SNAPSHOTS=<dir> to save renders.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

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
    // Let background thumbnail/preview decodes land.
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(40));
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

#[test]
fn window_flow() {
    let dir = scratch("ui");
    // Keep the recent-folders list out of the real %APPDATA%.
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
        .with_size(egui::vec2(1360.0, 860.0))
        .wgpu()
        .build_eframe(move |cc| BestOfBurstApp::new(cc, Some(folder)));

    // Home: counts arrive from a background scan; nothing new -> "Open results".
    settle(&mut h, |a| a.folder_info.is_some());
    snapshot(&mut h, "1_home");
    let info = h.state().folder_info.clone().unwrap();
    assert_eq!((info.total, info.analyzed, info.new), (15, 15, 0));
    h.get_by_label("Open results").click();
    settle(&mut h, |a| matches!(a.screen, Screen::Review));
    assert!(matches!(h.state().screen, Screen::Review));
    snapshot(&mut h, "2_review");

    // Tabs carry counts; P on a Maybe makes it the pick and demotes the group's old pick.
    let _ = h.get_by_label_contains("Picks 3");
    h.key_press(Key::Num2);
    settle(&mut h, |_| true);
    let rv = h.state().review.as_ref().unwrap();
    let sel = rv.selected.expect("a maybe is selected");
    let group = rv.ranking.entries[sel].group;
    let old_pick = rv
        .ranking
        .entries
        .iter()
        .position(|e| e.group == group && e.effective() == Status::Pick);
    h.key_press(Key::P);
    settle(&mut h, |_| true);
    let rv = h.state().review.as_ref().unwrap();
    assert_eq!(rv.ranking.entries[sel].effective(), Status::Pick);
    if let Some(op) = old_pick {
        assert_eq!(rv.ranking.entries[op].effective(), Status::Maybe);
    }
    snapshot(&mut h, "3_after_pick");

    // Undo restores the ranked statuses.
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::Z);
    settle(&mut h, |_| true);
    assert!(h.state().review.as_ref().unwrap().state.overrides.is_empty());

    // Compare on the selected group, then the full-window preview.
    h.key_press(Key::Num4);
    h.key_press(Key::C);
    settle(&mut h, |a| a.review.as_ref().map(|r| r.compare.is_some()).unwrap_or(false));
    assert!(h.state().review.as_ref().unwrap().compare.is_some());
    snapshot(&mut h, "4_compare");
    h.key_press(Key::Z);
    settle(&mut h, |_| true);
    std::thread::sleep(Duration::from_millis(600));
    settle(&mut h, |_| true);
    snapshot(&mut h, "5_compare_crop");
    h.key_press(Key::Escape);
    h.key_press(Key::Space);
    settle(&mut h, |a| a.review.as_ref().map(|r| r.fullscreen).unwrap_or(false));
    snapshot(&mut h, "6_preview");
    h.key_press(Key::Escape);
    settle(&mut h, |_| true);
    assert!(!h.state().review.as_ref().unwrap().fullscreen);

    // Changing Keep re-ranks in memory.
    {
        let rv = h.state_mut().review.as_mut().unwrap();
        rv.keep = Keep::Count(5);
        rv.rerank();
    }
    settle(&mut h, |_| true);
    assert_eq!(h.state().review.as_ref().unwrap().ranking.summary.picks, 5);
    snapshot(&mut h, "7_keep5");
}
