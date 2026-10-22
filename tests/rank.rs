//! Ranking, state, output and originals.

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bestofburst::analyze::Metrics;
use bestofburst::engine::{self, RunConfig};
use bestofburst::output::{self, OutputOptions};
use bestofburst::rank::{self, Entry, Keep, Preset, Status};
use bestofburst::state::{FileEntry, State};
use common::*;

fn metrics(sharp: f32) -> Metrics {
    Metrics {
        sharpness: sharp,
        tenengrad: sharp * 20.0,
        mean_luma: 0.45,
        clip_shadows: 0.01,
        clip_highlights: 0.0,
        contrast: 0.2,
        noise: 2.0,
        colorfulness: 30.0,
    }
}

fn entry(path: &str, sharp: f32, hash: u64, t: i64) -> Entry {
    Entry::new(path, metrics(sharp), [sharp; 64], hash, t)
}

/// Flip `n` low bits.
fn near(hash: u64, n: u32) -> u64 {
    hash ^ ((1u64 << n) - 1)
}

const A: u64 = 0xF0F0_F0F0_0F0F_0F0F;
const B: u64 = 0x0123_4567_89AB_CDEF;
const C: u64 = 0xFEDC_BA98_7654_3210;
const D: u64 = 0xAAAA_5555_AAAA_5555;

#[test]
fn bursts_join_within_two_seconds_and_distant_hashes_split() {
    let mut e = vec![
        entry("a1", 500.0, A, 0),
        entry("a2", 500.0, near(A, 8), 1500),
        entry("a3", 500.0, near(A, 9), 3000),
        entry("b1", 500.0, B, 4000),
        entry("a4", 500.0, near(A, 9), 200_000), // far later, distance 9 > 6: new group
    ];
    rank::group_entries(&mut e);
    assert_eq!(e[0].group, e[1].group);
    assert_eq!(e[1].group, e[2].group);
    assert_ne!(e[2].group, e[3].group);
    assert_ne!(e[0].group, e[4].group);
    // Near-duplicates (distance <= 4) join regardless of time gap.
    let mut e = vec![entry("x", 500.0, A, 0), entry("y", 500.0, near(A, 3), 60_000)];
    rank::group_entries(&mut e);
    assert_eq!(e[0].group, e[1].group);
}

#[test]
fn lookback_merge_returns_to_shot_a() {
    let mut e = vec![
        entry("1_a", 500.0, A, 0),
        entry("2_b", 500.0, B, 10_000),
        entry("3_c", 500.0, C, 20_000),
        entry("4_a_again", 500.0, near(A, 5), 60_000),
    ];
    rank::group_entries(&mut e);
    assert_eq!(e[3].group, e[0].group);
    assert_ne!(e[1].group, e[0].group);
    assert_ne!(e[2].group, e[0].group);
}

fn run_rank(mut e: Vec<Entry>, target: usize, max_per_group: usize) -> Vec<Entry> {
    let w = Preset::General.weights();
    rank::group_entries(&mut e);
    rank::apply_rejects(&mut e);
    rank::score_entries(&mut e, &w);
    rank::group_scores(&mut e, &w);
    rank::select(&mut e, target, max_per_group);
    rank::renumber(&mut e);
    e
}

#[test]
fn picks_respect_target_group_limit_and_rejects() {
    let mut e = Vec::new();
    // Three groups of four frames, plus a blurry and a black frame.
    for (g, h) in [A, B, C].iter().enumerate() {
        for k in 0..4 {
            e.push(entry(&format!("g{g}_{k}"), 400.0 + (k * 50) as f32 + g as f32, near(*h, k as u32), (g as i64) * 100_000 + k as i64 * 500));
        }
    }
    let mut blurry = entry("blurry", 3.0, D, 900_000);
    blurry.metrics.contrast = 0.2;
    e.push(blurry);
    let mut dark = entry("dark", 900.0, !D, 950_000);
    dark.metrics.mean_luma = 0.02;
    dark.metrics.clip_shadows = 0.8;
    e.push(dark);

    let r = run_rank(e.clone(), 100, 2);
    let picks: Vec<&Entry> = r.iter().filter(|x| x.status == Status::Pick).collect();
    assert_eq!(picks.len(), 6, "3 groups x 2");
    let mut per_group: BTreeMap<u64, usize> = BTreeMap::new();
    for p in &picks {
        *per_group.entry(p.group).or_default() += 1;
    }
    assert!(per_group.values().all(|&n| n <= 2));
    for x in &r {
        if !x.reasons.is_empty() {
            assert_ne!(x.status, Status::Pick, "{} rejected but picked", x.path);
        }
    }
    assert!(r.iter().find(|x| x.path == "blurry").unwrap().reasons.contains(&"blurry".to_string()));
    let dark = r.iter().find(|x| x.path == "dark").unwrap();
    assert!(dark.reasons.contains(&"too_dark".to_string()));
    assert!(dark.reasons.contains(&"underexposed".to_string()));

    // Every group has 4 frames, so each keeps one even though only 2 were asked for.
    let r = run_rank(e.clone(), 2, 1);
    assert_eq!(r.iter().filter(|x| x.status == Status::Pick).count(), 3);
    let ranks: Vec<usize> = r.iter().filter_map(|x| x.rank).collect();
    assert_eq!(ranks.len(), 3);

    // Default max-per-group 1: one pick per group even with a big target.
    let r = run_rank(e, 100, 1);
    assert_eq!(r.iter().filter(|x| x.status == Status::Pick).count(), 3);
}

