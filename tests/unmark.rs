//! Watermark remover without originals, end to end on a synthetic set: varied photos carrying
//! the same see-through text mark, saved as JPEG (quality 85, colour at half resolution) like a
//! real preview. The mark is learned from them and removed with the classical edge fill, and
//! with the AI edge clean-up when its model is installed; both are compared with the clean
//! photos.
//!
//! What is asserted for the AI step: it brings the result closer to the clean photo along the
//! mark's border (PSNR there), where the outline is, and costs at most 0.25 dB of PSNR over the
//! whole photo (it repaints a wider band than the classical fill, and painted texture is
//! plausible rather than the original's exact pixels). The outline scores are printed for
//! comparison: the pixel-wise one counts any painted texture that differs from the original as
//! error, so it does not favour the AI on this set even where the outline is gone.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize};

use bestofburst::decode::RgbImage;
use bestofburst::{inpaint, unmark};
use common::Rng;

const W: usize = 768;
const H: usize = 512;
const OPACITY: f32 = 0.4;

/// 5 x 7 letters of the mark.
const GLYPHS: [[&str; 7]; 5] = [
    ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
    ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
    ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
];

/// How much the mark covers each pixel (0..1): "PROOF", centred, 80 % of the width,
/// anti-aliased by 4 x 4 supersampling.
fn mark_cover() -> Vec<f32> {
    let cols = 5 * 5 + 4;
    let cell = 0.8 * W as f32 / cols as f32;
    let (x0, y0) = ((W as f32 - cols as f32 * cell) / 2.0, (H as f32 - 7.0 * cell) / 2.0);
    let inside = |x: f32, y: f32| -> bool {
        let (cx, cy) = (((x - x0) / cell).floor(), ((y - y0) / cell).floor());
        if cx < 0.0 || cy < 0.0 || cy >= 7.0 || cx >= cols as f32 {
            return false;
        }
        let (letter, col) = ((cx as usize) / 6, (cx as usize) % 6);
        col < 5 && GLYPHS[letter][cy as usize].as_bytes()[col] == b'1'
    };
    let mut cover = vec![0f32; W * H];
    for y in 0..H {
        for x in 0..W {
            let mut n = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    if inside(x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0) {
                        n += 1;
                    }
                }
            }
            cover[y * W + x] = n as f32 / 16.0;
        }
    }
    cover
}

/// A varied clean photo: smooth colour fields with fine texture, a dark "suit" block in some,
/// stripes ("fabric") and speckle ("leaves") in others, and in all of them sharp-edged shapes
/// and thin lines (collars, lapels, branches) that cross the mark's outline.
fn photo(seed: u64) -> RgbImage {
    let base = common::scene(W as u32, H as u32, seed);
    let mut rng = Rng::new(seed ^ 0xABCD);
    let mut rgb = base.rgb.clone();
    for _ in 0..9 {
        let colour = [rng.uniform() * 230.0 + 10.0, rng.uniform() * 230.0 + 10.0, rng.uniform() * 230.0 + 10.0];
        let (cx, cy) = (rng.uniform() * W as f64, rng.uniform() * H as f64);
        let (rx, ry) = (20.0 + rng.uniform() * 140.0, 20.0 + rng.uniform() * 120.0);
        let shape = (rng.uniform() * 3.0) as u32;
        let angle = rng.uniform() * std::f64::consts::PI;
        for y in 0..H {
            for x in 0..W {
                let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                let inside = match shape {
                    0 => dx.abs() < rx && dy.abs() < ry,
                    1 => (dx / rx).powi(2) + (dy / ry).powi(2) < 1.0,
                    // A thin straight line, 2 to 4 px wide.
                    _ => (dx * angle.sin() - dy * angle.cos()).abs() < 1.0 + rx / 70.0 && dx.hypot(dy) < 3.0 * rx,
                };
                if inside {
                    let i = (y * W + x) * 3;
                    for c in 0..3 {
                        rgb[i + c] = colour[c] as u8;
                    }
                }
            }
        }
    }
    let kind = seed % 4;
    let (sx0, sx1) = ((rng.uniform() * 0.4 * W as f64) as usize, (0.5 + rng.uniform() * 0.5) * W as f64);
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) * 3;
            let mut add = rng.gauss() * 4.0;
            match kind {
                1 => {
                    if x >= sx0 && (x as f64) < sx1 {
                        for c in 0..3 {
                            rgb[i + c] = (rgb[i + c] as f64 * 0.15 + 12.0) as u8;
                        }
                    }
                }
                2 => add += 18.0 * ((x as f64 * 0.45 + y as f64 * 0.12).sin()),
                3 => add += if rng.uniform() < 0.08 { -45.0 } else { 0.0 },
                _ => {}
            }
            for c in 0..3 {
                rgb[i + c] = (rgb[i + c] as f64 + add).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    RgbImage { width: W as u32, height: H as u32, rgb }
}

