//! AI clean-up of a removed watermark's edge (used by `unmark`).
//!
//! Un-blending gets the inside of a see-through mark right, but previews are JPEGs: the mark's
//! edge is smeared over 8 x 8 blocks, colour is kept at half resolution and a ringing halo sits
//! just outside the mark. What is left is a thin pale outline. Here that outline, and the
//! parts of the mark so opaque that un-blending only boosts noise, are painted over by LaMa
//! (an inpainting network), working on the already un-blended photo so it sees clean
//! surroundings.
//!
//! LaMa takes a fixed 512 x 512 input. The photo is never shrunk to fit: the thin outline would
//! vanish into one blurry pixel. Instead the areas to paint are covered by overlapping 512 px
//! tiles at the photo's own resolution, and only the painted pixels are put back.
//!
//! The model (`lama_fp32.onnx`, about 200 MB, Apache-2.0) is not built in. It is looked for in a
//! `models` folder beside the program and in `%LOCALAPPDATA%\BestOfBurst\models\`
//! (`scripts/fetch-lama.ps1` puts it there), and its SHA-256 is checked before use. Without it
//! everything works as before, with the classical fill.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use tract_onnx::prelude::*;

use crate::decode::RgbImage;
use crate::unmark::morph_n;

/// The model file, as published at huggingface.co/Carve/LaMa-ONNX.
pub const MODEL_FILE: &str = "lama_fp32.onnx";
/// Its size in bytes and SHA-256 (from the Git LFS pointer of that repository).
pub const MODEL_BYTES: u64 = 208_044_816;
pub const MODEL_SHA256: &str = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6";
/// Where to get it.
pub const MODEL_URL: &str = "https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx";

// The model's interface (checked by loading it; the model card does not spell it out):
// - input `image`: float32 [1, 3, 512, 512], RGB, values 0..1;
// - input `mask`: float32 [1, 1, 512, 512], 1 = paint this pixel, 0 = keep it;
// - output `output`: float32 [1, 3, 512, 512], RGB, values 0..255, with the kept pixels
//   returned as given.
// It is opset 17. Its Fourier units are written with ordinary ops (Einsum, MatMul, Sin, Cos),
// not ONNX DFT, so tract runs it; tract's output matches onnxruntime's to within 0.001 of 255.

/// LaMa's fixed input size.
pub const TILE: usize = 512;
/// How much neighbouring tiles share.
pub const OVERLAP: usize = 64;
const STRIDE: usize = TILE - OVERLAP;
/// A JPEG block, the distance the mark's edge is smeared over in a JPEG preview.
const JPEG_BLOCK: usize = 8;
/// Never grow the edge band by less than this.
const MIN_JPEG_REACH: usize = 6;
/// How far the painted band reaches either side of the mark's edge: at least one JPEG block.
pub const JPEG_REACH: usize = if JPEG_BLOCK > MIN_JPEG_REACH { JPEG_BLOCK } else { MIN_JPEG_REACH };
/// Where the mark is at least this opaque, un-blending only boosts noise: paint it instead.
pub const STRONG: f32 = 0.6;
/// The painted pixels fade into the classical result over this many pixels at the band's rim.
const FEATHER: usize = 3;
/// Tiles run at the same time (each holds about 300 MB while it runs).
const TILE_THREADS: usize = 4;
/// How much further the band grows on the second try when the outline still shows.
pub const EXTRA_REACH: usize = JPEG_BLOCK;

type Plan = std::sync::Arc<TypedRunnableModel>;
/// A painted tile: its top-left corner and its RGB output, planar, 0..255.
type Painted = ((usize, usize), Vec<f32>);

/// The loaded inpainting model.
pub struct Lama {
    plan: Plan,
}

/// Where the model is looked for, in order.
pub fn model_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(dir.join("models").join(MODEL_FILE));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(local).join("BestOfBurst").join("models").join(MODEL_FILE));
    }
    v
}

/// The model file, when one of the right size is in place (quick: the hash is checked on load).
pub fn find_model() -> Option<PathBuf> {
    model_paths().into_iter().find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() == MODEL_BYTES))
}

fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

