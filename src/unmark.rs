//! Watermark remover that needs no originals: it learns the watermark from the watermarked
//! photos themselves.
//!
//! A preview service stamps the same mark on every photo it sends (a word or a logo, light and
//! partly see-through). Over many different photos the only edges that stay put are the mark's,
//! so the median of the photos' edges is the mark's outline. Those edges are read in a space
//! where a light see-through mark is a step of the same height whatever is behind it
//! (`ln(L - value)`), so adding them up (a Poisson solve) gives the mark's opacity everywhere.
//! The mark's exact opacity and colour then come from the photos (just inside the mark against
//! just outside it), and each photo is un-blended: `real = (seen - a * colour) / (1 - a)`.
//! The few pixels along the mark's anti-aliased edge, which no estimate gets exactly right,
//! are filled smoothly from both sides.
//!
//! The mark is assumed to be scaled with the photo's width and centred (how preview services
//! and export presets place it), so portrait and landscape photos of one set share it.
//! Photos where the learned mark is not found are left alone.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{bail, Context, Result};
use rayon::prelude::*;

use crate::clean::{save_jpeg, CleanSummary};
use crate::decode::{self, RgbImage};
use crate::scan;

/// Width of the frame the mark is learned in.
const WORK_WIDTH: u32 = 2000;
/// At most this many photos are read to learn the mark.
const MAX_SAMPLES: usize = 40;
/// Fewer photos than this cannot tell the mark from the pictures.
pub const MIN_PHOTOS: usize = 3;
/// Photos read again in colour to measure the mark's opacity and colour.
const COLOUR_SAMPLES: usize = 10;
/// The light level a light mark is measured against (a little above white, so white stays finite).
const LIGHT: f32 = 262.0;
/// Values this bright (in any channel) say nothing about a light mark.
const SATURATED: u8 = 250;

#[inline]
fn lum(r: u8, g: u8, b: u8) -> f32 {
    (r as f32 + g as f32 + b as f32) / 3.0
}

#[inline]
fn lnl(v: f32) -> f32 {
    (LIGHT - v).max(3.0).ln()
}

/// The learned watermark.
pub struct Mark {
    /// Frame size: every photo is brought to this width and centred in this height.
    fw: u32,
    fh: u32,
    /// How much of the mark covers each frame pixel, 0..1.
    cover: Vec<f32>,
    /// Opacity and colour per channel.
    pub alpha: [f32; 3],
    pub colour: [f32; 3],
    /// Frame points just inside and just outside the mark's edge, to check a photo carries it.
    pairs: Vec<[f32; 4]>,
    /// Frame points on the mark's edge: x, y, the way out of the mark (unit), edge strength.
    edges: Vec<[f32; 5]>,
    /// Share of the frame the mark covers.
    pub coverage: f32,
    /// Photos it was learned from.
    pub learned_from: usize,
}

/// One photo as the learner keeps it: grey, brought to the frame width (255 = too bright to use).
struct Sample {
    h: u32,
    grey: Vec<u8>,
}

fn open(path: &Path) -> Result<RgbImage> {
    let format = scan::detect_file(path)
        .with_context(|| format!("cannot read {}", path.display()))?
        .context("not a photo BestOfBurst can open")?;
    Ok(decode::decode_full(path, format)?.image)
}

fn placed_height(fw: u32, iw: u32, ih: u32) -> u32 {
    ((ih as f64 * fw as f64 / iw as f64).round() as u32).max(1)
}

fn to_frame(img: RgbImage, fw: u32) -> Result<RgbImage> {
    let h = placed_height(fw, img.width, img.height);
    decode::resize_exact(img, fw, h)
}

fn evenly(paths: &[PathBuf], n: usize) -> Vec<PathBuf> {
    if paths.len() <= n {
        return paths.to_vec();
    }
    (0..n).map(|i| paths[i * paths.len() / n].clone()).collect()
}

/// Binary erosion (`grow` false) or dilation with a 3x3 square.
fn morph(m: &[bool], w: usize, h: usize, grow: bool) -> Vec<bool> {
    let mut out = vec![false; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut any = false;
            let mut all = true;
            for yy in y.saturating_sub(1)..(y + 2).min(h) {
                for xx in x.saturating_sub(1)..(x + 2).min(w) {
                    let v = m[yy * w + xx];
                    any |= v;
                    all &= v;
                }
            }
            row[x] = if grow { any } else { all };
        }
    });
    out
}

pub(crate) fn morph_n(m: &[bool], w: usize, h: usize, grow: bool, n: usize) -> Vec<bool> {
    let mut v = m.to_vec();
    for _ in 0..n {
        v = morph(&v, w, h, grow);
    }
    v
}

