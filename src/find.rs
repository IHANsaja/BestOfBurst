//! Find the original of a photo someone sent back: a watermarked preview, a re-saved or resized
//! copy, or the original itself. Works from the thumbnails the analysis already wrote, so a
//! lookup never re-reads the shoot.
//!
//! Three steps per photo: a coarse 32 x 32 brightness grid shortlists look-alikes (tried in all
//! four turns, since previews are upright and stored thumbnails are not), a 96 x 96 comparison
//! with a brightness/contrast fit picks the exact frame out of its burst, and a second pass
//! learns where the watermark sits (the part of the preview that differs the same way on every
//! match) and compares again without it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::decode::{self, RgbImage};
use crate::scan;
use crate::state::State;

pub const COARSE: usize = 32;
pub const FINE: usize = 96;
/// Look-alikes carried from the coarse step into the exact comparison.
pub const SHORTLIST: usize = 24;
/// Share of the frame (the best-agreeing part) that decides "same photo": a watermark may spoil
/// the rest.
pub const TRIM: f32 = 0.70;
/// Similarity (0..100) from which a match is trusted, and from which it is at least shown.
pub const FOUND_MIN: f32 = 80.0;
/// A crop must lift the match at least to this before it is believed.
pub const CROP_MIN: f32 = 72.0;
pub const CHECK_MIN: f32 = 50.0;
/// The runner-up must be this much worse (ratio of leftover difference) for a sure match.
pub const CLEAR_RATIO: f32 = 0.80;
/// ...unless both are this close to pixel-identical; then the levels gap decides.
pub const CLEAR_LEVELS: f32 = 1.2;
/// Look-alikes that get the crop search (the closest ones, plus any whose file name matches).
pub const ALIGN_TOP: usize = 5;
/// A sent photo may show as little as this share of the original's width or height.
pub const MIN_CROP_SIDE: f32 = 0.6;
/// A crop is believed only when it lifts the similarity by this much over the full frame (and
/// to a sure match): an uncropped explanation is always preferred.
pub const CROP_GAIN: f32 = 8.0;
/// Between near-equal candidates the uncropped one wins: a cropped one's leftover counts this
/// much more.
pub const CROP_HANDICAP: f32 = 1.15;
/// Matches of one shape needed before a watermark is learned from them.
pub const WM_MIN_MATCHES: usize = 4;
/// A cell belongs to the watermark when previews differ from their originals there by this many
/// brightness levels, the same way, on at least half of the matches.
pub const WM_LEVELS: f32 = 4.0;
/// The folder BestOfBurst fills with the found originals (and empties before refilling).
pub const FOUND_DIR: &str = "found originals";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Shape {
    Landscape,
    Portrait,
    Square,
}