impl Lama {
    /// Check the file's hash, then load it (about 20 seconds).
    pub fn load(path: &Path) -> Result<Lama> {
        let hash = sha256_file(path)?;
        if hash != MODEL_SHA256 {
            bail!("{} is not the expected model (its SHA-256 is {hash})", path.display());
        }
        let model = tract_onnx::onnx()
            .model_for_path(path)?
            .with_input_fact(0, f32::fact([1, 3, TILE, TILE]).into())?
            .with_input_fact(1, f32::fact([1, 1, TILE, TILE]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Lama { plan: model })
    }

    /// Paint one tile: `image` is RGB 0..1 and `mask` 1 where to paint, both planar 512 x 512.
    /// Returns RGB 0..255, planar.
    pub fn run(&self, image: Vec<f32>, mask: Vec<f32>) -> Result<Vec<f32>> {
        let img = tract_ndarray::Array4::from_shape_vec((1, 3, TILE, TILE), image)?.into_tensor();
        let msk = tract_ndarray::Array4::from_shape_vec((1, 1, TILE, TILE), mask)?.into_tensor();
        let out = self.plan.run(tvec!(img.into(), msk.into()))?;
        let s = out.first().context("the model gave no output")?.try_as_plain_ram()?.as_slice::<f32>()?.to_vec();
        if s.len() != 3 * TILE * TILE {
            bail!("the model gave an output of an unexpected size");
        }
        Ok(s)
    }
}

/// The model, loaded once and kept: `Ok(None)` when it is not installed, `Err` when it is
/// there but cannot be used (wrong file, failed to load).
pub fn shared() -> Result<Option<&'static Lama>, String> {
    static M: OnceLock<Lama> = OnceLock::new();
    if let Some(m) = M.get() {
        return Ok(Some(m));
    }
    let Some(path) = find_model() else { return Ok(None) };
    let lama = std::panic::catch_unwind(|| Lama::load(&path))
        .unwrap_or_else(|_| Err(anyhow!("the AI model crashed while loading")))
        .map_err(|e| format!("{e:#}"))?;
    Ok(Some(M.get_or_init(|| lama)))
}

/// Where the un-blended photo can be trusted for its smooth tone: away (by `CLEAR` pixels)
/// from where the mark only partly covers, which is where JPEG smears its edge.
pub fn trusted(cover: &[f32], w: usize, h: usize) -> Vec<bool> {
    let partial: Vec<bool> = cover.iter().map(|&k| k > 0.01 && k < 0.99).collect();
    morph_n(&partial, w, h, true, CLEAR).into_iter().map(|b| !b).collect()
}

/// How far from the mark's partly covered edge the photo's tone is trusted.
const CLEAR: usize = 3;

/// What the AI paints on one photo, at the photo's resolution: the mark's edge band (where it
/// partly covers, `cover` between 0.01 and 0.99) grown by `reach` or at least one JPEG block,
/// plus every pixel where the mark is so opaque (`cover * alpha_max > STRONG`) that
/// un-blending only boosts noise.
pub fn ai_mask(cover: &[f32], w: usize, h: usize, alpha_max: f32, reach: usize) -> Vec<bool> {
    let partial: Vec<bool> = cover.iter().map(|&k| k > 0.01 && k < 0.99).collect();
    let mut band = morph_n(&partial, w, h, true, reach.max(JPEG_REACH));
    for (b, &k) in band.iter_mut().zip(cover) {
        if k * alpha_max > STRONG {
            *b = true;
        }
    }
    band
}

/// The top-left corners of 512 px tiles along one axis covering `lo..=hi` inside `0..n`, each
/// sharing at least `OVERLAP` with the next and starting a little before `lo` for context.
fn starts(lo: usize, hi: usize, n: usize) -> Vec<usize> {
    if n <= TILE {
        return vec![0];
    }
    let last = n - TILE;
    let mut s = lo.saturating_sub(OVERLAP / 2).min(last);
    let mut v = vec![s];
    while s + TILE <= hi && s < last {
        s = (s + STRIDE).min(last);
        v.push(s);
    }
    v
}