/// A smooth surface through `u` where `skip` is false (16 px blocks, holes filled from their
/// neighbours, smoothed, read back bilinearly).
fn background(u: &[f32], skip: &[bool], w: usize, h: usize) -> Vec<f32> {
    const B: usize = 16;
    let (bw, bh) = (w.div_ceil(B), h.div_ceil(B));
    let mut sum = vec![0f64; bw * bh];
    let mut n = vec![0usize; bw * bh];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !skip[i] {
                let j = (y / B) * bw + x / B;
                sum[j] += u[i] as f64;
                n[j] += 1;
            }
        }
    }
    let mut v: Vec<Option<f32>> = (0..bw * bh).map(|j| (n[j] * 10 >= B * B * 3).then(|| (sum[j] / n[j] as f64) as f32)).collect();
    if v.iter().all(|b| b.is_none()) {
        return vec![0.0; w * h];
    }
    // Fill the blocks the mark covers from the outside in.
    while v.iter().any(|b| b.is_none()) {
        let prev = v.clone();
        for by in 0..bh {
            for bx in 0..bw {
                if prev[by * bw + bx].is_some() {
                    continue;
                }
                let (mut s, mut c) = (0f32, 0f32);
                for yy in by.saturating_sub(1)..(by + 2).min(bh) {
                    for xx in bx.saturating_sub(1)..(bx + 2).min(bw) {
                        if let Some(q) = prev[yy * bw + xx] {
                            s += q;
                            c += 1.0;
                        }
                    }
                }
                if c > 0.0 {
                    v[by * bw + bx] = Some(s / c);
                }
            }
        }
    }
    let mut g: Vec<f32> = v.into_iter().map(|b| b.unwrap_or(0.0)).collect();
    for _ in 0..2 {
        let prev = g.clone();
        for by in 0..bh {
            for bx in 0..bw {
                let (mut s, mut c) = (0f32, 0f32);
                for yy in by.saturating_sub(1)..(by + 2).min(bh) {
                    for xx in bx.saturating_sub(1)..(bx + 2).min(bw) {
                        s += prev[yy * bw + xx];
                        c += 1.0;
                    }
                }
                g[by * bw + bx] = s / c;
            }
        }
    }
    let mut out = vec![0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) / B as f32 - 0.5).clamp(0.0, (bh - 1) as f32);
        let (y0, ty) = (fy as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(bh - 1);
        for (x, o) in row.iter_mut().enumerate() {
            let fx = ((x as f32 + 0.5) / B as f32 - 0.5).clamp(0.0, (bw - 1) as f32);
            let (x0, tx) = (fx as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(bw - 1);
            let a = g[y0 * bw + x0] * (1.0 - tx) + g[y0 * bw + x1] * tx;
            let b = g[y1 * bw + x0] * (1.0 - tx) + g[y1 * bw + x1] * tx;
            *o = a * (1.0 - ty) + b * ty;
        }
    });
    out
}

struct Region {
    area: usize,
    /// Mean edge strength along its rim.
    support: f32,
    touches_frame: bool,
}

/// 4-connected regions of `m` (labels from 1; 0 is outside `m`).
fn regions(m: &[bool], w: usize, h: usize, mag: &[f32]) -> (Vec<u32>, Vec<Region>) {
    let mut label = vec![0u32; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !m[start] || label[start] != 0 {
            continue;
        }
        let id = out.len() as u32 + 1;
        label[start] = id;
        stack.push(start);
        let (mut area, mut edge_n, mut edge_sum, mut touches) = (0usize, 0usize, 0f64, false);
        while let Some(i) = stack.pop() {
            area += 1;
            let (x, y) = (i % w, i / w);
            touches |= x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            let mut rim = false;
            for (ok, j) in [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)] {
                if !ok {
                    continue;
                }
                if m[j] {
                    if label[j] == 0 {
                        label[j] = id;
                        stack.push(j);
                    }
                } else {
                    rim = true;
                }
            }
            if rim {
                edge_n += 1;
                edge_sum += mag[i] as f64;
            }
        }
        let support = if edge_n > 0 { (edge_sum / edge_n as f64) as f32 } else { 0.0 };
        out.push(Region { area, support, touches_frame: touches });
    }
    (label, out)
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    v.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    v[mid]
}

/// A value at a fraction of the way through `v` (sorted order), from a thinned copy.
fn percentile(v: &[f32], q: f32) -> f32 {
    let step = (v.len() / 400_000).max(1);
    let mut s: Vec<f32> = v.iter().step_by(step).copied().collect();
    if s.is_empty() {
        return 0.0;
    }
    let k = ((s.len() - 1) as f32 * q) as usize;
    s.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    s[k]
}

// ---- Poisson solve on the pixel grid (Neumann edges), by multigrid ----