#[test]
fn every_pose_shot_three_times_keeps_one() {
    let mut e = Vec::new();
    // A pose shot 3 times in a dim room: low brightness, deep shadows, softer than the rest.
    for k in 0..3 {
        let mut x = entry(&format!("dim_{k}"), 300.0 + k as f32, near(A, k as u32), k as i64 * 400);
        x.metrics.mean_luma = 0.12;
        x.metrics.clip_shadows = 0.35;
        x.metrics.contrast = 0.08;
        e.push(x);
    }
    // A pose shot 4 times where every frame is out of focus: nothing to keep.
    for k in 0..4 {
        e.push(entry(&format!("soft_{k}"), 3.0, near(B, k as u32), 100_000 + k as i64 * 400));
    }
    // Five bright pairs that would win on score alone.
    for (g, h) in [C, D, !A, !B, !C].iter().enumerate() {
        for k in 0..2 {
            e.push(entry(&format!("pair{g}_{k}"), 800.0 + g as f32, near(*h, k as u32), 200_000 + g as i64 * 100_000 + k as i64 * 400));
        }
    }
    let r = run_rank(e, 2, 1);
    let picked = |p: &str| r.iter().filter(|x| x.path.starts_with(p) && x.status == Status::Pick).count();
    assert_eq!(picked("dim_"), 1, "the dim pose keeps its best frame");
    assert_eq!(picked("soft_"), 0, "set-aside frames are never picked");
    assert_eq!(picked("pair"), 1, "the place left in the target goes to the best of the rest");
}

#[test]
fn dim_scenes_keep_most_of_their_exposure_credit() {
    let mut m = metrics(500.0);
    let mid = rank::exposure(&m);
    m.mean_luma = 0.12;
    m.clip_shadows = 0.35;
    let dim = rank::exposure(&m);
    assert!(dim > 0.4 * mid, "dim {dim} vs mid {mid}");
    m.mean_luma = 0.45;
    m.clip_shadows = 0.0;
    m.clip_highlights = 0.2;
    assert!(rank::exposure(&m) < 0.25 * mid, "blown highlights still cost the most");
}

#[test]
fn keep_percent_rounds_up() {
    assert_eq!(Keep::Percent(10.0).target(101), 11);
    assert_eq!(Keep::Percent(10.0).target(100), 10);
    assert_eq!(Keep::Count(7).target(3), 7);
    assert_eq!(Keep::parse("12%"), Some(Keep::Percent(12.0)));
    assert_eq!(Keep::parse("1000"), Some(Keep::Count(1000)));
}

#[test]
fn subject_sharpness_prefers_sharp_centre() {
    let centre = [27usize, 28, 35, 36];
    let mut bg_sharp = entry("frame_a", 900.0, A, 0);
    let mut centre_sharp = entry("frame_b", 600.0, near(A, 2), 300);
    for t in 0..64 {
        let c = centre.contains(&t);
        bg_sharp.tiles[t] = if c { 50.0 } else { 400.0 };
        centre_sharp.tiles[t] = if c { 1000.0 } else { 100.0 };
    }
    let ss = rank::subject_sharpness(&[&bg_sharp, &centre_sharp]);
    assert!(ss[1] > ss[0]);

    let r = run_rank(vec![bg_sharp, centre_sharp, entry("other", 500.0, B, 900_000)], 10, 1);
    let a = r.iter().find(|x| x.path == "frame_a").unwrap();
    let b = r.iter().find(|x| x.path == "frame_b").unwrap();
    assert_eq!(a.group, b.group);
    assert!(a.score > b.score, "frame a is sharper overall");
    assert_eq!(b.status, Status::Pick, "but frame b is sharp on the subject");
    assert_eq!(a.status, Status::Maybe);
}

