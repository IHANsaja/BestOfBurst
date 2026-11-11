//! Watermark remover: rebuilds what a watermark hides in a preview from the photographer's own
//! original of the same frame.
//!
//! The preview keeps its look (its edit, size and crop). The original is brought into the
//! preview's frame, given the preview's tones (a per-channel tone curve learned from the two
//! pictures, plus a smooth local correction), and used only where the two disagree: under the
//! watermark. Everywhere else the preview's own pixels stay. Because it needs the original, it
//! works for whoever owns the photos, and the result is the real picture, not a guess.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{bail, Context, Result};
use rayon::prelude::*;

use crate::decode::{self, RgbImage};
use crate::find::{Match, Verdict};
use crate::scan;

/// Long edge of the copies the mask and the tone match are worked out on.
const WORK_EDGE: u32 = 1400;
/// More than this share of the frame differing is another edit or another photo, not a mark.
pub const MAX_COVERAGE: f32 = 0.55;
/// The folder cleaned previews go to, inside the shoot's `_bestofburst`.
pub const CLEAN_DIR: &str = "clean previews";

pub struct Cleaned {
    pub image: RgbImage,
    /// Share of the frame that was rebuilt, 0..1.
    pub coverage: f32,
}

/// A plane of numbers the size of the working copy.
struct Plane {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Plane {
    /// Mean over a (2r+1) square around every cell (edges use what is there).
    fn blur(&self, r: usize) -> Plane {
        let (w, h) = (self.w, self.h);
        let mut sums = vec![0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0f64;
            for x in 0..w {
                row += self.v[y * w + x] as f64;
                sums[(y + 1) * (w + 1) + x + 1] = sums[y * (w + 1) + x + 1] + row;
            }
        }
        let mut v = vec![0f32; w * h];
        for y in 0..h {
            let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
            for x in 0..w {
                let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
                let s = sums[y1 * (w + 1) + x1] - sums[y0 * (w + 1) + x1] - sums[y1 * (w + 1) + x0] + sums[y0 * (w + 1) + x0];
                v[y * w + x] = (s / ((y1 - y0) * (x1 - x0)) as f64) as f32;
            }
        }
        Plane { w, h, v }
    }