/// One red-black Gauss-Seidel sweep pair of sum over neighbours (u_q - u_p) = rhs_p.
fn smooth(u: &mut [f32], rhs: &[f32], w: usize, h: usize, sweeps: usize) {
    for _ in 0..sweeps {
        for colour in 0..2 {
            for y in 0..h {
                let start = (y + colour) % 2;
                for x in (start..w).step_by(2) {
                    let i = y * w + x;
                    let (mut s, mut d) = (0f32, 0f32);
                    if x > 0 {
                        s += u[i - 1];
                        d += 1.0;
                    }
                    if x + 1 < w {
                        s += u[i + 1];
                        d += 1.0;
                    }
                    if y > 0 {
                        s += u[i - w];
                        d += 1.0;
                    }
                    if y + 1 < h {
                        s += u[i + w];
                        d += 1.0;
                    }
                    if d > 0.0 {
                        u[i] = (s - rhs[i]) / d;
                    }
                }
            }
        }
    }
}

fn residual(u: &[f32], rhs: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut r = vec![0f32; w * h];
    r.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let i = y * w + x;
            let mut l = 0f32;
            if x > 0 {
                l += u[i - 1] - u[i];
            }
            if x + 1 < w {
                l += u[i + 1] - u[i];
            }
            if y > 0 {
                l += u[i - w] - u[i];
            }
            if y + 1 < h {
                l += u[i + w] - u[i];
            }
            row[x] = rhs[i] - l;
        }
    });
    r
}

fn vcycle(u: &mut [f32], rhs: &[f32], w: usize, h: usize) {
    if w * h <= 256 || w < 4 || h < 4 {
        smooth(u, rhs, w, h, 300);
        return;
    }
    smooth(u, rhs, w, h, 2);
    let r = residual(u, rhs, w, h);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut rc = vec![0f32; cw * ch];
    for y in 0..h {
        for x in 0..w {
            rc[(y / 2) * cw + x / 2] += r[y * w + x];
        }
    }
    let mut e = vec![0f32; cw * ch];
    vcycle(&mut e, &rc, cw, ch);
    // Bilinear prolongation of the correction.
    u.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (ch - 1) as f32);
        let (y0, ty) = (fy as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(ch - 1);
        for (x, v) in row.iter_mut().enumerate() {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (cw - 1) as f32);
            let (x0, tx) = (fx as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(cw - 1);
            let a = e[y0 * cw + x0] * (1.0 - tx) + e[y0 * cw + x1] * tx;
            let b = e[y1 * cw + x0] * (1.0 - tx) + e[y1 * cw + x1] * tx;
            *v += a * (1.0 - ty) + b * ty;
        }
    });
    smooth(u, rhs, w, h, 2);
}

/// The field whose differences best match `gx` (to the right) and `gy` (downwards).
fn integrate(gx: &[f32], gy: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut rhs = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x + 1 < w {
                rhs[i] += gx[i];
                rhs[i + 1] -= gx[i];
            }
            if y + 1 < h {
                rhs[i] += gy[i];
                rhs[i + w] -= gy[i];
            }
        }
    }
    let mut u = vec![0f32; w * h];
    for _ in 0..30 {
        vcycle(&mut u, &rhs, w, h);
    }
    u
}

/// Robust straight-line fit `y = k x + b` (iteratively reweighted least squares).
fn robust_line(xs: &[f32], ys: &[f32]) -> Option<(f32, f32)> {
    if xs.len() < 50 {
        return None;
    }
    let mut wts = vec![1f64; xs.len()];
    let (mut k, mut b) = (1f64, 0f64);
    for _ in 0..8 {
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64);
        for ((&x, &y), &wt) in xs.iter().zip(ys).zip(&wts) {
            let (x, y) = (x as f64, y as f64);
            sw += wt;
            sx += wt * x;
            sy += wt * y;
            sxx += wt * x * x;
            sxy += wt * x * y;
        }
        let den = sw * sxx - sx * sx;
        if den.abs() < 1e-9 {
            return None;
        }
        k = (sw * sxy - sx * sy) / den;
        b = (sy - k * sx) / sw;
        let mut res: Vec<f32> = xs.iter().zip(ys).map(|(&x, &y)| (y as f64 - (k * x as f64 + b)).abs() as f32).collect();
        let s = median(&mut res.clone()) as f64 * 1.5 + 1e-3;
        for (wt, r) in wts.iter_mut().zip(res.iter_mut()) {
            *wt = 1.0 / (*r as f64 / s).max(1.0);
        }
    }
    Some((k as f32, b as f32))
}