#[test]
fn close_calls_are_flagged() {
    let r = run_rank(vec![entry("x1", 500.0, A, 0), entry("x2", 501.0, near(A, 1), 100), entry("y", 900.0, B, 900_000)], 10, 1);
    assert!(r.iter().find(|x| x.path == "x1").unwrap().close_call);
    assert!(!r.iter().find(|x| x.path == "y").unwrap().close_call);
}

fn fake_state(dir: &std::path::Path) -> State {
    let mut s = State::new(dir);
    for (k, h) in [A, B, C].iter().enumerate() {
        s.files.insert(
            format!("sub\\img{k}.jpg"),
            FileEntry {
                size: 12345 + k as u64,
                mtime_ms: 1_700_000_000_000 + k as i64,
                status: "done".into(),
                format: "jpeg".into(),
                decoder: Some("zune-jpeg".into()),
                capture_ms: Some(1_700_000_000_000 + k as i64 * 600_000),
                dhash: Some(format!("{h:016x}")),
                metrics: Some(metrics(300.0 + k as f32 * 100.0)),
                error: None,
                thumb: Some(format!("thumbs\\{k}.jpg")),
                tiles: Some(vec![12.5; 64]),
            },
        );
    }
    s.files.insert(
        "bad.heic".into(),
        FileEntry {
            size: 999_999,
            mtime_ms: 5,
            status: "failed".into(),
            format: "heif".into(),
            decoder: Some("wic".into()),
            capture_ms: None,
            dhash: None,
            metrics: None,
            error: Some("No Windows codec for this format.".into()),
            thumb: None,
            tiles: None,
        },
    );
    s
}

#[test]
fn state_round_trip_and_safe_save() {
    let dir = scratch("state");
    let out = dir.join("_bestofburst");
    let mut s = fake_state(&dir);
    s.overrides.insert("sub\\img0.jpg".into(), "pick".into());
    let r = rank::rank(&s, Preset::General, Keep::Count(1), 1);
    rank::store(&mut s, &r);
    s.save(&out).unwrap();
    let (loaded, warn) = State::load(&dir, &out).unwrap();
    assert!(warn.is_none());
    assert_eq!(loaded, s);

    // A crash mid-save leaves only a partial .tmp; state.json is untouched.
    std::fs::write(out.join("state.json.tmp"), b"{\"version\": 1, \"inp").unwrap();
    let (again, _) = State::load(&dir, &out).unwrap();
    assert_eq!(again, s);

    // Corrupt state is set aside and a fresh one starts.
    std::fs::write(out.join("state.json"), b"not json").unwrap();
    let (fresh, warn) = State::load(&dir, &out).unwrap();
    assert!(fresh.files.is_empty());
    assert!(warn.is_some());
    assert!(std::fs::read_dir(&out).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("state.corrupt-")));
}

#[test]
fn overrides_apply_and_survive_restart() {
    let dir = scratch("overrides");
    let out = dir.join("_bestofburst");
    let mut s = fake_state(&dir);
    let r = rank::rank(&s, Preset::General, Keep::Count(1), 1);
    let top = r.entries.iter().find(|e| e.rank == Some(1)).unwrap().path.clone();
    let low = r.entries.iter().find(|e| e.ok && e.path != top).unwrap().path.clone();
    s.overrides.insert(top.clone(), "reject".into());
    s.overrides.insert(low.clone(), "pick".into());
    s.save(&out).unwrap();
    let (s2, _) = State::load(&dir, &out).unwrap();
    let r2 = rank::rank(&s2, Preset::General, Keep::Count(1), 1);
    let e_top = r2.entries.iter().find(|e| e.path == top).unwrap();
    let e_low = r2.entries.iter().find(|e| e.path == low).unwrap();
    assert_eq!(e_top.effective(), Status::Reject);
    assert_eq!(e_low.effective(), Status::Pick);
    assert_eq!(e_low.rank, Some(1));
}