fn stamp(img: &RgbImage, cover: &[f32]) -> RgbImage {
    let mut rgb = img.rgb.clone();
    for (i, &k) in cover.iter().enumerate() {
        let a = k * OPACITY;
        for c in 0..3 {
            rgb[i * 3 + c] = ((1.0 - a) * rgb[i * 3 + c] as f32 + a * 255.0).round() as u8;
        }
    }
    RgbImage { width: img.width, height: img.height, rgb }
}

/// Keep colour at half resolution (4:2:0), as camera and export JPEGs do; the `image` encoder
/// does not, so it is done here before encoding.
fn halve_colour(img: &RgbImage) -> RgbImage {
    let (w, h) = (img.width as usize, img.height as usize);
    let ycc: Vec<[f32; 3]> = img
        .rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            [y, -0.168_736 * r - 0.331_264 * g + 0.5 * b, 0.5 * r - 0.418_688 * g - 0.081_312 * b]
        })
        .collect();
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let (bx, by) = (x / 2 * 2, y / 2 * 2);
            let (mut cb, mut cr, mut n) = (0f32, 0f32, 0f32);
            for yy in by..(by + 2).min(h) {
                for xx in bx..(bx + 2).min(w) {
                    cb += ycc[yy * w + xx][1];
                    cr += ycc[yy * w + xx][2];
                    n += 1.0;
                }
            }
            let (l, cb, cr) = (ycc[y * w + x][0], cb / n, cr / n);
            let px = [l + 1.402 * cr, l - 0.344_136 * cb - 0.714_136 * cr, l + 1.772 * cb];
            for c in 0..3 {
                rgb[(y * w + x) * 3 + c] = px[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    RgbImage { width: img.width, height: img.height, rgb }
}

fn save_jpeg(img: &RgbImage, path: &Path, quality: u8) {
    std::fs::write(path, common::encode_jpeg(img, quality)).expect("write jpeg");
}

fn load(path: &Path) -> RgbImage {
    let i = image::open(path).expect("open result").to_rgb8();
    RgbImage { width: i.width(), height: i.height(), rgb: i.into_raw() }
}

fn psnr(a: &RgbImage, b: &RgbImage) -> f64 {
    let mse = a.rgb.iter().zip(&b.rgb).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum::<f64>() / a.rgb.len() as f64;
    10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
}

fn luma_diff(result: &RgbImage, clean: &RgbImage) -> Vec<f64> {
    (0..W * H).map(|i| (0..3).map(|c| result.rgb[i * 3 + c] as f64 - clean.rgb[i * 3 + c] as f64).sum::<f64>() / 3.0).collect()
}

/// Mean gradient magnitude of (result - clean), pixel by pixel, over the mark's border. Counts
/// a leftover outline, but also any texture that differs from the original's exact pixels.
fn outline_pixelwise(result: &RgbImage, clean: &RgbImage, border: &[bool]) -> f64 {
    let diff = luma_diff(result, clean);
    let (mut sum, mut n) = (0.0, 0usize);
    for y in 1..H - 1 {
        for x in 1..W - 1 {
            let i = y * W + x;
            if border[i] {
                let gx = (diff[i + 1] - diff[i - 1]) / 2.0;
                let gy = (diff[i + W] - diff[i - W]) / 2.0;
                sum += gx.hypot(gy);
                n += 1;
            }
        }
    }
    sum / n.max(1) as f64
}

/// The outline score: (result - clean) is read across the mark's border at every edge point
/// (from 8 px inside to 8 px outside, along the edge's normal) and averaged over all points, so
/// an outline that follows the mark adds up while texture that merely differs cancels out; the
/// score is the mean gradient magnitude of that averaged cross-border profile (grey levels per px).
fn outline(result: &RgbImage, clean: &RgbImage, pts: &[[f64; 4]]) -> f64 {
    let diff = luma_diff(result, clean);
    let at = |x: f64, y: f64| -> f64 {
        let (xi, yi) = (x.round().clamp(0.0, (W - 1) as f64) as usize, y.round().clamp(0.0, (H - 1) as f64) as usize);
        diff[yi * W + xi]
    };
    let profile: Vec<f64> = (-8..=8)
        .map(|d| pts.iter().map(|p| at(p[0] + p[2] * d as f64, p[1] + p[3] * d as f64)).sum::<f64>() / pts.len().max(1) as f64)
        .collect();
    profile.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (profile.len() - 1) as f64
}

/// Points on the mark's edge (where its cover crosses one half) with the outward normal.
fn edge_points(cover: &[f32]) -> Vec<[f64; 4]> {
    let mut v = Vec::new();
    for y in 1..H - 1 {
        for x in 1..W - 1 {
            let i = y * W + x;
            let (gx, gy) = ((cover[i + 1] - cover[i - 1]) as f64, (cover[i + W] - cover[i - W]) as f64);
            let g = gx.hypot(gy);
            if g > 0.2 && (cover[i] - 0.5).abs() < 0.5 {
                // Cover falls going out of the mark, so outward is down the gradient.
                v.push([x as f64, y as f64, -gx / g, -gy / g]);
            }
        }
    }
    v
}

fn psnr_in(a: &RgbImage, b: &RgbImage, region: &[bool]) -> f64 {
    let (mut se, mut n) = (0.0, 0usize);
    for (i, _) in region.iter().enumerate().filter(|(_, &r)| r) {
        for c in 0..3 {
            se += (a.rgb[i * 3 + c] as f64 - b.rgb[i * 3 + c] as f64).powi(2);
            n += 1;
        }
    }
    10.0 * (255.0f64 * 255.0 / (se / n.max(1) as f64).max(1e-9)).log10()
}

/// The mark's border: pixels it partly covers, or that touch both covered and uncovered ones,
/// widened by 3 px (where JPEG smears it).
fn border(cover: &[f32]) -> Vec<bool> {
    let edge: Vec<bool> = (0..W * H)
        .map(|i| {
            let (x, y) = (i % W, i / W);
            let k = cover[i];
            if k > 0.0 && k < 1.0 {
                return true;
            }
            let on = k >= 0.5;
            [(x > 0, i.wrapping_sub(1)), (x + 1 < W, i + 1), (y > 0, i.wrapping_sub(W)), (y + 1 < H, i + W)]
                .iter()
                .any(|&(ok, j)| ok && (cover[j] >= 0.5) != on)
        })
        .collect();
    let mut out = edge.clone();
    for y in 0..H {
        for x in 0..W {
            if edge[y * W + x] {
                for yy in y.saturating_sub(3)..(y + 4).min(H) {
                    for xx in x.saturating_sub(3)..(x + 4).min(W) {
                        out[yy * W + xx] = true;
                    }
                }
            }
        }
    }
    out
}

struct Scores {
    psnr: f64,
    psnr_border: f64,
    outline: f64,
    pixelwise: f64,
}

fn score(dir: &Path, names: &[String], cleans: &[RgbImage], border: &[bool], pts: &[[f64; 4]]) -> Scores {
    let (mut p, mut pb, mut o, mut px) = (0.0, 0.0, 0.0, 0.0);
    for (name, clean) in names.iter().zip(cleans) {
        let r = load(&dir.join(format!("{name}.jpg")));
        p += psnr(&r, clean);
        pb += psnr_in(&r, clean, border);
        o += outline(&r, clean, pts);
        px += outline_pixelwise(&r, clean, border);
    }
    let n = names.len() as f64;
    Scores { psnr: p / n, psnr_border: pb / n, outline: o / n, pixelwise: px / n }
}

impl std::fmt::Display for Scores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PSNR {:.2} dB (border {:.2} dB), outline score {:.3} (pixel-wise), {:.3} (coherent profile)",
            self.psnr, self.psnr_border, self.pixelwise, self.outline
        )
    }
}