impl Mark {
    /// Learn the mark from watermarked photos (at least `MIN_PHOTOS`, different pictures).
    pub fn learn(paths: &[PathBuf], stop: &AtomicBool, progress: impl Fn(usize, usize) + Sync) -> Result<Mark> {
        if paths.len() < MIN_PHOTOS {
            bail!("it takes at least {MIN_PHOTOS} photos with the same watermark to learn it (there are {})", paths.len());
        }
        let picked = evenly(paths, MAX_SAMPLES);
        let fw = WORK_WIDTH;
        let done = AtomicUsize::new(0);
        let total = picked.len() + COLOUR_SAMPLES.min(picked.len());
        let pool = rayon::ThreadPoolBuilder::new().num_threads(3).build().context("cannot start worker threads")?;
        let samples: Vec<Sample> = pool.install(|| {
            picked
                .par_iter()
                .filter_map(|p| {
                    if stop.load(Ordering::Relaxed) {
                        return None;
                    }
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Sample> {
                        let img = to_frame(open(p)?, fw)?;
                        let grey = img
                            .rgb
                            .chunks_exact(3)
                            .map(|c| if c[0].max(c[1]).max(c[2]) >= SATURATED { 255 } else { lum(c[0], c[1], c[2]).round().min(254.0) as u8 })
                            .collect();
                        Ok(Sample { h: img.height, grey })
                    }));
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                    r.ok().and_then(|r| r.ok())
                })
                .collect()
        });
        if stop.load(Ordering::Relaxed) {
            bail!("stopped");
        }
        if samples.len() < MIN_PHOTOS {
            bail!("only {} of the photos could be opened; it takes at least {MIN_PHOTOS}", samples.len());
        }
        let fh = samples.iter().map(|s| s.h).max().unwrap_or(1);
        let (w, h) = (fw as usize, fh as usize);
        let offs: Vec<i64> = samples.iter().map(|s| (fh as i64 - s.h as i64) / 2).collect();
        let lut: Vec<f32> = (0..256).map(|v| lnl(v as f32)).collect();

        // The median edge, across the photos, of ln(L - value), right and down.
        let mut gx = vec![0f32; w * h];
        let mut gy = vec![0f32; w * h];
        gx.par_chunks_mut(w).zip(gy.par_chunks_mut(w)).enumerate().for_each(|(y, (rx, ry))| {
            let mut ax = Vec::with_capacity(samples.len());
            let mut ay = Vec::with_capacity(samples.len());
            for x in 0..w {
                ax.clear();
                ay.clear();
                for (s, &off) in samples.iter().zip(&offs) {
                    let r = y as i64 - off;
                    if r < 0 || r >= s.h as i64 {
                        continue;
                    }
                    let i = r as usize * w + x;
                    let v = s.grey[i];
                    if v == 255 {
                        continue;
                    }
                    if x + 1 < w && s.grey[i + 1] != 255 {
                        ax.push(lut[s.grey[i + 1] as usize] - lut[v as usize]);
                    }
                    if (r as usize) + 1 < s.h as usize && s.grey[i + w] != 255 {
                        ay.push(lut[s.grey[i + w] as usize] - lut[v as usize]);
                    }
                }
                rx[x] = if ax.len() >= 3 { median(&mut ax) } else { 0.0 };
                ry[x] = if ay.len() >= 3 { median(&mut ay) } else { 0.0 };
            }
        });
        let mag: Vec<f32> = gx.iter().zip(&gy).map(|(a, b)| a.hypot(*b)).collect();
        // Only clear edges are added up; the rest is noise from the pictures.
        let (kx, ky): (Vec<f32>, Vec<f32>) = gx.iter().zip(&gy).zip(&mag).map(|((&a, &b), &m)| if m > 0.03 { (a, b) } else { (0.0, 0.0) }).unzip();
        let mut u = integrate(&kx, &ky, w, h);
        let base = median(&mut u.iter().step_by(7).copied().collect::<Vec<_>>());
        for v in u.iter_mut() {
            *v -= base;
        }
        // Opacity read from the integrated step: ln(L - seen) = ln(1 - a) + ln(L - real).
        // Adding up many small edges also leaves a slow drift (shading the photos share, and
        // gaps in the mark's outline); it is taken out by levelling against what lies around
        // the mark, twice, the second time with a better idea of where the mark is.
        let level = |u: &[f32]| -> Result<(Vec<f32>, f32)> {
            let opacity: Vec<f32> = u.iter().map(|&v| 1.0 - v.exp()).collect();
            let top = percentile(&opacity, 0.997);
            if top < 0.04 {
                bail!("no watermark repeats across these photos");
            }
            let mut inside: Vec<f32> = opacity.iter().copied().filter(|&a| a > top / 2.0).collect();
            Ok((opacity, median(&mut inside)))
        };
        for _ in 0..2 {
            let (opacity, a_est) = level(&u)?;
            let marked: Vec<bool> = opacity.iter().map(|&a| a > a_est / 3.0).collect();
            let near = morph_n(&marked, w, h, true, 4);
            let b = background(&u, &near, w, h);
            for (v, b) in u.iter_mut().zip(&b) {
                *v -= b;
            }
        }
        let (opacity, a_est) = level(&u)?;
        let raw: Vec<bool> = opacity.iter().map(|&a| a > a_est / 2.0).collect();
        let opened = morph_n(&morph_n(&raw, w, h, false, 2), w, h, true, 2);

        // Keep the parts with real edges around them: a soft patch of the pictures' own shared
        // shade (a sky in many photos) has none.
        let step = -(1.0 - a_est).max(0.05).ln();
        let (label, parts) = regions(&opened, w, h, &mag);
        let keep: Vec<bool> = std::iter::once(false).chain(parts.iter().map(|r| r.area as f32 > 2e-4 * (w * h) as f32 && r.support > 0.12 * step)).collect();
        let mut mask: Vec<bool> = label.iter().map(|&l| keep[l as usize]).collect();
        // And fill gaps inside the mark that have no edge around them (where the photos say
        // too little); a letter's own hole (the inside of an O) has a clear edge.
        let gaps: Vec<bool> = mask.iter().map(|&m| !m).collect();
        let (glabel, gparts) = regions(&gaps, w, h, &mag);
        let fill: Vec<bool> = std::iter::once(false).chain(gparts.iter().map(|r| !r.touches_frame && r.area as f32 <= 0.02 * (w * h) as f32 && r.support <= 0.12 * step)).collect();
        for (m, &l) in mask.iter_mut().zip(&glabel) {
            if fill[l as usize] {
                *m = true;
            }
        }
        // Close small notches in the outline (where bright skies left the edge unread): a mark's
        // outline is smooth at this scale, and the gaps between its letters are much wider.
        let mask = morph_n(&morph_n(&mask, w, h, true, 4), w, h, false, 4);
        let covered = mask.iter().filter(|&&m| m).count();
        if covered == 0 {
            bail!("no watermark repeats across these photos");
        }

        // Pairs of points about 4 px inside and 4 px outside the mark's edge.
        let in3 = morph_n(&mask, w, h, false, 3);
        let in5 = morph_n(&in3, w, h, false, 2);
        let shell: Vec<usize> = (0..w * h).filter(|&i| in3[i] && !in5[i]).collect();
        let thin = (shell.len() / 20_000).max(1);
        let mut pairs = Vec::new();
        let m_at = |x: f32, y: f32| -> Option<bool> {
            let (xi, yi) = (x.round() as i64, y.round() as i64);
            (xi >= 0 && yi >= 0 && (xi as usize) < w && (yi as usize) < h).then(|| mask[yi as usize * w + xi as usize])
        };
        for &i in shell.iter().step_by(thin) {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            // Outward: where the mask thins out around the point.
            let (mut nx, mut ny) = (0f32, 0f32);
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    if m_at(x + dx as f32, y + dy as f32) == Some(false) {
                        nx += dx as f32;
                        ny += dy as f32;
                    }
                }
            }
            let n = nx.hypot(ny);
            if n < 1e-3 {
                continue;
            }
            let (nx, ny) = (nx / n, ny / n);
            let mut t = 1.0;
            while t < 16.0 && m_at(x + nx * t, y + ny * t) == Some(true) {
                t += 1.0;
            }
            let (ox, oy) = (x + nx * (t + 3.0), y + ny * (t + 3.0));
            if t < 16.0 && m_at(ox, oy) == Some(false) && m_at(x + nx * (t + 1.0), y + ny * (t + 1.0)) == Some(false) {
                pairs.push([x, y, ox, oy]);
            }
        }
        if pairs.len() < 50 {
            bail!("the watermark is too small to measure");
        }

        // Opacity and colour per channel, from photos read again in colour:
        // inside = (1 - a) * outside + a * colour.
        let colour_from = evenly(&picked, COLOUR_SAMPLES);
        let mut xs: [Vec<f32>; 3] = Default::default();
        let mut ys: [Vec<f32>; 3] = Default::default();
        for p in &colour_from {
            if stop.load(Ordering::Relaxed) {
                bail!("stopped");
            }
            let Ok(Ok(img)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open(p).and_then(|i| to_frame(i, fw)))) else { continue };
            let off = (fh as i64 - img.height as i64) / 2;
            for q in &pairs {
                let (iy, oy) = (q[1].round() as i64 - off, q[3].round() as i64 - off);
                if iy < 0 || oy < 0 || iy >= img.height as i64 || oy >= img.height as i64 {
                    continue;
                }
                let a = (iy as usize * w + q[0].round() as usize) * 3;
                let b = (oy as usize * w + q[2].round() as usize) * 3;
                let (pi, po) = (&img.rgb[a..a + 3], &img.rgb[b..b + 3]);
                if pi.iter().chain(po).any(|&v| v >= SATURATED) {
                    continue;
                }
                for c in 0..3 {
                    xs[c].push(po[c] as f32);
                    ys[c].push(pi[c] as f32);
                }
            }
            progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
        }
        let mut alpha = [0f32; 3];
        let mut colour = [0f32; 3];
        for c in 0..3 {
            let (k, b) = robust_line(&xs[c], &ys[c]).context("the watermark could not be measured")?;
            let a = 1.0 - k;
            if !(0.03..=0.9).contains(&a) {
                bail!("the watermark could not be measured (it looks {:.0}% opaque)", a * 100.0);
            }
            alpha[c] = a;
            colour[c] = (b / a).clamp(0.0, 400.0);
        }
        let a_mean = (alpha[0] + alpha[1] + alpha[2]) / 3.0;

        // How much of each pixel the mark covers: all of it inside, its own reading at the edge.
        let near = {
            let out = morph_n(&mask, w, h, true, 2);
            let inn = morph_n(&mask, w, h, false, 2);
            (0..w * h).map(|i| out[i] && !inn[i]).collect::<Vec<bool>>()
        };
        let cover: Vec<f32> = (0..w * h)
            .map(|i| if near[i] { (opacity[i] / a_mean).clamp(0.0, 1.0) } else if mask[i] { 1.0 } else { 0.0 })
            .collect();
        if let Some(p) = std::env::var_os("BOB_UNMARK_DEBUG") {
            let px: Vec<u8> = cover.iter().map(|&c| (c * 255.0).round() as u8).collect();
            let op: Vec<u8> = opacity.iter().map(|&a| (a / 0.5 * 255.0).clamp(0.0, 255.0) as u8).collect();
            let p = PathBuf::from(p);
            let _ = image::save_buffer(p.join("cover.png"), &px, fw, fh, image::ExtendedColorType::L8);
            let _ = image::save_buffer(p.join("opacity.png"), &op, fw, fh, image::ExtendedColorType::L8);
        }
        // Points on the mark's edge with the way out of it, to line the mark up on each photo.
        let rim = {
            let o = morph_n(&mask, w, h, true, 3);
            let i = morph_n(&mask, w, h, false, 3);
            (0..w * h).map(|j| o[j] && !i[j]).collect::<Vec<bool>>()
        };
        let mut edges = Vec::new();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = y * w + x;
                if !rim[i] {
                    continue;
                }
                let (cx, cy) = ((gx[i - 1] + gx[i]) * 0.5, (gy[i - w] + gy[i]) * 0.5);
                let m = cx.hypot(cy);
                if m > 0.06 {
                    edges.push([x as f32, y as f32, cx / m, cy / m, m]);
                }
            }
        }
        let thin = (edges.len() / 40_000).max(1);
        let edges: Vec<[f32; 5]> = edges.into_iter().step_by(thin).collect();
        Ok(Mark {
            fw,
            fh,
            cover,
            alpha,
            colour,
            pairs,
            edges,
            coverage: covered as f32 / (w * h) as f32,
            learned_from: samples.len(),
        })
    }

    fn base_placement(&self, iw: u32, ih: u32) -> Placement {
        let ph = placed_height(self.fw, iw, ih) as f32;
        Placement {
            sx: self.fw as f32 / iw as f32,
            sy: ph / ih as f32,
            off: (self.fh as f32 - ph) / 2.0,
            cx: self.fw as f32 / 2.0,
            cy: self.fh as f32 / 2.0,
            k: 1.0,
            ox: 0.0,
            oy: 0.0,
        }
    }

    /// How well the mark's edges at `pl` match the photo's (higher is better).
    fn fit(&self, ln: &LnMap, pl: &Placement) -> f32 {
        let (mut sum, mut wsum) = (0f32, 0f32);
        for e in &self.edges {
            let (ax, ay) = pl.image(e[0] + e[2], e[1] + e[3]);
            let (bx, by) = pl.image(e[0] - e[2], e[1] - e[3]);
            let d = ln.at(ax, ay) - ln.at(bx, by);
            if d.is_finite() {
                sum += e[4] * d;
                wsum += e[4];
            }
        }
        if wsum > 0.0 {
            sum / wsum
        } else {
            f32::NEG_INFINITY
        }
    }

    /// Where the mark sits on this photo: its width rule, then nudged by up to 3 frame pixels
    /// and a fraction of a percent in size until its edges meet the photo's.
    fn place(&self, ln: &LnMap) -> Placement {
        let base = self.base_placement(ln.w as u32, ln.h as u32);
        let mut best = (self.fit(ln, &base), base);
        let grid: Vec<(f32, f32)> = (-6..=6).flat_map(|j| (-6..=6).map(move |i| (i as f32 * 0.5, j as f32 * 0.5))).collect();
        let scored: Vec<(f32, Placement)> = grid
            .par_iter()
            .map(|&(ox, oy)| {
                let pl = Placement { ox, oy, ..base };
                (self.fit(ln, &pl), pl)
            })
            .collect();
        for c in scored {
            if c.0 > best.0 {
                best = c;
            }
        }
        // Then size and a finer nudge around the best.
        let around = best.1;
        let mut tries = Vec::new();
        for k in [0.994f32, 0.997, 1.0, 1.003, 1.006] {
            for j in -2..=2 {
                for i in -2..=2 {
                    tries.push(Placement { k, ox: around.ox + i as f32 * 0.25, oy: around.oy + j as f32 * 0.25, ..around });
                }
            }
        }
        for c in tries.par_iter().map(|pl| (self.fit(ln, pl), *pl)).collect::<Vec<_>>() {
            if c.0 > best.0 {
                best = c;
            }
        }
        best.1
    }

    /// How strongly the photo shows the mark at `pl`: about 1 when it carries it, about 0 when
    /// it does not. `None` when the photo is too bright there to tell.
    fn presence_at(&self, ln: &LnMap, pl: &Placement) -> Option<f32> {
        let a_mean = (self.alpha[0] + self.alpha[1] + self.alpha[2]) / 3.0;
        let step = (1.0 - a_mean).ln();
        let mut d = Vec::new();
        for q in &self.pairs {
            let (ix, iy) = pl.image(q[0], q[1]);
            let (ox, oy) = pl.image(q[2], q[3]);
            let v = ln.at(ix, iy) - ln.at(ox, oy);
            if v.is_finite() {
                d.push(v);
            }
        }
        (d.len() >= 40).then(|| median(&mut d) / step)
    }

    /// Line the mark up on `img` and say how strongly it shows there (see `presence_at`).
    pub fn prepare(&self, img: &RgbImage) -> (Placement, Option<f32>) {
        let ln = LnMap::new(img);
        let pl = self.place(&ln);
        let p = self.presence_at(&ln, &pl);
        (pl, p)
    }

    /// Take the mark off one photo (any size: the mark follows its width).
    pub fn remove(&self, img: &RgbImage) -> RgbImage {
        let (pl, _) = self.prepare(img);
        self.remove_at(img, &pl)
    }

    /// Take the mark off one photo, placed at `pl` (from `prepare`).
    pub fn remove_at(&self, img: &RgbImage, pl: &Placement) -> RgbImage {
        let (w, h) = (img.width as usize, img.height as usize);
        let (fw, fh) = (self.fw as usize, self.fh as usize);
        let cover_at = |x: usize, y: usize| -> f32 {
            let (fx, fy) = pl.frame(x as f32, y as f32);
            if fx < -1.0 || fy < -1.0 || fx > fw as f32 || fy > fh as f32 {
                return 0.0;
            }
            let (fx, fy) = (fx.clamp(0.0, (fw - 1) as f32), fy.clamp(0.0, (fh - 1) as f32));
            let (x0, y0) = (fx as usize, fy as usize);
            let (x1, y1) = ((x0 + 1).min(fw - 1), (y0 + 1).min(fh - 1));
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let c = |xx: usize, yy: usize| self.cover[yy * fw + xx];
            (c(x0, y0) * (1.0 - tx) + c(x1, y0) * tx) * (1.0 - ty) + (c(x0, y1) * (1.0 - tx) + c(x1, y1) * tx) * ty
        };
        let mut s = vec![0f32; w * h];
        s.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = cover_at(x, y);
            }
        });

        // Un-blend.
        let mut out: Vec<f32> = img.rgb.iter().map(|&v| v as f32).collect();
        out.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let k = s[y * w + x];
                if k <= 0.002 {
                    continue;
                }
                for c in 0..3 {
                    let a = (k * self.alpha[c]).min(0.95);
                    let v = &mut row[x * 3 + c];
                    *v = ((*v - a * self.colour[c]) / (1.0 - a)).clamp(0.0, 255.0);
                }
            }
        });

        // The edge: where the mark is partly there, and a little either side (the learned edge
        // is only as sharp as the frame it was learned in), filled smoothly from both sides.
        let reach = (1.0 / (pl.sx * pl.k)).ceil() as usize + 1;
        let partial: Vec<bool> = s.iter().map(|&k| k > 0.01 && k < 0.99).collect();
        let band = morph_n(&partial, w, h, true, reach);
        let idx: Vec<usize> = (0..w * h).filter(|&i| band[i] && i % w > 0 && i % w + 1 < w && i / w > 0 && i / w + 1 < h).collect();
        if !idx.is_empty() {
            // Start from the un-blended values and relax towards the surroundings.
            for _ in 0..(6 * reach * reach + 20).min(160) {
                for &i in &idx {
                    for c in 0..3 {
                        let v = (out[(i - 1) * 3 + c] + out[(i + 1) * 3 + c] + out[(i - w) * 3 + c] + out[(i + w) * 3 + c]) * 0.25;
                        out[i * 3 + c] = v;
                    }
                }
            }
        }
        RgbImage { width: img.width, height: img.height, rgb: out.iter().map(|&v| v.round().clamp(0.0, 255.0) as u8).collect() }
    }
}