fn make_shoot(dir: &std::path::Path) {
    let sub = dir.join("Ceremony");
    std::fs::create_dir_all(&sub).unwrap();
    for k in 0..6u64 {
        let mut img = scene(900, 600, 10 + k);
        // Photographic texture, so the sharp frame has real detail to lose.
        let mut rng = Rng::new(100 + k);
        for v in img.rgb.iter_mut() {
            *v = (*v as f64 + 18.0 * rng.gauss()).round().clamp(0.0, 255.0) as u8;
        }
        save_jpeg(&img, &sub.join(format!("IMG_{k:04}.jpg")));
        // A softer duplicate of each shot.
        save_jpeg(&blur(&img, 2.5), &sub.join(format!("IMG_{k:04}b.jpg")));
    }
    save_jpeg(&gray(900, 600, 3), &dir.join("black.jpg"));
    std::fs::write(dir.join("notes.xmp"), vec![b'x'; 20_000]).unwrap();
    std::fs::write(dir.join("broken.jpg"), {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.extend(std::iter::repeat(0x11).take(20_000));
        v
    })
    .unwrap();
}

#[test]
fn full_run_resume_output_and_originals_untouched() {
    let dir = scratch("run");
    make_shoot(&dir);
    let out = dir.join("_bestofburst");
    let before = hash_tree(&dir, &out);

    let mut cfg = RunConfig::new(dir.clone());
    cfg.keep = Keep::Count(4);
    cfg.threads = 2;
    cfg.output = OutputOptions { maybe: true, rejects: true, copy: false, keep_names: false };
    let stop = Arc::new(AtomicBool::new(false));
    let o = engine::analyze_folder(&cfg, &stop, |_| {}).unwrap();
    assert!(!o.stopped);
    assert_eq!(o.total, 14, "12 shots + black + broken; sidecar skipped");
    assert_eq!(o.new, 14);
    assert_eq!(o.failed_now, 1);
    let mut state = o.state;
    let (ranking, summary) = engine::rank_and_output(&mut state, &cfg).unwrap();
    assert_eq!(ranking.summary.picks, 4);
    assert!(summary.errors.is_empty());

    // Picks are rank-prefixed; each shot pair contributes at most one pick.
    let mut names: Vec<String> = std::fs::read_dir(out.join("picks")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    assert_eq!(names.len(), 4);
    assert!(names[0].starts_with("0001_IMG_"), "{names:?}");
    assert!(names.iter().all(|n| !n.contains("b.jpg")), "sharp frame wins each pair: {names:?}");
    assert!(out.join("rejects").join("unreadable").join("broken.jpg").exists());
    assert!(out.join("report.csv").exists());
    let report = std::fs::read_to_string(out.join("report.csv")).unwrap();
    assert_eq!(report.lines().count(), 15);
    assert!(report.lines().nth(1).unwrap().starts_with("1,pick,"));

    // Resume: nothing new.
    let o2 = engine::analyze_folder(&cfg, &stop, |_| {}).unwrap();
    assert_eq!(o2.new, 0);

    // Retry failed re-analyzes only the broken file.
    let mut retry = cfg.clone();
    retry.retry_failed = true;
    let o3 = engine::analyze_folder(&retry, &stop, |_| {}).unwrap();
    assert_eq!(o3.new, 1);

    // Owned folders only: foreign files survive, stale picks are cleared.
    std::fs::write(out.join("mine.txt"), b"keep me").unwrap();
    std::fs::write(out.join("picks").join("stale.jpg"), b"old").unwrap();
    let mut copy_cfg = cfg.clone();
    copy_cfg.output.copy = true;
    copy_cfg.output.keep_names = true;
    copy_cfg.keep = Keep::Count(2);
    let mut st = o3.state;
    engine::rank_and_output(&mut st, &copy_cfg).unwrap();
    assert!(out.join("mine.txt").exists());
    assert!(!out.join("picks").join("stale.jpg").exists());
    let names: Vec<String> = std::fs::read_dir(out.join("picks")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.iter().all(|n| n.starts_with("IMG_")), "{names:?}");

    // Originals identical, none missing.
    let after = hash_tree(&dir, &out);
    assert_eq!(before, after);
}

#[test]
fn out_equal_to_input_is_refused() {
    let dir = scratch("refuse");
    assert!(output::check_out(&dir, &dir).is_err());
    let mut cfg = RunConfig::new(dir.clone());
    cfg.out = dir.clone();
    let stop = Arc::new(AtomicBool::new(false));
    assert!(engine::analyze_folder(&cfg, &stop, |_| {}).is_err());
}
