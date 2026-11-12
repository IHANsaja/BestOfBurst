//! Finding originals from the previews a client sends back.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bestofburst::decode::{self, RgbImage};
use bestofburst::engine::{self, RunConfig};
use bestofburst::find::{self, Index, Shape, Verdict};
use bestofburst::rank::Keep;
use common::*;

/// A scene with detail in it: the smooth field plus a patchwork of 18 px blocks, so that a
/// shifted or cropped copy really is different (as it is in a photo).
fn photo(seed: u64) -> RgbImage {
    let mut img = scene(900, 600, seed);
    let mut rng = Rng::new(seed ^ 0xABCD);
    let (bw, bh) = (img.width.div_ceil(18), img.height.div_ceil(18));
    let blocks: Vec<f64> = (0..bw * bh).map(|_| rng.uniform() * 70.0 - 35.0).collect();
    for y in 0..img.height {
        for x in 0..img.width {
            let d = blocks[((y / 18) * bw + x / 18) as usize];
            let i = ((y * img.width + x) * 3) as usize;
            for c in 0..3 {
                img.rgb[i + c] = (img.rgb[i + c] as f64 + d).clamp(0.0, 255.0) as u8;
            }
        }
    }
    img
}

/// A scene with a small bright card at (cx, cy): two frames of one burst differ only by it.
fn frame(seed: u64, cx: u32, cy: u32) -> RgbImage {
    let mut img = photo(seed);
    for y in cy..cy + 70 {
        for x in cx..cx + 90 {
            let i = ((y * img.width + x) * 3) as usize;
            let v = if (x / 10 + y / 10) % 2 == 0 { 245 } else { 20 };
            img.rgb[i..i + 3].copy_from_slice(&[v, v, v]);
        }
    }
    img
}

/// What a proofing gallery does: shrink, stamp a see-through mark across the middle and a solid
/// logo in the corner, save as a small JPEG.
fn preview(img: &RgbImage, edge: u32, watermark: bool) -> RgbImage {
    let mut p = decode::resize_long_edge(img.clone(), edge).unwrap();
    if watermark {
        let (w, h) = (p.width, p.height);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                // A diagonal band of text-like stripes, 45% white.
                let d = (x as i32 - y as i32 * w as i32 / h as i32).abs();
                let band = d < (w / 9) as i32 && (x / 6) % 2 == 0;
                // A solid logo, bottom right.
                let logo = x > w * 78 / 100 && y > h * 82 / 100;
                for c in 0..3 {
                    let v = p.rgb[i + c] as f32;
                    p.rgb[i + c] = if logo {
                        250
                    } else if band {
                        (v * 0.55 + 255.0 * 0.45) as u8
                    } else {
                        v as u8
                    };
                }
            }
        }
    }
    p
}

fn save(img: &RgbImage, path: &Path, quality: u8) {
    std::fs::write(path, encode_jpeg(img, quality)).unwrap();
}

fn run(index: &Index, queries: &[PathBuf]) -> find::FindReport {
    find::find_originals(index, queries, None, &AtomicBool::new(false), |_, _| {})
}

fn shoot() -> Vec<(String, RgbImage)> {
    let mut v: Vec<(String, RgbImage)> = (0..10u64).map(|k| (format!("IMG_{:04}.jpg", 100 + k), photo(11 + k))).collect();
    // A burst of three: same scene, the card moves.
    for (k, cx) in [200u32, 260, 330].iter().enumerate() {
        v.push((format!("IMG_{:04}.jpg", 200 + k), frame(77, *cx, 250)));
    }
    v
}