/// Where the mark lands on one photo: the photo's width brought to the frame, centred, then
/// scaled by `k` about the frame's centre and moved by (`ox`, `oy`) frame pixels.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    sx: f32,
    sy: f32,
    off: f32,
    cx: f32,
    cy: f32,
    k: f32,
    ox: f32,
    oy: f32,
}

impl Placement {
    /// Photo pixel to frame position.
    #[inline]
    fn frame(&self, x: f32, y: f32) -> (f32, f32) {
        let fx = (x + 0.5) * self.sx - 0.5;
        let fy = (y + 0.5) * self.sy - 0.5 + self.off;
        (self.cx + (fx - self.cx) * self.k + self.ox, self.cy + (fy - self.cy) * self.k + self.oy)
    }

    /// Frame position to photo pixel.
    #[inline]
    fn image(&self, u: f32, v: f32) -> (f32, f32) {
        let fx = (u - self.ox - self.cx) / self.k + self.cx;
        let fy = (v - self.oy - self.cy) / self.k + self.cy;
        ((fx + 0.5) / self.sx - 0.5, (fy - self.off + 0.5) / self.sy - 0.5)
    }
}

/// ln(L - grey) of a photo, NaN where it is too bright to say anything.
struct LnMap {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl LnMap {
    fn new(img: &RgbImage) -> LnMap {
        let v = img
            .rgb
            .par_chunks_exact(3)
            .map(|c| if c[0].max(c[1]).max(c[2]) >= SATURATED { f32::NAN } else { lnl(lum(c[0], c[1], c[2])) })
            .collect();
        LnMap { w: img.width as usize, h: img.height as usize, v }
    }