#[test]
fn removes_a_learned_watermark_and_ai_cleans_its_outline() {
    let tmp = common::scratch("unmark");
    let input = tmp.join("previews");
    std::fs::create_dir_all(&input).expect("mkdir");
    let cover = mark_cover();
    let border = border(&cover);
    let pts = edge_points(&cover);
    let mut cleans = Vec::new();
    let mut names = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut marked_psnr = 0.0;
    for seed in 0..12u64 {
        let clean = photo(seed + 1);
        let marked = stamp(&clean, &cover);
        let name = format!("preview_{seed:02}");
        let p = input.join(format!("{name}.jpg"));
        save_jpeg(&halve_colour(&marked), &p, 85);
        marked_psnr += psnr(&load(&p), &clean);
        paths.push(p);
        // Kept beside the results for a look when a check fails.
        save_jpeg(&clean, &tmp.join(format!("{name}_clean.jpg")), 100);
        cleans.push(clean);
        names.push(name);
    }
    marked_psnr /= names.len() as f64;
    let stop = AtomicBool::new(false);
    let stage = AtomicUsize::new(0);

    let classical_dir = tmp.join("classical");
    let (mark, s) = unmark::unmark_all(&paths, &classical_dir, None, &stop, &stage, |_, _| {}).expect("unmark");
    assert_eq!(s.written, names.len(), "every preview cleaned: {:?}", s.errors);
    assert!(s.ai.is_empty());
    assert!((mark.alpha[0] - OPACITY).abs() < 0.08, "opacity learned as {:?}", mark.alpha);
    let classical = score(&classical_dir, &names, &cleans, &border, &pts);
    println!("watermarked:     PSNR {marked_psnr:.2} dB");
    println!("classical edges: {classical}");
    assert!(classical.psnr > marked_psnr + 3.0, "removing the mark helps: {:.2} -> {:.2}", marked_psnr, classical.psnr);

    if inpaint::find_model().is_none() {
        println!("SKIPPED the AI part: {} is not installed (run scripts\\fetch-lama.ps1).", inpaint::MODEL_FILE);
        return;
    }
    let lama = inpaint::shared().expect("load the AI model").expect("model present");
    let ai_dir = tmp.join("ai");
    let (_, s) = unmark::unmark_all(&paths, &ai_dir, Some(lama), &stop, &stage, |_, _| {}).expect("unmark with AI");
    assert_eq!(s.written, names.len(), "every preview cleaned: {:?}", s.errors);
    let used = s.ai.iter().filter(|(_, a)| *a == inpaint::AiEdge::Used).count();
    let ai = score(&ai_dir, &names, &cleans, &border, &pts);
    println!("AI edges:        {ai} ({used} of {} used the AI result)", s.ai.len());
    for (name, a) in &s.ai {
        if let inpaint::AiEdge::FellBack(why) = a {
            println!("  {name}: {why}");
        }
    }
    assert!(
        ai.psnr_border > classical.psnr_border,
        "the AI brings the border closer to the clean photo: {:.2} vs {:.2} dB",
        ai.psnr_border,
        classical.psnr_border
    );
    assert!(ai.psnr >= classical.psnr - 0.25, "the AI costs at most 0.25 dB overall: {:.2} vs {:.2} dB", ai.psnr, classical.psnr);
}