/// Tiles (top-left corners) that cover every pixel of `mask`: for each connected part, a grid
/// of overlapping tiles over its extent, keeping those that hold some of it. When the photo is
/// narrower or shorter than a tile, the tile starts at 0 and is filled out by reflection.
pub fn plan_tiles(mask: &[bool], w: usize, h: usize) -> Vec<(usize, usize)> {
    let mut label = vec![0u32; w * h];
    let mut stack = Vec::new();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut next = 0u32;
    for start in 0..w * h {
        if !mask[start] || label[start] != 0 {
            continue;
        }
        next += 1;
        label[start] = next;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            for yy in y.saturating_sub(1)..(y + 2).min(h) {
                for xx in x.saturating_sub(1)..(x + 2).min(w) {
                    let j = yy * w + xx;
                    if mask[j] && label[j] == 0 {
                        label[j] = next;
                        stack.push(j);
                    }
                }
            }
        }
        for &ty in &starts(y0, y1, h) {
            for &tx in &starts(x0, x1, w) {
                if out.contains(&(tx, ty)) {
                    continue;
                }
                let holds = (ty..(ty + TILE).min(h)).any(|y| (tx..(tx + TILE).min(w)).any(|x| label[y * w + x] == next));
                if holds {
                    out.push((tx, ty));
                }
            }
        }
    }
    out
}

/// Mirror an index into `0..n` (edge pixel not repeated).
fn reflect(i: usize, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let m = i % period;
    if m < n {
        m
    } else {
        period - m
    }
}

/// One tile's input: planar RGB 0..1 from `rgb` (interleaved 0..255) and the mask, reflected
/// where the tile runs past the photo.
fn tile_input(rgb: &[f32], mask: &[bool], w: usize, h: usize, tx: usize, ty: usize) -> (Vec<f32>, Vec<f32>) {
    let mut img = vec![0f32; 3 * TILE * TILE];
    let mut msk = vec![0f32; TILE * TILE];
    for y in 0..TILE {
        let sy = reflect(ty + y, h);
        for x in 0..TILE {
            let sx = reflect(tx + x, w);
            let i = sy * w + sx;
            let o = y * TILE + x;
            for c in 0..3 {
                img[c * TILE * TILE + o] = rgb[i * 3 + c] / 255.0;
            }
            msk[o] = if mask[i] { 1.0 } else { 0.0 };
        }
    }
    (img, msk)
}

/// How much a tile's pixel counts where tiles overlap: falls off towards the tile's edges.
fn tile_weight(x: usize, y: usize) -> f32 {
    let d = x.min(y).min(TILE - 1 - x).min(TILE - 1 - y);
    ((d + 1) as f32 / (OVERLAP / 2) as f32).min(1.0)
}

/// Sum over a (2r+1) square of `v` (`bw` x `bh`, `ch` values per cell), edges using what is there.
fn box_mean(v: &[f32], bw: usize, bh: usize, ch: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; v.len()];
    for y in 0..bh {
        for c in 0..ch {
            let mut run = 0f32;
            let at = |x: usize| v[(y * bw + x) * ch + c];
            for x in 0..r.min(bw) {
                run += at(x);
            }
            for x in 0..bw {
                if x + r < bw {
                    run += at(x + r);
                }
                if x > r {
                    run -= at(x - r - 1);
                }
                tmp[(y * bw + x) * ch + c] = run;
            }
        }
    }
    let mut out = vec![0f32; v.len()];
    for x in 0..bw {
        for c in 0..ch {
            let mut run = 0f32;
            let at = |y: usize| tmp[(y * bw + x) * ch + c];
            for y in 0..r.min(bh) {
                run += at(y);
            }
            for y in 0..bh {
                if y + r < bh {
                    run += at(y + r);
                }
                if y > r {
                    run -= at(y - r - 1);
                }
                out[(y * bw + x) * ch + c] = run;
            }
        }
    }
    out
}

/// How far around (in pixels) the painted area takes its tone from the photo.
const TONE_REACH: usize = 12;