    /// Bilinear read at a position given in pixels of a `fw` x `fh` image.
    #[inline]
    fn at(&self, x: usize, y: usize, fw: usize, fh: usize) -> f32 {
        let fx = ((x as f32 + 0.5) * self.w as f32 / fw as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = ((y as f32 + 0.5) * self.h as f32 / fh as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let p = |xx: usize, yy: usize| self.v[yy * self.w + xx];
        (p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx) * (1.0 - ty) + (p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx) * ty
    }
}

/// For each channel, the preview value that an original value usually becomes (the median, so
/// a watermark over part of the frame does not bend the curve).
fn tone_curves(original: &RgbImage, preview: &RgbImage) -> [[u8; 256]; 3] {
    let n = (original.width * original.height) as usize;
    let mut curves = [[0u8; 256]; 3];
    for c in 0..3 {
        let mut hist = vec![0u32; 256 * 256];
        for i in 0..n {
            hist[original.rgb[i * 3 + c] as usize * 256 + preview.rgb[i * 3 + c] as usize] += 1;
        }
        let counts: Vec<u32> = (0..256).map(|o| hist[o * 256..o * 256 + 256].iter().sum()).collect();
        let med: Vec<Option<f32>> = (0..256)
            .map(|o| {
                let row = &hist[o * 256..o * 256 + 256];
                if counts[o] < 12 {
                    return None;
                }
                let mut seen = 0;
                row.iter()
                    .position(|&k| {
                        seen += k;
                        seen * 2 >= counts[o]
                    })
                    .map(|p| p as f32)
            })
            .collect();
        if med.iter().all(|m| m.is_none()) {
            for (o, v) in curves[c].iter_mut().enumerate() {
                *v = o as u8;
            }
            continue;
        }
        // A tone curve never turns back. Make it so by pooling neighbours that do, weighted by
        // how many pixels speak for each value: a few stray pixels (dark ones under a white
        // mark) cannot drag the curve with them.
        let mut blocks: Vec<(f64, f64, usize)> = Vec::new(); // value, weight, values covered
        for o in 0..256usize {
            let (v, wgt) = match med[o] {
                Some(v) => (v as f64, counts[o] as f64),
                None => (blocks.last().map_or(o as f64, |b| b.0), 0.0),
            };
            blocks.push((v, wgt, 1));
            while blocks.len() >= 2 && blocks[blocks.len() - 2].0 > blocks[blocks.len() - 1].0 {
                let (b, a) = (blocks.pop().unwrap_or((0.0, 0.0, 0)), blocks.pop().unwrap_or((0.0, 0.0, 0)));
                let wsum = a.1 + b.1;
                let v = if wsum > 0.0 { (a.0 * a.1 + b.0 * b.1) / wsum } else { a.0.min(b.0) };
                blocks.push((v, wsum, a.2 + b.2));
            }
        }
        let flat: Vec<f64> = blocks.iter().flat_map(|b| std::iter::repeat(b.0).take(b.2)).collect();
        // Values the photo never uses before its first known one start from that one.
        let first = (0..256).find(|&o| med[o].is_some()).unwrap_or(0);
        for o in 0..256usize {
            let (a, b) = (o.saturating_sub(3).max(first.min(o)), (o + 4).min(256));
            let mean = (a..b).map(|k| flat[k.max(first)]).sum::<f64>() / (b - a) as f64;
            curves[c][o] = mean.round().clamp(0.0, 255.0) as u8;
        }
    }
    curves
}

/// Turn clockwise by quarter turns.
fn turned(img: RgbImage, turns: u8) -> RgbImage {
    match turns % 4 {
        1 => decode::apply_orientation(img, 6),
        2 => decode::apply_orientation(img, 3),
        3 => decode::apply_orientation(img, 8),
        _ => img,
    }
}

/// The original as the preview frames it: turned, cropped to `crop` and sized like the preview.
fn framed(original: &RgbImage, turns: u8, crop: [f32; 4], w: u32, h: u32) -> Result<RgbImage> {
    let o = turned(original.clone(), turns);
    let (x0, y0) = ((crop[0] * o.width as f32).round() as u32, (crop[1] * o.height as f32).round() as u32);
    let (x1, y1) = (((crop[2] * o.width as f32).round() as u32).min(o.width), ((crop[3] * o.height as f32).round() as u32).min(o.height));
    if x1 <= x0 + 8 || y1 <= y0 + 8 {
        bail!("the crop is empty");
    }
    let cut = if (x0, y0, x1, y1) == (0, 0, o.width, o.height) {
        o
    } else {
        let mut rgb = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 3) as usize);
        for y in y0..y1 {
            let s = ((y * o.width + x0) * 3) as usize;
            rgb.extend_from_slice(&o.rgb[s..s + ((x1 - x0) * 3) as usize]);
        }
        RgbImage { width: x1 - x0, height: y1 - y0, rgb }
    };
    decode::resize_exact(cut, w, h)
}

/// Remove the watermark from `sent` using its `original`. `turns` and `crop` are how the
/// original lines up with it (from `find`): quarter turns clockwise, then the part shown.
pub fn remove_watermark(sent: &RgbImage, original: RgbImage, turns: u8, crop: [f32; 4]) -> Result<Cleaned> {
    let first = attempt(sent, &original, turns, crop);
    if crop == crate::find::FULL_FRAME || first.as_ref().is_ok_and(|c| c.coverage < 0.25) {
        return first;
    }
    // The crop the matcher lined up may be a hair off, which shows as differences everywhere.
    // The whole frame is the other reading: keep whichever leaves less to rebuild.
    match (first, attempt(sent, &original, turns, crate::find::FULL_FRAME)) {
        (Ok(a), Ok(b)) => Ok(if a.coverage <= b.coverage { a } else { b }),
        (Ok(a), Err(_)) => Ok(a),
        (Err(_), Ok(b)) => Ok(b),
        (Err(e), Err(_)) => Err(e),
    }
}

fn attempt(sent: &RgbImage, original: &RgbImage, turns: u8, crop: [f32; 4]) -> Result<Cleaned> {
    let (w, h) = (sent.width as usize, sent.height as usize);
    let orig = framed(original, turns, crop, sent.width, sent.height)?;

    // Work out tones, the mask and the local correction on small copies.
    let ps = decode::resize_ref(sent, WORK_EDGE)?;
    let os = decode::resize_exact(orig.clone(), ps.width, ps.height)?;
    let curves = tone_curves(&os, &ps);
    let (sw, sh) = (ps.width as usize, ps.height as usize);
    let toned = |c: usize, v: u8| curves[c][v as usize] as f32;

    // Where the preview and the toned original disagree.
    let mut diff = Plane { w: sw, h: sh, v: vec![0.0; sw * sh] };
    for i in 0..sw * sh {
        let mut d = 0f32;
        for c in 0..3 {
            d = d.max((ps.rgb[i * 3 + c] as f32 - toned(c, os.rgb[i * 3 + c])).abs());
        }
        diff.v[i] = d;
    }
    let long = sw.max(sh);
    let diff = diff.blur((long / 500).max(1));
    let mut sorted = diff.v.clone();
    let mid = sorted.len() / 2;
    sorted.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    let limit = (2.5 * sorted[mid]).clamp(9.0, 24.0);
    let raw = Plane { w: sw, h: sh, v: diff.v.iter().map(|&d| (d > limit) as u8 as f32).collect() };
    // Grow the mark a little (its soft edge), then feather it.
    let grow = (long / 220).max(2);
    let grown = Plane { w: sw, h: sh, v: raw.blur(grow).v.iter().map(|&m| (m > 0.04) as u8 as f32).collect() };
    let coverage = grown.v.iter().sum::<f32>() / (sw * sh) as f32;
    if coverage > MAX_COVERAGE {
        bail!("this photo differs from its original over {:.0}% of the frame: that is another edit, not a watermark", coverage * 100.0);
    }
    let alpha = grown.blur((grow / 2).max(1));

    // The smooth part of what the tone curves miss (a vignette, a local adjustment), read
    // from around the mark and carried under it.
    let clear = Plane { w: sw, h: sh, v: grown.v.iter().map(|&m| 1.0 - m).collect() };
    let reach = (long / 28).max(4);
    let weight = clear.blur(reach);
    let mut bias: Vec<Plane> = Vec::with_capacity(3);
    for c in 0..3 {
        let mut p = Plane { w: sw, h: sh, v: vec![0.0; sw * sh] };
        for i in 0..sw * sh {
            p.v[i] = (ps.rgb[i * 3 + c] as f32 - toned(c, os.rgb[i * 3 + c])) * clear.v[i];
        }
        let mut b = p.blur(reach);
        for i in 0..sw * sh {
            b.v[i] = if weight.v[i] > 0.03 { (b.v[i] / weight.v[i]).clamp(-40.0, 40.0) } else { 0.0 };
        }
        bias.push(b);
    }

    // Put it together at full size: the preview's own pixels, the original under the mark.
    let mut out = sent.rgb[..w * h * 3].to_vec();
    out.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let a = alpha.at(x, y, w, h);
            if a <= 0.004 {
                continue;
            }
            let i = (y * w + x) * 3;
            for c in 0..3 {
                let fill = toned(c, orig.rgb[i + c]) + bias[c].at(x, y, w, h);
                let v = a * fill + (1.0 - a) * row[x * 3 + c] as f32;
                row[x * 3 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    });
    Ok(Cleaned { image: RgbImage { width: sent.width, height: sent.height, rgb: out }, coverage })
}

fn open(path: &Path) -> Result<RgbImage> {
    let format = scan::detect_file(path)
        .with_context(|| format!("cannot read {}", path.display()))?
        .context("not a photo BestOfBurst can open")?;
    Ok(decode::decode_full(path, format)?.image)
}

/// Save as a high-quality JPEG (written beside, then renamed into place).
pub fn save_jpeg(img: &RgbImage, dest: &Path) -> Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = dest.with_extension("jpg.tmp");
    {
        let file = std::fs::File::create(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
        let mut w = std::io::BufWriter::new(file);
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 93);
        enc.encode(&img.rgb, img.width, img.height, image::ExtendedColorType::Rgb8)?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct CleanSummary {
    pub dir: PathBuf,
    pub written: usize,
    /// Sent photos with no sure original: nothing was made for them.
    pub unmatched: usize,
    /// Sent file and why it could not be cleaned.
    pub errors: Vec<(String, String)>,
    pub stopped: bool,
}

/// Remove the watermark from every sent photo whose original is sure (`Found`), writing
/// `<name>.jpg` into `dir`. Sent photos and originals are only read. `input` is the folder the
/// originals' paths are relative to.
pub fn clean_all(input: &Path, matches: &[Match], dir: &Path, stop: &AtomicBool, progress: impl Fn(usize, usize) + Sync) -> Result<CleanSummary> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let todo: Vec<&Match> = matches.iter().filter(|m| m.verdict == Verdict::Found && m.best.is_some()).collect();
    let done = AtomicUsize::new(0);
    // Two at a time: each holds two full-size photos in memory.
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().context("cannot start worker threads")?;
    let results: Vec<Option<(String, Result<()>)>> = pool.install(|| {
        todo.par_iter()
            .map(|m| {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                let name = m.query.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let best = m.best.as_ref().context("no original")?;
                    let sent = open(&m.query)?;
                    let original = open(&input.join(&best.path))?;
                    let cleaned = remove_watermark(&sent, original, best.turns, best.crop)?;
                    let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "photo".into());
                    save_jpeg(&cleaned.image, &dir.join(format!("{stem}.jpg")))
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the photo could not be processed")));
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, todo.len());
                Some((name, r))
            })
            .collect()
    });
    let mut s = CleanSummary { dir: dir.to_path_buf(), unmatched: matches.len() - todo.len(), stopped: stop.load(Ordering::Relaxed), ..Default::default() };
    for (name, r) in results.into_iter().flatten() {
        match r {
            Ok(()) => s.written += 1,
            Err(e) => s.errors.push((name, format!("{e:#}"))),
        }
    }
    Ok(s)
}