#[test]
fn watermarked_previews_find_their_originals() {
    let dir = scratch("find-wm");
    let originals = shoot();
    let index = Index::from_images(&originals);
    let mut queries = Vec::new();
    for (k, (_, img)) in originals.iter().enumerate() {
        // Names say nothing: the match has to come from the pixels.
        let q = dir.join(format!("download ({k}).jpg"));
        save(&preview(img, 480, true), &q, 70);
        queries.push(q);
    }
    let report = run(&index, &queries);
    assert_eq!(report.matches.len(), originals.len());
    for (m, (name, _)) in report.matches.iter().zip(&originals) {
        assert_eq!(m.best.as_ref().map(|b| b.path.as_str()), Some(name.as_str()), "{:?} matched wrongly: {m:?}", m.query);
        assert_ne!(m.verdict, Verdict::NotFound, "{m:?}");
    }
    assert!(
        report.count(Verdict::Found) >= originals.len() - 3,
        "only the burst may need a look: {:?}",
        report.matches.iter().map(|m| m.verdict).collect::<Vec<_>>()
    );
    let wm = report.watermarks.get(&Shape::Landscape).expect("the watermark is learned from the matches");
    assert!(wm.coverage > 0.03 && wm.coverage < 0.5, "coverage {}", wm.coverage);
}

#[test]
fn burst_frames_are_told_apart() {
    let dir = scratch("find-burst");
    let originals = shoot();
    let index = Index::from_images(&originals);
    let mut queries = Vec::new();
    for (k, original) in originals.iter().enumerate().take(13).skip(10) {
        let q = dir.join(format!("pick{k}.jpg"));
        save(&preview(&original.1, 600, false), &q, 80);
        queries.push(q);
    }
    let report = run(&index, &queries);
    for (m, k) in report.matches.iter().zip(10..13) {
        assert_eq!(m.verdict, Verdict::Found, "{m:?}");
        assert_eq!(m.best.as_ref().unwrap().path, originals[k].0);
        // The runner-up is a sibling of the same burst.
        assert!(m.runner_up.as_ref().unwrap().path.starts_with("IMG_02"));
    }
}