/// Give the painted pixels the photo's own tone: LaMa supplies the detail (free of the
/// outline), while the smooth level comes from the un-blended photo where it can be trusted
/// (`trust`: where the mark covered fully or not at all, away from its smeared edge). Only
/// pixels in `have` change.
fn match_tone(ai: &mut [f32], have: &[bool], unblended: &[f32], trust: &[bool], w: usize, h: usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, _) in have.iter().enumerate().filter(|(_, &m)| m) {
        let (x, y) = (i % w, i / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    if x0 > x1 {
        return;
    }
    let pad = 2 * TONE_REACH;
    let (x0, y0, x1, y1) = (x0.saturating_sub(pad), y0.saturating_sub(pad), (x1 + pad).min(w - 1), (y1 + pad).min(h - 1));
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    // Differences (photo minus paint) at trusted painted pixels, and how many there are.
    let mut d = vec![0f32; bw * bh * 3];
    let mut n = vec![0f32; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let i = (y0 + by) * w + x0 + bx;
            if have[i] && trust[i] {
                for c in 0..3 {
                    d[(by * bw + bx) * 3 + c] = unblended[i * 3 + c] - ai[i * 3 + c];
                }
                n[by * bw + bx] = 1.0;
            }
        }
    }
    let r = TONE_REACH / 2;
    let (d, n) = (box_mean(&box_mean(&d, bw, bh, 3, r), bw, bh, 3, r), box_mean(&box_mean(&n, bw, bh, 1, r), bw, bh, 1, r));
    for by in 0..bh {
        for bx in 0..bw {
            let i = (y0 + by) * w + x0 + bx;
            let k = n[by * bw + bx];
            if have[i] && k > 0.5 {
                for c in 0..3 {
                    ai[i * 3 + c] += d[(by * bw + bx) * 3 + c] / k;
                }
            }
        }
    }
}

/// Put painted tiles back into `base` (interleaved RGB): only pixels inside `mask` change.
/// Overlapping tiles are averaged with `tile_weight`; with `tone` (the un-blended photo and
/// where it can be trusted) the paint takes the photo's smooth tone (see `match_tone`); and the
/// painted pixels fade into `base` over `FEATHER` pixels at the mask's rim.
pub fn paste(base: &RgbImage, mask: &[bool], tiles: &[Painted], tone: Option<(&[f32], &[bool])>) -> RgbImage {
    let (w, h) = (base.width as usize, base.height as usize);
    let mut acc = vec![0f32; w * h * 3];
    let mut wsum = vec![0f32; w * h];
    for ((tx, ty), out) in tiles {
        for y in 0..TILE {
            let iy = ty + y;
            if iy >= h {
                break;
            }
            for x in 0..TILE {
                let ix = tx + x;
                if ix >= w {
                    break;
                }
                let i = iy * w + ix;
                if !mask[i] {
                    continue;
                }
                let wt = tile_weight(x, y);
                let o = y * TILE + x;
                for c in 0..3 {
                    acc[i * 3 + c] += wt * out[c * TILE * TILE + o];
                }
                wsum[i] += wt;
            }
        }
    }
    let have: Vec<bool> = (0..w * h).map(|i| mask[i] && wsum[i] > 0.0).collect();
    let mut ai: Vec<f32> = (0..w * h * 3).map(|j| if have[j / 3] { acc[j] / wsum[j / 3] } else { 0.0 }).collect();
    if let Some((unblended, trust)) = tone {
        match_tone(&mut ai, &have, unblended, trust, w, h);
    }
    // Depth into the mask, 1..=FEATHER, for the fade.
    let mut depth = vec![0u8; w * h];
    let mut inner = mask.to_vec();
    for d in 1..=FEATHER as u8 {
        let next = morph_n(&inner, w, h, false, 1);
        for i in 0..w * h {
            if inner[i] && (!next[i] || d == FEATHER as u8) && depth[i] == 0 {
                depth[i] = d;
            }
        }
        inner = next;
    }
    let mut rgb = base.rgb.clone();
    rgb.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
        if !have[i] {
            return;
        }
        let f = depth[i].max(1) as f32 / FEATHER as f32;
        for c in 0..3 {
            px[c] = (f * ai[i * 3 + c] + (1.0 - f) * px[c] as f32).round().clamp(0.0, 255.0) as u8;
        }
    });
    RgbImage { width: base.width, height: base.height, rgb }
}