    /// Bilinear read (NaN outside or next to a too-bright pixel).
    #[inline]
    fn at(&self, x: f32, y: f32) -> f32 {
        if !(x >= 0.0 && y >= 0.0 && x <= (self.w - 1) as f32 && y <= (self.h - 1) as f32) {
            return f32::NAN;
        }
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let p = |xx: usize, yy: usize| self.v[yy * self.w + xx];
        (p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx) * (1.0 - ty) + (p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx) * ty
    }
}

/// Learn the mark from `paths`, then take it off each photo that carries it, writing
/// `<name>.jpg` into `dir`. The photos themselves are only read. `stage` is set to 0 while
/// learning and 2 while removing; `progress` gets (done, total) for each.
pub fn unmark_all(paths: &[PathBuf], dir: &Path, stop: &AtomicBool, stage: &AtomicUsize, progress: impl Fn(usize, usize) + Sync) -> Result<(Mark, CleanSummary)> {
    let dir_l = dir.to_string_lossy().to_lowercase();
    let paths: Vec<PathBuf> = paths.iter().filter(|p| !p.to_string_lossy().to_lowercase().starts_with(&dir_l)).cloned().collect();
    stage.store(0, Ordering::Relaxed);
    let mark = Mark::learn(&paths, stop, &progress)?;
    stage.store(2, Ordering::Relaxed);
    progress(0, paths.len());
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let done = AtomicUsize::new(0);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().context("cannot start worker threads")?;
    // Some(Ok(true)) written, Some(Ok(false)) no mark there.
    let results: Vec<Option<(String, Result<bool>)>> = pool.install(|| {
        paths
            .par_iter()
            .map(|p| {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<bool> {
                    let img = open(p)?;
                    let (pl, presence) = mark.prepare(&img);
                    if presence.is_some_and(|v| v < 0.45) {
                        return Ok(false);
                    }
                    let clean = mark.remove_at(&img, &pl);
                    let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "photo".into());
                    save_jpeg(&clean, &dir.join(format!("{stem}.jpg")))?;
                    Ok(true)
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the photo could not be processed")));
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, paths.len());
                Some((name, r))
            })
            .collect()
    });
    let mut s = CleanSummary { dir: dir.to_path_buf(), stopped: stop.load(Ordering::Relaxed), ..Default::default() };
    for (name, r) in results.into_iter().flatten() {
        match r {
            Ok(true) => s.written += 1,
            Ok(false) => s.unmatched += 1,
            Err(e) => s.errors.push((name, format!("{e:#}"))),
        }
    }
    Ok((mark, s))
}