#[test]
fn full_size_copies_turned_previews_and_strangers() {
    let dir = scratch("find-misc");
    let originals = shoot();
    let index = Index::from_images(&originals);

    // The original itself under another name.
    let same = dir.join("WhatsApp Image.jpg");
    save(&originals[3].1, &same, 95);
    // A preview turned a quarter (the shoot's thumbnails are stored unrotated).
    let turned = dir.join("turned.jpg");
    save(&decode::apply_orientation(preview(&originals[5].1, 500, false), 6), &turned, 80);
    // A brighter, flatter edit of a photo.
    let mut edit = preview(&originals[7].1, 500, false);
    for v in edit.rgb.iter_mut() {
        *v = (*v as f32 * 0.8 + 40.0) as u8;
    }
    let edited = dir.join("edited.jpg");
    save(&edit, &edited, 80);
    // A photo from another shoot, and a file that is not a photo.
    let stranger = dir.join("stranger.jpg");
    save(&photo(999), &stranger, 85);
    let junk = dir.join("notes.jpg");
    std::fs::write(&junk, [0xFFu8, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();

    let report = run(&index, &[same, turned, edited, stranger, junk]);
    let m = &report.matches;
    assert_eq!((m[0].verdict, m[0].best.as_ref().unwrap().path.as_str()), (Verdict::Found, "IMG_0103.jpg"));
    assert!(m[0].best.as_ref().unwrap().similarity > 95.0);
    assert_eq!((m[1].verdict, m[1].best.as_ref().unwrap().path.as_str()), (Verdict::Found, "IMG_0105.jpg"));
    assert_ne!(m[1].best.as_ref().unwrap().turns, 0);
    assert_eq!((m[2].verdict, m[2].best.as_ref().unwrap().path.as_str()), (Verdict::Found, "IMG_0107.jpg"));
    assert_eq!(m[3].verdict, Verdict::NotFound, "{:?}", m[3]);
    assert_eq!(m[3].original(), None);
    assert_eq!(m[4].verdict, Verdict::Failed);
    assert!(m[4].error.is_some());
}

#[test]
fn a_raw_and_its_jpeg_are_one_frame() {
    let dir = scratch("find-twins");
    let mut originals = shoot();
    // The camera wrote a JPEG beside each "RAW" of the first three frames.
    for k in 0..3 {
        let (name, img) = originals[k].clone();
        originals.push((name.replace(".jpg", ".CR3"), img));
    }
    let index = Index::from_images(&originals);
    let q = dir.join("proof.jpg");
    save(&preview(&originals[1].1, 480, false), &q, 75);
    let m = run(&index, &[q]).matches.remove(0);
    assert_eq!(m.verdict, Verdict::Found, "{m:?}");
    let mut files = m.originals();
    files.sort();
    assert_eq!(files, ["IMG_0101.CR3", "IMG_0101.jpg"]);
    assert!(m.runner_up.as_ref().is_some_and(|r| !r.path.starts_with("IMG_0101")));
}

#[test]
fn cropped_previews_are_lined_up() {
    let dir = scratch("find-crop");
    let originals = shoot();
    let index = Index::from_images(&originals);
    let mut queries = Vec::new();
    // 5% off the left and top, 3% off the right, 8% off the bottom; then the gallery's mark.
    for k in [2usize, 6, 10, 11, 12] {
        let img = &originals[k].1;
        let (x0, y0) = (img.width * 5 / 100, img.height * 5 / 100);
        let (cw, ch) = (img.width * 92 / 100, img.height * 87 / 100);
        let mut rgb = Vec::new();
        for y in y0..y0 + ch {
            let s = ((y * img.width + x0) * 3) as usize;
            rgb.extend_from_slice(&img.rgb[s..s + (cw * 3) as usize]);
        }
        let crop = RgbImage { width: cw, height: ch, rgb };
        let q = dir.join(format!("crop{k}.jpg"));
        save(&preview(&crop, 500, true), &q, 72);
        queries.push(q);
    }
    let report = run(&index, &queries);
    for (m, k) in report.matches.iter().zip([2usize, 6, 10, 11, 12]) {
        let b = m.best.as_ref().unwrap();
        assert_eq!(b.path, originals[k].0, "{m:?}");
        assert_ne!(m.verdict, Verdict::NotFound);
        assert!(b.cropped());
        assert!((b.crop[0] - 0.05).abs() < 0.03 && (b.crop[3] - 0.92).abs() < 0.03, "crop {:?}", b.crop);
    }
}

#[test]
fn file_names_settle_near_ties() {
    assert!(find::names_match("sub\\IMG_1234.CR3", "C:\\in\\IMG_1234-preview.jpg"));
    assert!(find::names_match("IMG_1234.jpg", "0007_IMG_1234.jpg"));
    assert!(!find::names_match("IMG_123.jpg", "IMG_1234.jpg"), "a longer number is another frame");
    assert!(!find::names_match("a1.jpg", "a1 (2).jpg"), "too short to mean anything");
    assert!(!find::names_match("portrait.jpg", "portrait-small.jpg"), "no frame number");
}

#[test]
fn a_swap_takes_the_other_burst_frame() {
    let originals = shoot();
    let index = Index::from_images(&originals);
    let dir = scratch("find-swap");
    let q = dir.join("q.jpg");
    save(&preview(&originals[11].1, 600, false), &q, 80);
    let mut m = run(&index, &[q]).matches.remove(0);
    let (best, other) = (m.best.clone().unwrap().path, m.runner_up.clone().unwrap().path);
    m.swap();
    assert_eq!(m.original(), Some(other.as_str()));
    assert_eq!(m.runner_up.unwrap().path, best);
}

#[test]
fn found_originals_are_delivered_from_a_real_run() {
    let dir = scratch("find-run");
    let originals = shoot();
    for (name, img) in &originals {
        save(img, &dir.join(name), 92);
    }
    let out = dir.join("_bestofburst");
    let mut cfg = RunConfig::new(dir.clone());
    cfg.keep = Keep::Count(3);
    cfg.threads = 2;
    cfg.eyes = false;
    let o = engine::analyze_folder(&cfg, &Arc::new(AtomicBool::new(false)), |_| {}).unwrap();
    let mut state = o.state;
    engine::rank_and_output(&mut state, &cfg).unwrap();
    let before = hash_tree(&dir, &out);

    let sent = scratch("find-run-sent");
    for k in [1usize, 4, 4, 8] {
        save(&preview(&originals[k].1, 480, true), &sent.join(format!("proof-{k}-{}.jpg", std::fs::read_dir(&sent).unwrap().count())), 72);
    }
    save(&photo(4242), &sent.join("other-shoot.jpg"), 80);

    let queries = find::collect_queries(std::slice::from_ref(&sent));
    assert_eq!(queries.len(), 5);
    let index = Index::build(&state, &out);
    assert_eq!(index.len(), originals.len());
    let thumbs = out.join("sent thumbs");
    let report = find::find_originals(&index, &queries, Some(&thumbs), &AtomicBool::new(false), |_, _| {});
    assert_eq!(report.count(Verdict::Found), 4, "{:?}", report.matches);
    assert_eq!(report.count(Verdict::NotFound), 1, "{:?}", report.matches);
    assert!(report.matches.iter().filter(|m| m.verdict == Verdict::Found).all(|m| m.thumb.as_ref().is_some_and(|t| t.exists())));

    let s = find::write_found(&dir, &out, &report.matches, false).unwrap();
    assert!(s.errors.is_empty());
    assert_eq!(s.written, 3, "two previews of one photo deliver it once");
    let mut names: Vec<String> =
        std::fs::read_dir(out.join(find::FOUND_DIR)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    assert_eq!(names, ["IMG_0101.jpg", "IMG_0104.jpg", "IMG_0108.jpg"]);
    let csv = std::fs::read_to_string(out.join("found.csv")).unwrap();
    assert_eq!(csv.lines().count(), 6);
    assert!(csv.contains(",not_found,"));
    // The originals are untouched.
    assert_eq!(before, hash_tree(&dir, &out));
    // A second delivery replaces the folder.
    let one: Vec<_> = report.matches.iter().filter(|m| m.verdict == Verdict::Found).take(1).cloned().collect();
    let s2 = find::write_found(&dir, &out, &one, true).unwrap();
    assert_eq!(s2.written, 1);
    assert_eq!(std::fs::read_dir(out.join(find::FOUND_DIR)).unwrap().count(), 1);
}

// ---- Watermark remover and name lists ----

/// Mean absolute difference per channel value over a rectangle (x0, y0, x1, y1 as 0..1).
fn mean_diff(a: &RgbImage, b: &RgbImage, r: [f32; 4]) -> f32 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (w, h) = (a.width as usize, a.height as usize);
    let (mut sum, mut n) = (0f64, 0usize);
    for y in (r[1] * h as f32) as usize..(r[3] * h as f32) as usize {
        for x in (r[0] * w as f32) as usize..(r[2] * w as f32) as usize {
            for c in 0..3 {
                let i = (y * w + x) * 3 + c;
                sum += (a.rgb[i] as f64 - b.rgb[i] as f64).abs();
                n += 1;
            }
        }
    }
    (sum / n.max(1) as f64) as f32
}

/// A bright, contrasty edit, as a preview exported from an editor has.
fn graded(img: &RgbImage) -> RgbImage {
    let mut out = img.clone();
    for v in out.rgb.iter_mut() {
        *v = (255.0 * (*v as f32 / 255.0).powf(0.75) * 1.05).min(255.0) as u8;
    }
    out
}

#[test]
fn the_watermark_is_removed_and_the_edit_kept() {
    let original = photo(31);
    let edit = graded(&decode::resize_long_edge(original.clone(), 600).unwrap());
    let marked = preview(&graded(&original), 600, true);
    let band = [0.3, 0.3, 0.7, 0.7];
    let before = mean_diff(&marked, &edit, band);
    let cleaned = bestofburst::clean::remove_watermark(&marked, original.clone(), 0, find::FULL_FRAME).unwrap();
    let after = mean_diff(&cleaned.image, &edit, band);
    assert!(before > 8.0, "the test mark is visible: {before}");
    assert!(after < before / 4.0 && after < 5.0, "under the mark: {before} -> {after}");
    assert!(mean_diff(&cleaned.image, &edit, [0.0, 0.0, 1.0, 1.0]) < 4.0);
    // Away from the mark the preview's own pixels are untouched.
    assert!(mean_diff(&cleaned.image, &marked, [0.02, 0.75, 0.3, 0.98]) < 0.5);
    assert!(cleaned.coverage > 0.03 && cleaned.coverage < 0.5, "coverage {}", cleaned.coverage);

    // A different photo is refused rather than blended in.
    assert!(bestofburst::clean::remove_watermark(&marked, photo(77), 0, find::FULL_FRAME).is_err());
}

#[test]
fn clean_previews_come_out_of_a_match() {
    let dir = scratch("clean-run");
    let originals = shoot();
    for (name, img) in &originals {
        save(img, &dir.join(name), 93);
    }
    let sent = scratch("clean-run-sent");
    for k in [0usize, 3, 7, 9, 5] {
        save(&preview(&graded(&originals[k].1), 520, true), &sent.join(format!("proof {k}.jpg")), 80);
    }
    save(&photo(555), &sent.join("elsewhere.jpg"), 80);
    let before = hash_tree(&dir, &dir.join("_bestofburst"));
    let stop = AtomicBool::new(false);
    // No culling was run on this folder: the index is read straight from the photos.
    let index = Index::open(&dir, &stop, |_, _| {});
    assert_eq!(index.len(), originals.len());
    let report = find::find_originals(&index, &find::collect_queries(std::slice::from_ref(&sent)), None, &stop, |_, _| {});
    let out = scratch("clean-run-out");
    let s = bestofburst::clean::clean_all(&dir, &report.matches, &out, &stop, |_, _| {}).unwrap();
    assert_eq!(
        (s.written, s.unmatched),
        (5, 1),
        "{:?} {:?}",
        s.errors,
        report.matches.iter().map(|m| m.best.as_ref().map(|b| (b.path.clone(), b.turns, b.crop, b.similarity))).collect::<Vec<_>>()
    );
    assert!(out.join("proof 3.jpg").exists());
    let data = std::fs::read(out.join("proof 3.jpg")).unwrap();
    let img = decode::decode_jpeg_bytes(&data).unwrap();
    assert_eq!(img.width, 520);
    assert_eq!(before, hash_tree(&dir, &dir.join("_bestofburst")), "originals untouched");
}

#[test]
fn name_lists_take_the_chosen_ending() {
    use bestofburst::output::{with_ending, write_name_lists};
    assert_eq!(with_ending("RNU_2419.JPG", ""), "RNU_2419.JPG");
    assert_eq!(with_ending("RNU_2419.JPG", ".nef"), "RNU_2419.nef");
    assert_eq!(with_ending("RNU_2419.JPG", "NEF"), "RNU_2419.NEF");
    assert_eq!(with_ending("RNU_2419.JPG", "none"), "RNU_2419");
    let dir = scratch("names");
    let picked = vec!["day1\\RNU_2419.JPG".to_string(), "day1\\RNU_2419.NEF".to_string(), "RNU_2420.JPG".to_string()];
    let (txt, csv) = write_name_lists(&dir, "client selection", &picked, "nef").unwrap();
    assert_eq!(std::fs::read_to_string(&txt).unwrap(), "RNU_2419.nef\r\nRNU_2420.nef\r\n", "a RAW + JPEG pair is one name");
    assert_eq!(std::fs::read_to_string(&csv).unwrap().lines().collect::<Vec<_>>(), ["file", "RNU_2419.nef", "RNU_2420.nef"]);
    let (txt, _) = write_name_lists(&dir, "keepers", &picked, "").unwrap();
    assert_eq!(std::fs::read_to_string(&txt).unwrap().lines().count(), 3);
}