/// Paint `mask` on a photo: LaMa sees `unblended` (interleaved RGB 0..255, the mark already
/// lifted) and the result is pasted over `base`, taking its smooth tone from `unblended` where
/// `trust` says it is reliable. `None` when stopped.
pub fn inpaint(lama: &Lama, unblended: &[f32], trust: &[bool], base: &RgbImage, mask: &[bool], stop: &AtomicBool) -> Result<Option<RgbImage>> {
    let (w, h) = (base.width as usize, base.height as usize);
    let tiles = plan_tiles(mask, w, h);
    if tiles.is_empty() {
        return Ok(Some(base.clone()));
    }
    let pool = rayon::ThreadPoolBuilder::new().num_threads(TILE_THREADS).build().context("cannot start worker threads")?;
    let done: Vec<Option<Result<Painted>>> = pool.install(|| {
        tiles
            .par_iter()
            .map(|&(tx, ty)| {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                let (img, msk) = tile_input(unblended, mask, w, h, tx, ty);
                Some(lama.run(img, msk).map(|o| ((tx, ty), o)))
            })
            .collect()
    });
    if stop.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let mut painted = Vec::with_capacity(done.len());
    for t in done.into_iter().flatten() {
        painted.push(t?);
    }
    Ok(Some(paste(base, mask, &painted, Some((unblended, trust)))))
}

/// How the AI edge clean-up went on one photo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiEdge {
    /// The AI result was kept.
    Used,
    /// The classical result was kept, and why.
    FellBack(String),
}

/// How much of the mark's outline still shows on a cleaned photo.
#[derive(Clone, Copy, Debug)]
pub struct Outline {
    /// Line and step contrast right on the outline (grey levels).
    pub at_mark: f32,
    /// The same, measured on parallel lines a little away from it: the photo's own texture.
    pub nearby: f32,
}

impl Outline {
    /// The outline stands out from what is around it.
    pub fn visible(&self) -> bool {
        self.at_mark > 1.0 && self.at_mark > 1.5 * self.nearby
    }
}

fn luma_at(img: &RgbImage, x: f32, y: f32) -> f32 {
    let (w, h) = (img.width as usize, img.height as usize);
    if !(x >= 0.0 && y >= 0.0 && x <= (w - 1) as f32 && y <= (h - 1) as f32) {
        return f32::NAN;
    }
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let l = |xx: usize, yy: usize| {
        let i = (yy * w + xx) * 3;
        (img.rgb[i] as f32 + img.rgb[i + 1] as f32 + img.rgb[i + 2] as f32) / 3.0
    };
    (l(x0, y0) * (1.0 - tx) + l(x1, y0) * tx) * (1.0 - ty) + (l(x0, y1) * (1.0 - tx) + l(x1, y1) * tx) * ty
}

/// The typical signed contrast of a thin line, and of a step, at distance `o` along each
/// point's outward normal. A leftover outline has the same sign all the way round; the
/// photo's own detail averages out.
fn contrast_at(img: &RgbImage, pts: &[[f32; 4]], o: f32) -> f32 {
    let (mut line, mut step) = (Vec::new(), Vec::new());
    for p in pts {
        let at = |d: f32| luma_at(img, p[0] + p[2] * d, p[1] + p[3] * d);
        let l = at(o) - 0.5 * (at(o - 3.0) + at(o + 3.0));
        let s = at(o + 1.5) - at(o - 1.5);
        if l.is_finite() && s.is_finite() {
            line.push(l);
            step.push(s);
        }
    }
    let med = |v: &mut Vec<f32>| {
        if v.is_empty() {
            return 0.0;
        }
        let m = v.len() / 2;
        v.select_nth_unstable_by(m, |a, b| a.total_cmp(b));
        v[m]
    };
    med(&mut line).abs().max(med(&mut step).abs())
}