impl Shape {
    pub fn of(w: u32, h: u32) -> Shape {
        let (w, h) = (w as f32, h as f32);
        if w > h * 1.1 {
            Shape::Landscape
        } else if h > w * 1.1 {
            Shape::Portrait
        } else {
            Shape::Square
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Shape::Landscape => "landscape",
            Shape::Portrait => "portrait",
            Shape::Square => "square",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// One original, clearly.
    Found,
    /// A likely original, but look: a burst frame is nearly as close, or the match is weak.
    Check,
    /// Nothing in this shoot looks like it.
    NotFound,
    /// The sent file could not be opened.
    Failed,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Found => "found",
            Verdict::Check => "check",
            Verdict::NotFound => "not_found",
            Verdict::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// Path of the original relative to the shoot folder (the key in `state.json`).
    pub path: String,
    /// 0..100: how much of the sent photo this original explains, watermark aside.
    pub similarity: f32,
    /// Mean leftover difference in brightness levels over the whole frame (lower is closer).
    pub diff: f32,
    /// Quarter turns between the stored thumbnail and the sent photo.
    pub turns: u8,
    /// The file names share their stem (IMG_1234 and IMG_1234-preview).
    pub name_match: bool,
    /// Other files of the same frame (the JPEG beside a RAW): delivered together.
    pub twins: Vec<String>,
    /// The part of the original the sent photo shows: left, top, right, bottom as 0..1
    /// (0, 0, 1, 1 when it was not cropped).
    pub crop: [f32; 4],
}

impl Candidate {
    pub fn cropped(&self) -> bool {
        self.crop != FULL_FRAME
    }
}

pub const FULL_FRAME: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

#[derive(Clone, Debug)]
pub struct Match {
    pub query: PathBuf,
    pub verdict: Verdict,
    pub best: Option<Candidate>,
    pub runner_up: Option<Candidate>,
    pub error: Option<String>,
    pub shape: Shape,
    /// A small JPEG of the sent photo for the window, when a folder for them was given.
    pub thumb: Option<PathBuf>,
}

impl Match {
    /// The original to deliver: the best candidate of a found or to-check match.
    pub fn original(&self) -> Option<&str> {
        match self.verdict {
            Verdict::Found | Verdict::Check => self.best.as_ref().map(|c| c.path.as_str()),
            _ => None,
        }
    }
    /// Every file to deliver: the original and its RAW or JPEG twin.
    pub fn originals(&self) -> Vec<&str> {
        match (self.original(), &self.best) {
            (Some(o), Some(b)) => std::iter::once(o).chain(b.twins.iter().map(|t| t.as_str())).collect(),
            _ => Vec::new(),
        }
    }
    /// Take the runner-up instead (the user looked and chose the other burst frame).
    pub fn swap(&mut self) {
        if self.best.is_some() && self.runner_up.is_some() {
            std::mem::swap(&mut self.best, &mut self.runner_up);
            self.verdict = Verdict::Found;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Watermark {
    /// Share of the frame the learned watermark covers, 0..1.
    pub coverage: f32,
    /// Matches it was learned from.
    pub learned_from: usize,
}

#[derive(Clone, Debug, Default)]
pub struct FindReport {
    pub matches: Vec<Match>,
    /// A watermark learned per preview shape, when there was one.
    pub watermarks: BTreeMap<Shape, Watermark>,
    pub stopped: bool,
}

impl FindReport {
    pub fn count(&self, v: Verdict) -> usize {
        self.matches.iter().filter(|m| m.verdict == v).count()
    }
}

// ---- Grids ----

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Brightness averaged into an `n` x `n` grid stretched over the whole image (0..255).
pub fn luma_grid(img: &RgbImage, n: usize) -> Vec<f32> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = vec![0f32; n * n];
    if w == 0 || h == 0 {
        return out;
    }
    let xs: Vec<(usize, usize)> = (0..n).map(|c| span(c, n, w)).collect();
    for r in 0..n {
        let (y0, y1) = span(r, n, h);
        for (c, &(x0, x1)) in xs.iter().enumerate() {
            let mut s = 0f32;
            for y in y0..y1 {
                let row = &img.rgb[(y * w + x0) * 3..(y * w + x1) * 3];
                for p in row.chunks_exact(3) {
                    s += LUMA[0] * p[0] as f32 + LUMA[1] * p[1] as f32 + LUMA[2] * p[2] as f32;
                }
            }
            out[r * n + c] = s / ((y1 - y0) * (x1 - x0)) as f32;
        }
    }
    out
}

/// Source pixels of cell `c` of `n` along an edge of `len` pixels: never empty.
fn span(c: usize, n: usize, len: usize) -> (usize, usize) {
    let a = (c * len / n).min(len - 1);
    let b = ((c + 1) * len).div_ceil(n).clamp(a + 1, len);
    (a, b)
}

/// The grid turned a quarter clockwise.
pub fn turn_cw(g: &[f32], n: usize) -> Vec<f32> {
    let mut out = vec![0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            out[y * n + x] = g[(n - 1 - x) * n + y];
        }
    }
    out
}

pub fn turned(g: &[f32], n: usize, turns: u8) -> Vec<f32> {
    let mut out = g.to_vec();
    for _ in 0..turns % 4 {
        out = turn_cw(&out, n);
    }
    out
}

/// Zero mean, unit length: the dot product of two such grids is their correlation.
pub fn normalized(g: &[f32]) -> Vec<f32> {
    let mean = g.iter().sum::<f32>() / g.len().max(1) as f32;
    let norm = g.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>().sqrt();
    if norm < 1e-3 {
        return vec![0.0; g.len()];
    }
    g.iter().map(|v| (v - mean) / norm).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ---- Exact comparison ----

#[derive(Clone, Debug)]
pub struct Fit {
    /// 0..100, from the best-agreeing `TRIM` share of the frame.
    pub similarity: f32,
    /// Mean leftover difference over every compared cell, in levels (capped per cell).
    pub diff: f32,
    /// Sent photo minus fitted original, per cell (0 where masked out).
    pub residual: Vec<f32>,
}

/// Mean of the smallest `share` of the values.
fn trimmed_mean(v: &mut [f32], share: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let k = (((v.len() as f32) * share).ceil() as usize).clamp(1, v.len());
    v.select_nth_unstable_by(k - 1, |a, b| a.total_cmp(b));
    v[..k].iter().sum::<f32>() / k as f32
}

/// Least-squares `q = gain * o + offset` over the cells in `use_`.
fn line_fit(q: &[f32], o: &[f32], use_: &[bool]) -> (f32, f32) {
    let (mut n, mut sq, mut so, mut soo, mut sqo) = (0f64, 0f64, 0f64, 0f64, 0f64);
    for i in 0..q.len() {
        if use_[i] {
            let (a, b) = (q[i] as f64, o[i] as f64);
            n += 1.0;
            sq += a;
            so += b;
            soo += b * b;
            sqo += a * b;
        }
    }
    if n < 2.0 {
        return (1.0, 0.0);
    }
    let var = soo - so * so / n;
    if var < 1e-6 {
        return (1.0, ((sq - so) / n) as f32);
    }
    // A preview is the same picture, not its negative or a wash: keep the gain sensible.
    let gain = ((sqo - sq * so / n) / var).clamp(0.4, 2.5);
    (gain as f32, ((sq - gain * so) / n) as f32)
}

/// Compare a sent photo (`q`) with an original (`o`), both `FINE` grids in the sent photo's
/// orientation. Cells where `mask` is true (the watermark) are left out.
pub fn compare(q: &[f32], o: &[f32], mask: Option<&[bool]>) -> Fit {
    let n = q.len();
    let keep: Vec<bool> = (0..n).map(|i| mask.map_or(true, |m| !m[i])).collect();
    let kept = keep.iter().filter(|k| **k).count();
    if kept < n / 8 {
        return Fit { similarity: 0.0, diff: f32::MAX, residual: vec![0.0; n] };
    }
    // Fit, drop the worst-agreeing cells (watermark, a moved subject), fit again.
    let (g, off) = line_fit(q, o, &keep);
    let mut abs: Vec<f32> = (0..n).filter(|&i| keep[i]).map(|i| (q[i] - (g * o[i] + off)).abs()).collect();
    let k = (((abs.len() as f32) * TRIM).ceil() as usize).clamp(1, abs.len());
    abs.select_nth_unstable_by(k - 1, |a, b| a.total_cmp(b));
    let cut = abs[k - 1];
    let inl: Vec<bool> = (0..n).map(|i| keep[i] && (q[i] - (g * o[i] + off)).abs() <= cut).collect();
    let (g, off) = line_fit(q, o, &inl);

    let mut residual = vec![0f32; n];
    let mut abs = Vec::with_capacity(kept);
    let mut full = 0f32;
    for i in 0..n {
        if keep[i] {
            let r = q[i] - (g * o[i] + off);
            residual[i] = r;
            abs.push(r.abs());
            full += r.abs().min(48.0);
        }
    }
    let left = trimmed_mean(&mut abs, TRIM);

    // How much the sent photo varies on its own, measured the same trimmed way: a flat grey
    // card matches nothing, however small the leftover.
    let mut vals: Vec<f32> = (0..n).filter(|&i| keep[i]).map(|i| q[i]).collect();
    let mid = vals.len() / 2;
    vals.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    let median = vals[mid];
    let mut dev: Vec<f32> = vals.iter().map(|v| (v - median).abs()).collect();
    let own = trimmed_mean(&mut dev, TRIM);
    let similarity = if own < 1.0 { 0.0 } else { (100.0 * (1.0 - left / own)).clamp(0.0, 100.0) };
    Fit { similarity, diff: full / kept as f32, residual }
}

// ---- The shoot's index ----

pub struct Index {
    /// Relative paths of the originals, parallel to `thumbs` and `sigs`.
    pub paths: Vec<String>,
    thumbs: Vec<PathBuf>,
    /// `COARSE` x `COARSE` normalized grids, one after another.
    sigs: Vec<f32>,
    fine: Mutex<HashMap<usize, std::sync::Arc<Luma>>>,
}

/// The brightness of a thumbnail, kept so an original can be re-cut when the sent photo is a
/// crop of it.
pub struct Luma {
    w: usize,
    h: usize,
    v: Vec<u8>,
}

impl Luma {
    fn of(img: &RgbImage) -> Luma {
        let (w, h) = (img.width as usize, img.height as usize);
        let v = img.rgb.chunks_exact(3).take(w * h).map(|p| (LUMA[0] * p[0] as f32 + LUMA[1] * p[1] as f32 + LUMA[2] * p[2] as f32).round() as u8).collect();
        Luma { w, h, v }
    }

    /// Summed-area table of the image turned `turns` quarters clockwise: (w, h, sums).
    fn table(&self, turns: u8) -> Table {
        let swap = turns % 2 == 1;
        let (w, h) = if swap { (self.h, self.w) } else { (self.w, self.h) };
        let at = |x: usize, y: usize| -> f64 {
            let (sx, sy) = match turns % 4 {
                1 => (y, self.h - 1 - x),
                2 => (self.w - 1 - x, self.h - 1 - y),
                3 => (self.w - 1 - y, x),
                _ => (x, y),
            };
            self.v[sy * self.w + sx] as f64
        };
        let mut sums = vec![0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0f64;
            for x in 0..w {
                row += at(x, y);
                sums[(y + 1) * (w + 1) + x + 1] = sums[y * (w + 1) + x + 1] + row;
            }
        }
        Table { w, h, sums }
    }
}

struct Table {
    w: usize,
    h: usize,
    sums: Vec<f64>,
}

impl Table {
    /// Sum of the image over [0, x) x [0, y), at fractional positions.
    fn upto(&self, x: f64, y: f64) -> f64 {
        let (x, y) = (x.clamp(0.0, self.w as f64), y.clamp(0.0, self.h as f64));
        let (x0, y0) = ((x.floor() as usize).min(self.w - 1), (y.floor() as usize).min(self.h - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let s = |xi: usize, yi: usize| self.sums[yi * (self.w + 1) + xi];
        (s(x0, y0) * (1.0 - fx) + s(x0 + 1, y0) * fx) * (1.0 - fy) + (s(x0, y0 + 1) * (1.0 - fx) + s(x0 + 1, y0 + 1) * fx) * fy
    }

    /// The `FINE` grid of the part `rect` (left, top, right, bottom as 0..1) of the image.
    fn grid(&self, rect: [f32; 4]) -> Vec<f32> {
        self.grid_n(rect, FINE)
    }

    /// The same at `n` x `n` cells.
    fn grid_n(&self, rect: [f32; 4], n: usize) -> Vec<f32> {
        let (l, t) = (rect[0] as f64 * self.w as f64, rect[1] as f64 * self.h as f64);
        let (cw, ch) = ((rect[2] - rect[0]) as f64 * self.w as f64 / n as f64, (rect[3] - rect[1]) as f64 * self.h as f64 / n as f64);
        // Corner sums once, then every cell is four of them.
        let mut corner = vec![0f64; (n + 1) * (n + 1)];
        for r in 0..=n {
            for c in 0..=n {
                corner[r * (n + 1) + c] = self.upto(l + c as f64 * cw, t + r as f64 * ch);
            }
        }
        let area = (cw * ch).max(1e-9);
        let mut out = vec![0f32; n * n];
        for r in 0..n {
            for c in 0..n {
                let s = corner[(r + 1) * (n + 1) + c + 1] - corner[r * (n + 1) + c + 1] - corner[(r + 1) * (n + 1) + c] + corner[r * (n + 1) + c];
                out[r * n + c] = (s / area) as f32;
            }
        }
        out
    }
}

/// Cells per side of the rough grid the crop search starts on.
const ROUGH: usize = 24;
/// Each edge is tried this far in, in steps of 4% (0, 4, ... 20%).
const ROUGH_STEPS: usize = 6;

/// `FINE` grid averaged down to `ROUGH`, and the mask with it (a rough cell is masked when
/// most of it is).
fn roughen(q: &[f32], mask: Option<&[bool]>) -> (Vec<f32>, Option<Vec<bool>>) {
    let k = FINE / ROUGH;
    let mut out = vec![0f32; ROUGH * ROUGH];
    let mut m = vec![false; ROUGH * ROUGH];
    for r in 0..ROUGH {
        for c in 0..ROUGH {
            let (mut sum, mut masked) = (0f32, 0usize);
            for y in 0..k {
                for x in 0..k {
                    let i = (r * k + y) * FINE + c * k + x;
                    sum += q[i];
                    masked += mask.is_some_and(|m| m[i]) as usize;
                }
            }
            out[r * ROUGH + c] = sum / (k * k) as f32;
            m[r * ROUGH + c] = masked * 2 > k * k;
        }
    }
    (out, mask.map(|_| m))
}

/// Compare the sent photo with an original, looking for the crop that lines them up. First a
/// rough look at every combination of the four edges moved in by 0 to 20% (on a small grid,
/// where a near miss still scores), then each edge is nudged in shrinking steps on the full
/// grid while the match improves.
fn align(q: &[f32], table: &Table, mask: Option<&[bool]>, search: bool, start: [f32; 4]) -> (Fit, [f32; 4]) {
    let full = compare(q, &table.grid(start), mask);
    if !search || full.similarity >= 96.0 {
        return (full, start);
    }
    let (rq, rmask) = roughen(q, mask);
    let (mut rect, mut top) = (start, compare(&rq, &table.grid_n(start, ROUGH), rmask.as_deref()).similarity);
    let at = |k: usize| k as f32 * 0.04;
    for l in 0..ROUGH_STEPS {
        for t in 0..ROUGH_STEPS {
            for r in 0..ROUGH_STEPS {
                for b in 0..ROUGH_STEPS {
                    let cand = [at(l), at(t), 1.0 - at(r), 1.0 - at(b)];
                    if l + t + r + b == 0 || cand[2] - cand[0] < MIN_CROP_SIDE || cand[3] - cand[1] < MIN_CROP_SIDE {
                        continue;
                    }
                    let sim = compare(&rq, &table.grid_n(cand, ROUGH), rmask.as_deref()).similarity;
                    if sim > top + 1.0 {
                        (rect, top) = (cand, sim);
                    }
                }
            }
        }
    }
    if rect == start {
        return (full, start);
    }
    let mut best = compare(q, &table.grid(rect), mask);
    let mut step = 0.02f32;
    for _ in 0..40 {
        let mut improved = false;
        for edge in 0..4 {
            for dir in [1.0f32, -1.0] {
                let mut r = rect;
                // Positive moves an edge inwards.
                r[edge] = (r[edge] + if edge < 2 { dir * step } else { -dir * step }).clamp(0.0, 1.0);
                if r == rect || r[2] - r[0] < MIN_CROP_SIDE || r[3] - r[1] < MIN_CROP_SIDE {
                    continue;
                }
                let f = compare(q, &table.grid(r), mask);
                if f.similarity > best.similarity + 0.25 {
                    (best, rect, improved) = (f, r, true);
                }
            }
        }
        if !improved {
            step /= 2.0;
            if step < 0.004 {
                break;
            }
        }
    }
    // A crop has to earn its keep: without a clear gain the full frame stands. (Otherwise the
    // next frame of a burst, shifted a little, could pass for this one.)
    if best.similarity < CROP_MIN || best.similarity < full.similarity + CROP_GAIN {
        return (full, start);
    }
    (best, rect)
}

fn read_thumb(path: &Path) -> Option<RgbImage> {
    let data = std::fs::read(path).ok()?;
    std::panic::catch_unwind(|| decode::decode_jpeg_bytes(&data).ok()).ok().flatten()
}

impl Index {
    /// Read every analyzed photo's thumbnail once (in parallel) and keep its coarse grid.
    pub fn build(state: &State, out: &Path) -> Index {
        let list: Vec<(String, PathBuf)> = state
            .files
            .iter()
            .filter(|(_, f)| f.is_done())
            .filter_map(|(p, f)| f.thumb.as_ref().map(|t| (p.clone(), out.join(t))))
            .collect();
        let grids: Vec<Option<Vec<f32>>> = list
            .par_iter()
            .map(|(_, t)| read_thumb(t).map(|img| normalized(&luma_grid(&img, COARSE))))
            .collect();
        let mut idx = Index { paths: Vec::new(), thumbs: Vec::new(), sigs: Vec::new(), fine: Mutex::new(HashMap::new()) };
        for ((path, thumb), grid) in list.into_iter().zip(grids) {
            if let Some(g) = grid {
                idx.paths.push(path);
                idx.thumbs.push(thumb);
                idx.sigs.extend_from_slice(&g);
            }
        }
        idx
    }

    /// An index straight from a folder of originals that was never culled: every photo is
    /// opened once (in parallel) and kept small in memory.
    pub fn from_folder(input: &Path, stop: &AtomicBool, progress: impl Fn(usize, usize) + Sync) -> Index {
        let items = scan::scan(input, &scan::default_out(input), Some(stop));
        let done = AtomicUsize::new(0);
        let small: Vec<Option<RgbImage>> = items
            .par_iter()
            .map(|i| {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                let r = std::panic::catch_unwind(|| decode::decode_full(&i.abs, i.format).ok().and_then(|d| decode::resize_long_edge(d.image, 320).ok()));
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, items.len());
                r.ok().flatten()
            })
            .collect();
        let mut idx = Index { paths: Vec::new(), thumbs: Vec::new(), sigs: Vec::new(), fine: Mutex::new(HashMap::new()) };
        let mut fine = HashMap::new();
        for (item, img) in items.iter().zip(small) {
            if let Some(img) = img {
                fine.insert(idx.paths.len(), std::sync::Arc::new(Luma::of(&img)));
                idx.paths.push(item.rel.clone());
                idx.thumbs.push(PathBuf::new());
                idx.sigs.extend_from_slice(&normalized(&luma_grid(&img, COARSE)));
            }
        }
        idx.fine = Mutex::new(fine);
        idx
    }

    /// The index of a folder of originals: from its culling results when it has them (fast),
    /// else by opening every photo.
    pub fn open(input: &Path, stop: &AtomicBool, progress: impl Fn(usize, usize) + Sync) -> Index {
        let out = scan::default_out(input);
        if let Ok(Some((state, _))) = State::load_existing(input, &out) {
            let idx = Index::build(&state, &out);
            if !idx.is_empty() {
                return idx;
            }
        }
        Index::from_folder(input, stop, progress)
    }

    /// An index over images held in memory (tests, and callers without thumbnails on disk).
    pub fn from_images(items: &[(String, RgbImage)]) -> Index {
        let mut idx = Index { paths: Vec::new(), thumbs: Vec::new(), sigs: Vec::new(), fine: Mutex::new(HashMap::new()) };
        let mut fine = HashMap::new();
        for (i, (path, img)) in items.iter().enumerate() {
            let small = small_copy(img);
            idx.paths.push(path.clone());
            idx.thumbs.push(PathBuf::new());
            idx.sigs.extend_from_slice(&normalized(&luma_grid(&small, COARSE)));
            fine.insert(i, std::sync::Arc::new(Luma::of(&small)));
        }
        idx.fine = Mutex::new(fine);
        idx
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    fn sig(&self, i: usize) -> &[f32] {
        &self.sigs[i * COARSE * COARSE..(i + 1) * COARSE * COARSE]
    }

    fn fine_of(&self, i: usize) -> Option<std::sync::Arc<Luma>> {
        if let Some(g) = self.fine.lock().ok().and_then(|m| m.get(&i).cloned()) {
            return Some(g);
        }
        let g = std::sync::Arc::new(Luma::of(&read_thumb(&self.thumbs[i])?));
        if let Ok(mut m) = self.fine.lock() {
            if m.len() > 1500 && !self.thumbs[i].as_os_str().is_empty() {
                m.clear();
            }
            m.insert(i, g.clone());
        }
        Some(g)
    }

    /// The `SHORTLIST` closest look-alikes of a coarse grid, over all four turns:
    /// (index, quarter turns of the original that line it up with the sent photo).
    fn shortlist(&self, coarse: &[f32]) -> Vec<(usize, u8)> {
        // Turning the sent photo by t is the same as turning the original by 4 - t.
        let q: Vec<Vec<f32>> = (0..4u8).map(|t| normalized(&turned(coarse, COARSE, t))).collect();
        let mut best: Vec<(f32, usize, u8)> = (0..self.len())
            .map(|i| {
                let s = self.sig(i);
                let (mut bc, mut bt) = (f32::MIN, 0u8);
                for (t, qt) in q.iter().enumerate() {
                    let c = dot(qt, s);
                    if c > bc {
                        (bc, bt) = (c, ((4 - t) % 4) as u8);
                    }
                }
                (bc, i, bt)
            })
            .collect();
        let k = SHORTLIST.min(best.len());
        if k == 0 {
            return Vec::new();
        }
        best.select_nth_unstable_by(k - 1, |a, b| b.0.total_cmp(&a.0));
        best.truncate(k);
        best.into_iter().map(|(_, i, t)| (i, t)).collect()
    }
}

fn small_copy(img: &RgbImage) -> RgbImage {
    decode::resize_long_edge(img.clone(), 320).unwrap_or_else(|_| img.clone())
}

fn stem(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let s = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    s.to_lowercase()
}

/// IMG_1234 and "0007_IMG_1234-preview" name the same frame; "1" and "10" do not.
pub fn names_match(original: &str, sent: &str) -> bool {
    let (a, b) = (stem(original), stem(sent));
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if short.len() < 4 || !short.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    long.match_indices(short.as_str()).any(|(at, _)| {
        // The frame number must not run on into more digits on either side.
        let before = long[..at].chars().next_back();
        let after = long[at + short.len()..].chars().next();
        let digit_edge = |edge: Option<char>, own: Option<char>| {
            edge.is_some_and(|c| c.is_ascii_digit()) && own.is_some_and(|c| c.is_ascii_digit())
        };
        !digit_edge(before, short.chars().next()) && !digit_edge(after, short.chars().next_back())
    })
}

// ---- One sent photo ----

struct Probe {
    fine: Vec<f32>,
    shape: Shape,
    shortlist: Vec<(usize, u8)>,
}

/// Crops found for a sent photo's look-alikes (by index position), so the second pass does
/// not search again.
type Crops = HashMap<usize, [f32; 4]>;

fn rate(index: &Index, query: &Path, p: &Probe, mask: Option<&[bool]>, known: Option<&Crops>) -> (Vec<(Candidate, Vec<f32>)>, Verdict, Crops) {
    let name = query.to_string_lossy();
    // Full frame first, for every look-alike.
    let mut first: Vec<(usize, u8, Table, Fit, bool)> = p
        .shortlist
        .iter()
        .filter_map(|&(i, turns)| {
            let table = index.fine_of(i)?.table(turns);
            let fit = compare(&p.fine, &table.grid(FULL_FRAME), mask);
            Some((i, turns, table, fit, names_match(&index.paths[i], &name)))
        })
        .collect();
    first.sort_by(|a, b| b.3.similarity.total_cmp(&a.3.similarity));
    let mut crops = Crops::new();
    let mut scored: Vec<(Candidate, Vec<f32>)> = first
        .into_iter()
        .enumerate()
        .map(|(rank, (i, turns, table, fit, name_match))| {
            // The closest few may be the photo behind a crop: line them up.
            let (fit, crop) = match known {
                Some(k) => match k.get(&i) {
                    Some(&r) if r != FULL_FRAME => align(&p.fine, &table, mask, false, r),
                    _ => (fit, FULL_FRAME),
                },
                None if rank < ALIGN_TOP || name_match => align(&p.fine, &table, mask, true, FULL_FRAME),
                None => (fit, FULL_FRAME),
            };
            crops.insert(i, crop);
            let c = Candidate {
                path: index.paths[i].clone(),
                similarity: fit.similarity,
                diff: fit.diff,
                turns,
                name_match,
                twins: Vec::new(),
                crop,
            };
            (c, fit.residual)
        })
        .collect();
    // Among the same-scene candidates the smallest leftover wins: a watermark costs every
    // burst frame the same, so the true frame still comes out lowest.
    let weighed = |c: &Candidate| if c.cropped() { c.diff * CROP_HANDICAP } else { c.diff };
    scored.sort_by(|a, b| {
        let (ka, kb) = (a.0.similarity >= CHECK_MIN, b.0.similarity >= CHECK_MIN);
        kb.cmp(&ka).then_with(|| weighed(&a.0).total_cmp(&weighed(&b.0))).then_with(|| a.0.path.cmp(&b.0.path))
    });
    // IMG_1234.CR3 and IMG_1234.JPG are one frame, not rivals: fold the twin into the leader.
    let frame = |p: &str| p.rfind('.').map_or(p, |i| &p[..i]).to_lowercase();
    let mut k = 0;
    while k < scored.len() {
        let key = frame(&scored[k].0.path);
        let mut j = k + 1;
        while j < scored.len() {
            if frame(&scored[j].0.path) == key {
                let twin = scored.remove(j).0;
                if twin.similarity >= CHECK_MIN && scored[k].0.similarity >= CHECK_MIN {
                    scored[k].0.twins.push(twin.path);
                }
            } else {
                j += 1;
            }
        }
        k += 1;
    }
    let verdict = match scored.first() {
        Some((best, _)) if best.similarity >= CHECK_MIN => {
            let rival = scored.get(1).map(|x| &x.0).filter(|r| r.similarity >= CHECK_MIN);
            let clear = match rival {
                None => true,
                Some(r) => {
                    best.diff <= CLEAR_RATIO * r.diff
                        || r.diff - best.diff >= CLEAR_LEVELS
                        || (best.name_match && !r.name_match)
                }
            };
            // A crop lined up by pixels alone can be the next frame of the burst: unless the
            // file name agrees, it is shown for a look rather than trusted.
            let trusted = !best.cropped() || best.name_match;
            if best.similarity >= FOUND_MIN && clear && trusted {
                Verdict::Found
            } else {
                Verdict::Check
            }
        }
        _ => Verdict::NotFound,
    };
    (scored, verdict, crops)
}

/// A name match settles a near-tie between burst frames in favour of the named one.
fn prefer_named(scored: &mut [(Candidate, Vec<f32>)]) {
    if scored.len() < 2 || scored[0].0.name_match {
        return;
    }
    let lead = scored[0].0.diff;
    if let Some(k) = scored.iter().position(|(c, _)| {
        c.name_match && c.similarity >= CHECK_MIN && c.diff <= lead / CLEAR_RATIO && c.diff - lead < CLEAR_LEVELS
    }) {
        scored[..=k].rotate_right(1);
    }
}

fn to_match(query: &Path, shape: Shape, thumb: Option<PathBuf>, scored: &[(Candidate, Vec<f32>)], verdict: Verdict) -> Match {
    Match {
        query: query.to_path_buf(),
        verdict,
        best: scored.first().map(|x| x.0.clone()),
        runner_up: scored.get(1).map(|x| x.0.clone()),
        error: None,
        shape,
        thumb,
    }
}

/// Where previews of one shape differ from their originals the same way on most matches.
fn learn_watermark(residuals: &[&Vec<f32>]) -> Option<(Vec<bool>, f32)> {
    if residuals.len() < WM_MIN_MATCHES {
        return None;
    }
    let n = FINE * FINE;
    let mut mask = vec![false; n];
    let mut col: Vec<f32> = Vec::with_capacity(residuals.len());
    for i in 0..n {
        col.clear();
        col.extend(residuals.iter().map(|r| r[i]));
        let mid = col.len() / 2;
        col.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
        mask[i] = col[mid].abs() >= WM_LEVELS;
    }
    // Grow by one cell: the soft edge of a mark differs less than its middle.
    let mut grown = mask.clone();
    for y in 0..FINE {
        for x in 0..FINE {
            if mask[y * FINE + x] {
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < FINE && (ny as usize) < FINE {
                        grown[ny as usize * FINE + nx as usize] = true;
                    }
                }
            }
        }
    }
    let coverage = grown.iter().filter(|m| **m).count() as f32 / n as f32;
    // Under half a percent is noise; over 60% is a different edit, not a mark.
    (0.005..=0.60).contains(&coverage).then_some((grown, coverage))
}

/// Match every sent photo against the shoot. `thumbs_dir` receives a small JPEG of each sent
/// photo for the window to show. `progress(done, total)` is called from worker threads.
pub fn find_originals(
    index: &Index,
    queries: &[PathBuf],
    thumbs_dir: Option<&Path>,
    stop: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> FindReport {
    let done = AtomicUsize::new(0);
    let total = queries.len();
    if let Some(d) = thumbs_dir {
        let _ = std::fs::create_dir_all(d);
    }

    // Pass 1: open, shortlist, compare.
    let first: Vec<Option<Result<(Probe, Option<PathBuf>), String>>> = queries
        .par_iter()
        .map(|q| {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(Probe, Option<PathBuf>)> {
                let format = scan::detect_file(q)
                    .with_context(|| format!("cannot read {}", q.display()))?
                    .context("not a photo BestOfBurst can open")?;
                let d = decode::decode_full(q, format)?;
                let shape = Shape::of(d.image.width, d.image.height);
                let small = decode::resize_long_edge(d.image, 320)?;
                let thumb = thumbs_dir.and_then(|dir| {
                    let t = dir.join(format!("{}.jpg", crate::analyze::fnv1a_hex(&q.to_string_lossy())));
                    crate::analyze::write_thumb(&small, &t).ok().map(|_| t)
                });
                let probe = Probe { fine: luma_grid(&small, FINE), shape, shortlist: index.shortlist(&luma_grid(&small, COARSE)) };
                Ok((probe, thumb))
            }));
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress(n, total);
            Some(match r {
                Ok(Ok(p)) => Ok(p),
                Ok(Err(e)) => Err(format!("{e:#}").split_whitespace().collect::<Vec<_>>().join(" ")),
                Err(_) => Err("decoder crashed".into()),
            })
        })
        .collect();

    let mut report = FindReport { stopped: stop.load(Ordering::Relaxed), ..Default::default() };
    let rated: Vec<Option<(Vec<(Candidate, Vec<f32>)>, Verdict, Crops)>> = first
        .par_iter()
        .zip(queries)
        .map(|(f, q)| match f {
            Some(Ok((p, _))) => Some(rate(index, q, p, None, None)),
            _ => None,
        })
        .collect();

    // Learn the watermark of each shape from the matches that already hold.
    let mut masks: HashMap<Shape, Vec<bool>> = HashMap::new();
    for shape in [Shape::Landscape, Shape::Portrait, Shape::Square] {
        let residuals: Vec<&Vec<f32>> = first
            .iter()
            .zip(&rated)
            .filter_map(|(f, r)| match (f, r) {
                (Some(Ok((p, _))), Some((scored, v, _))) if p.shape == shape && *v != Verdict::NotFound => scored.first().map(|x| &x.1),
                _ => None,
            })
            .collect();
        if let Some((mask, coverage)) = learn_watermark(&residuals) {
            report.watermarks.insert(shape, Watermark { coverage, learned_from: residuals.len() });
            masks.insert(shape, mask);
        }
    }

    // Pass 2: compare again without the watermark.
    report.matches = first
        .into_par_iter()
        .zip(rated)
        .zip(queries)
        .filter_map(|((f, r), q)| {
            let (probe, thumb) = match f? {
                Ok(x) => x,
                Err(e) => {
                    return Some(Match {
                        query: q.clone(),
                        verdict: Verdict::Failed,
                        best: None,
                        runner_up: None,
                        error: Some(e),
                        shape: Shape::Landscape,
                        thumb: None,
                    })
                }
            };
            let (mut scored, mut verdict, _) = match (masks.get(&probe.shape), r) {
                (Some(mask), Some((_, _, crops))) => rate(index, q, &probe, Some(mask), Some(&crops)),
                (_, r) => r?,
            };
            if verdict != Verdict::NotFound {
                let lead = scored[0].0.path.clone();
                prefer_named(&mut scored);
                if scored[0].0.path != lead && scored[0].0.similarity >= FOUND_MIN {
                    verdict = Verdict::Found;
                }
            }
            Some(to_match(q, probe.shape, thumb, &scored, verdict))
        })
        .collect();
    report
}

// ---- Collecting what was sent, and delivering what was found ----

/// The photos in `paths`: files as given, folders walked (skipping a `_bestofburst` inside).
pub fn collect_queries(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |p: PathBuf| {
        if matches!(scan::detect_file(&p), Ok(Some(_))) && seen.insert(p.to_string_lossy().to_lowercase()) {
            out.push(p);
        }
    };
    for p in paths {
        if p.is_dir() {
            let walker = walkdir::WalkDir::new(p).follow_links(false).into_iter().filter_entry(|e| {
                // Our own output folders are never input.
                !(e.file_type().is_dir() && e.depth() > 0 && {
                    let n = e.file_name().to_string_lossy();
                    n.eq_ignore_ascii_case("_bestofburst") || n.eq_ignore_ascii_case("watermark removed")
                })
            });
            let mut files: Vec<PathBuf> = walker.flatten().filter(|e| e.file_type().is_file()).map(|e| e.into_path()).collect();
            files.sort();
            files.into_iter().for_each(&mut add);
        } else if p.is_file() {
            add(p.clone());
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct FoundSummary {
    pub dir: PathBuf,
    pub written: usize,
    pub copied_fallback: bool,
    pub errors: Vec<String>,
    /// What else was made alongside (clean previews, name lists), in words.
    pub notes: Vec<String>,
}

/// Fill `<out>\found originals` with the originals of every found (and to-check) match, under
/// their own file names, and write `found.csv` beside it. Originals are linked or copied, never
/// moved or changed.
pub fn write_found(input: &Path, out: &Path, matches: &[Match], copy: bool) -> Result<FoundSummary> {
    crate::output::check_out(input, out)?;
    std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;
    let dir = out.join(FOUND_DIR);
    if dir.exists() {
        // Holds only links or copies BestOfBurst made.
        std::fs::remove_dir_all(&dir).with_context(|| format!("cannot empty {}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir)?;
    let mut summary = FoundSummary { dir: dir.clone(), ..Default::default() };
    let mut used = HashSet::new();
    let mut placed = HashSet::new();
    let mut hard = !copy;
    for rel in matches.iter().flat_map(|m| m.originals()) {
        if !placed.insert(rel.to_string()) {
            continue; // two previews of one original
        }
        let name = crate::output::unique_name(&mut used, &crate::output::file_name(rel));
        let (src, dst) = (input.join(rel), dir.join(name));
        if hard && std::fs::hard_link(&src, &dst).is_err() {
            hard = false;
            summary.copied_fallback = !copy;
        }
        if !hard {
            if let Err(e) = std::fs::copy(&src, &dst) {
                summary.errors.push(format!("{rel}: {e}"));
                continue;
            }
        }
        summary.written += 1;
    }

    let path = out.join("found.csv");
    let tmp = out.join("found.csv.tmp");
    {
        let mut w = csv::Writer::from_path(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
        w.write_record(["sent_file", "result", "original", "similarity", "runner_up", "runner_up_similarity", "error"])?;
        for m in matches {
            let sim = |c: &Option<Candidate>| c.as_ref().map(|c| format!("{:.0}", c.similarity)).unwrap_or_default();
            let shown = m.verdict != Verdict::NotFound;
            w.write_record([
                m.query.to_string_lossy().to_string(),
                m.verdict.name().to_string(),
                m.originals().join(";"),
                if shown { sim(&m.best) } else { String::new() },
                m.runner_up.as_ref().filter(|_| m.verdict == Verdict::Check).map(|c| c.path.clone()).unwrap_or_default(),
                if m.verdict == Verdict::Check { sim(&m.runner_up) } else { String::new() },
                m.error.clone().unwrap_or_default(),
            ])?;
        }
        w.flush()?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(summary)
}