/// Measure the outline left on `img`. `pts` are points on the mark's edge in photo pixels with
/// their outward unit normal (x, y, nx, ny); `away` is how far out the comparison lines are
/// (beyond anything that was painted).
pub fn outline_score(img: &RgbImage, pts: &[[f32; 4]], away: f32) -> Outline {
    let at_mark = [-3.0f32, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0].iter().map(|&o| contrast_at(img, pts, o)).fold(0.0, f32::max);
    let nearby = [-away - 4.0, -away, away, away + 4.0].iter().map(|&o| contrast_at(img, pts, o)).fold(0.0, f32::max);
    Outline { at_mark, nearby }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc_cover(w: usize, h: usize, cx: f32, cy: f32, r: f32) -> Vec<f32> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                (r + 0.5 - d).clamp(0.0, 1.0)
            })
            .collect()
    }

    #[test]
    fn mask_is_the_grown_edge_band() {
        let (w, h) = (120, 100);
        let cover = disc_cover(w, h, 60.0, 50.0, 25.0);
        let m = ai_mask(&cover, w, h, 0.35, 2);
        // The rim of the disc is in, grown by at least one JPEG block both ways.
        assert!(m[50 * w + 60 + 25]);
        assert!(m[50 * w + 60 + 25 + JPEG_REACH]);
        assert!(m[50 * w + 60 + 25 - JPEG_REACH]);
        // Well inside (see-through mark, nothing strong) and well outside are not.
        assert!(!m[50 * w + 60]);
        assert!(!m[50 * w + 60 + 25 + JPEG_REACH + 3]);
        assert!(!m[5 * w + 5]);
    }

    #[test]
    fn mask_takes_strong_areas_whole() {
        let (w, h) = (120, 100);
        let cover = disc_cover(w, h, 60.0, 50.0, 25.0);
        let m = ai_mask(&cover, w, h, 0.8, 2);
        assert!(m[50 * w + 60], "an opaque mark's inside is painted");
        let none = ai_mask(&vec![0.0; w * h], w, h, 0.8, 2);
        assert!(none.iter().all(|&b| !b));
    }

    #[test]
    fn reach_never_below_a_jpeg_block() {
        const { assert!(JPEG_REACH >= MIN_JPEG_REACH && JPEG_REACH >= JPEG_BLOCK) };
        let (w, h) = (80, 80);
        let cover = disc_cover(w, h, 40.0, 40.0, 15.0);
        let a = ai_mask(&cover, w, h, 0.3, 1);
        let b = ai_mask(&cover, w, h, 0.3, JPEG_REACH);
        assert_eq!(a, b, "a small reach is raised to the JPEG reach");
    }

    fn covered(tiles: &[(usize, usize)], w: usize, h: usize, x: usize, y: usize) -> bool {
        tiles.iter().any(|&(tx, ty)| x >= tx && x < (tx + TILE).min(w) && y >= ty && y < (ty + TILE).min(h))
    }

    #[test]
    fn tiles_cover_every_masked_pixel_inside_the_photo() {
        let (w, h) = (1700, 1100);
        let mut mask = vec![false; w * h];
        // A long thin outline across the photo, a blob by the corner, and one stray pixel.
        for x in 40..1660 {
            mask[300 * w + x] = true;
            mask[301 * w + x] = true;
        }
        for y in 1050..1100 {
            for x in 1650..1700 {
                mask[y * w + x] = true;
            }
        }
        mask[700 * w + 900] = true;
        let tiles = plan_tiles(&mask, w, h);
        for &(tx, ty) in &tiles {
            assert!(tx + TILE <= w && ty + TILE <= h, "tile ({tx}, {ty}) is clamped inside the photo");
        }
        for (i, _) in mask.iter().enumerate().filter(|(_, &m)| m) {
            assert!(covered(&tiles, w, h, i % w, i / w), "pixel {} {} not covered", i % w, i / w);
        }
        // Tiles along the outline share at least OVERLAP.
        let mut row: Vec<usize> = tiles.iter().filter(|t| t.1 == tiles.iter().map(|t| t.1).min().unwrap_or(0)).map(|t| t.0).collect();
        row.sort();
        for p in row.windows(2) {
            assert!(p[0] + TILE >= p[1] + OVERLAP, "neighbours {} and {} overlap by at least {OVERLAP}", p[0], p[1]);
        }
        // Nothing is tiled where there is nothing to paint.
        assert!(tiles.iter().all(|&(tx, ty)| (ty..ty + TILE).any(|y| (tx..tx + TILE).any(|x| mask[y * w + x]))));
    }

    #[test]
    fn small_photos_get_one_reflected_tile() {
        let (w, h) = (300, 200);
        let mut mask = vec![false; w * h];
        mask[100 * w + 150] = true;
        let tiles = plan_tiles(&mask, w, h);
        assert_eq!(tiles, vec![(0, 0)]);
        let rgb: Vec<f32> = (0..w * h * 3).map(|i| (i % 251) as f32).collect();
        let (img, msk) = tile_input(&rgb, &mask, w, h, 0, 0);
        // Inside the photo the tile is the photo; past it, a mirror image.
        assert_eq!(img[0], rgb[0] / 255.0);
        assert_eq!(img[TILE + 10], rgb[(w + 10) * 3] / 255.0);
        assert_eq!(img[w], rgb[(w - 2) * 3] / 255.0);
        assert_eq!(msk[100 * TILE + 150], 1.0);
        assert_eq!(msk[100 * TILE + 2 * (w - 1) - 150], 1.0, "the mask is mirrored too");
    }

    #[test]
    fn paste_changes_nothing_outside_the_mask() {
        let (w, h) = (700, 600);
        let base = RgbImage { width: w as u32, height: h as u32, rgb: (0..w * h * 3).map(|i| (i * 7 % 256) as u8).collect() };
        let mut mask = vec![false; w * h];
        for y in 200..260 {
            for x in 100..650 {
                mask[y * w + x] = true;
            }
        }
        let tiles = plan_tiles(&mask, w, h);
        // "Painted" tiles of pure white.
        let painted: Vec<Painted> = tiles.iter().map(|&t| (t, vec![255.0; 3 * TILE * TILE])).collect();
        let out = paste(&base, &mask, &painted, None);
        // With the tone taken from a photo, still nothing outside the mask changes.
        let trust = vec![true; w * h];
        let unblended: Vec<f32> = base.rgb.iter().map(|&v| v as f32).collect();
        let toned = paste(&base, &mask, &painted, Some((&unblended, &trust)));
        for (i, _) in mask.iter().enumerate().filter(|(_, &m)| !m) {
            assert_eq!(&toned.rgb[i * 3..i * 3 + 3], &base.rgb[i * 3..i * 3 + 3], "pixel {i} outside the mask changed");
        }
        for (i, _) in mask.iter().enumerate().filter(|(_, &m)| !m) {
            assert_eq!(&out.rgb[i * 3..i * 3 + 3], &base.rgb[i * 3..i * 3 + 3], "pixel {i} outside the mask changed");
        }
        // Deep inside the mask the paint is used as is; at the rim it is faded in.
        let deep = 230 * w + 400;
        assert_eq!(&out.rgb[deep * 3..deep * 3 + 3], &[255, 255, 255]);
        let rim = 200 * w + 400;
        assert!(out.rgb[rim * 3] < 255 || base.rgb[rim * 3] == 255);
    }

    #[test]
    fn overlap_weights_fall_off_towards_tile_edges() {
        assert!(tile_weight(0, 200) < tile_weight(10, 200));
        assert!(tile_weight(TILE - 1, 200) < tile_weight(TILE - 20, 200));
        assert_eq!(tile_weight(256, 256), 1.0);
        assert!(tile_weight(0, 0) > 0.0);
    }

    #[test]
    fn outline_measure_sees_a_pale_ring() {
        let (w, h) = (300, 300);
        let mut flat = RgbImage { width: w as u32, height: h as u32, rgb: vec![60; w * h * 3] };
        let pts: Vec<[f32; 4]> = (0..360)
            .map(|a| {
                let t = (a as f32).to_radians();
                [150.0 + 80.0 * t.cos(), 150.0 + 80.0 * t.sin(), t.cos(), t.sin()]
            })
            .collect();
        assert!(!outline_score(&flat, &pts, 12.0).visible());
        for i in 0..w * h {
            let (x, y) = ((i % w) as f32 - 150.0, (i / w) as f32 - 150.0);
            if ((x * x + y * y).sqrt() - 80.0).abs() < 1.0 {
                flat.rgb[i * 3..i * 3 + 3].copy_from_slice(&[75, 75, 75]);
            }
        }
        assert!(outline_score(&flat, &pts, 12.0).visible());
    }
}
