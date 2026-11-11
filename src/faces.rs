//! Faces and how open their eyes are, from Google's MediaPipe face models (the detector and
//! the 478-point landmark model of `face_landmarker.task`, 2.8 MB together, built into the
//! program and run on the processor by `tract`: no extra files, nothing leaves the computer).
//!
//! Per photo: find faces at several scales, lay a 478-point mesh on each, and measure both eyes
//! as an eye aspect ratio (lid gap over eye width: about 0.3 wide open, under 0.15 shut).
//! Whether shut eyes are a blink or a pose is decided later, in `rank`, where the other frames
//! of the burst and the other faces in the photo are known.

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tract_tflite::prelude::*;

use crate::decode::RgbImage;

static DETECTOR: &[u8] = include_bytes!("../assets/models/face_detector.tflite");
static LANDMARKS: &[u8] = include_bytes!("../assets/models/face_landmarks_detector.tflite");

const DET: usize = 128;
const MESH: usize = 256;
/// Long edge of the largest copy the detector looks at (its halves: 768, 384, 192).
const BASE_EDGE: u32 = 1536;
const DET_SCORE: f32 = 0.6;
const NMS_IOU: f32 = 0.3;
/// The mesh must agree a face is there.
const MESH_SCORE: f32 = 0.5;
/// Faces smaller than this share of the largest face are bystanders.
pub const MAIN_FACE_SHARE: f32 = 0.45;
/// At most this many faces get a mesh (largest first).
const MAX_FACES: usize = 10;

/// One face: where it is and how open each eye is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Face {
    /// Centre as 0..1 fractions of the upright photo.
    pub x: f32,
    pub y: f32,
    /// Face box long side over the image long edge.
    pub size: f32,
    /// Eye aspect ratio of the eye on the image's left and right; -1 when that eye is hidden
    /// (a face in profile).
    pub left: f32,
    pub right: f32,
}

type Plan = std::sync::Arc<TypedRunnableModel>;

struct Models {
    detector: Plan,
    mesh: Plan,
    anchors: Vec<[f32; 2]>,
}

fn load(bytes: &[u8]) -> Result<Plan> {
    let model = tract_tflite::tflite().model_for_read(&mut std::io::Cursor::new(bytes))?;
    Ok(model.into_optimized()?.into_runnable()?)
}

fn models() -> Result<&'static Models, String> {
    static M: OnceLock<Result<Models, String>> = OnceLock::new();
    M.get_or_init(|| {
        let build = || -> Result<Models> {
            Ok(Models { detector: load(DETECTOR)?, mesh: load(LANDMARKS)?, anchors: anchors() })
        };
        std::panic::catch_unwind(build)
            .unwrap_or_else(|_| Err(anyhow!("the face model crashed while loading")))
            .map_err(|e| format!("{e:#}"))
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// Load the models now (a second or two) instead of on the first photo. The error, if any, is
/// why the closed-eyes check is unavailable.
pub fn warm_up() -> Result<(), String> {
    models().map(|_| ())
}

/// BlazeFace short-range anchors: a 16 x 16 grid with 2 anchors per cell, then 8 x 8 with 6.
fn anchors() -> Vec<[f32; 2]> {
    let mut a = Vec::with_capacity(896);
    for (grid, per_cell) in [(16usize, 2usize), (8, 6)] {
        for y in 0..grid {
            for x in 0..grid {
                for _ in 0..per_cell {
                    a.push([(x as f32 + 0.5) / grid as f32, (y as f32 + 0.5) / grid as f32]);
                }
            }
        }
    }
    a
}

// ---- Small image helpers ----

/// One size of the photo, kept as stored on disk and read upright through its EXIF orientation
/// (faces on their side are missed by the detector, and turning 24 megapixels costs time).
struct Level<'a> {
    /// Stored size.
    w: usize,
    h: usize,
    rgb: std::borrow::Cow<'a, [u8]>,
    /// Pixels of this level per pixel of the original.
    scale: f32,
    /// EXIF orientation 1..8.
    orientation: u32,
}

impl Level<'_> {
    /// Upright width and height.
    fn up(&self) -> (usize, usize) {
        if self.orientation >= 5 { (self.h, self.w) } else { (self.w, self.h) }
    }

    /// Bilinear sample at (u, v) in this level's upright pixels; black outside.
    #[inline]
    fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let (w, h) = (self.w as f32, self.h as f32);
        let (x, y) = match self.orientation {
            2 => (w - u, v),
            3 => (w - u, h - v),
            4 => (u, h - v),
            5 => (v, u),
            6 => (v, h - u),
            7 => (w - v, h - u),
            8 => (w - v, u),
            _ => (u, v),
        };
        let (x, y) = (x - 0.5, y - 0.5);
        if x <= -1.0 || y <= -1.0 || x >= self.w as f32 || y >= self.h as f32 {
            return [0.0; 3];
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let px = |xi: i64, yi: i64| -> [f32; 3] {
            if xi < 0 || yi < 0 || xi >= self.w as i64 || yi >= self.h as i64 {
                return [0.0; 3];
            }
            let i = (yi as usize * self.w + xi as usize) * 3;
            [self.rgb[i] as f32, self.rgb[i + 1] as f32, self.rgb[i + 2] as f32]
        };
        let (a, b, c, d) = (px(x0 as i64, y0 as i64), px(x0 as i64 + 1, y0 as i64), px(x0 as i64, y0 as i64 + 1), px(x0 as i64 + 1, y0 as i64 + 1));
        let mut out = [0f32; 3];
        for k in 0..3 {
            out[k] = (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy;
        }
        out
    }

    /// Half size by 2 x 2 averaging.
    fn half(&self) -> Level<'static> {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let (x0, y0) = ((x * 2).min(self.w - 1), (y * 2).min(self.h - 1));
                let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
                for k in 0..3 {
                    let s = self.rgb[(y0 * self.w + x0) * 3 + k] as u32
                        + self.rgb[(y0 * self.w + x1) * 3 + k] as u32
                        + self.rgb[(y1 * self.w + x0) * 3 + k] as u32
                        + self.rgb[(y1 * self.w + x1) * 3 + k] as u32;
                    rgb[(y * w + x) * 3 + k] = ((s + 2) / 4) as u8;
                }
            }
        }
        Level { w, h, rgb: rgb.into(), scale: self.scale * w as f32 / self.w as f32, orientation: self.orientation }
    }
}

/// The photo itself, then 1,536 px and its halves down to about 192 px: largest first.
fn pyramid(img: &RgbImage, orientation: u32) -> Result<Vec<Level<'_>>> {
    let n = img.width as usize * img.height as usize * 3;
    let mut levels = vec![Level { w: img.width as usize, h: img.height as usize, rgb: (&img.rgb[..n]).into(), scale: 1.0, orientation }];
    if img.width.max(img.height) > BASE_EDGE {
        let base = crate::decode::resize_ref(img, BASE_EDGE)?;
        let scale = base.width as f32 / img.width as f32;
        levels.push(Level { w: base.width as usize, h: base.height as usize, rgb: base.rgb.into(), scale, orientation });
    }
    while levels.last().map(|l| l.w.max(l.h) > 200).unwrap_or(false) {
        let next = levels[levels.len() - 1].half();
        levels.push(next);
    }
    Ok(levels)
}

// ---- Detection ----

#[derive(Clone, Copy, Debug)]
struct Det {
    /// Box and the two eye points, in pixels of the original image.
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    eye_a: [f32; 2],
    eye_b: [f32; 2],
    score: f32,
}

impl Det {
    fn area(&self) -> f32 {
        (self.x1 - self.x0).max(0.0) * (self.y1 - self.y0).max(0.0)
    }
    fn overlap(&self, o: &Det) -> (f32, f32) {
        let w = (self.x1.min(o.x1) - self.x0.max(o.x0)).max(0.0);
        let h = (self.y1.min(o.y1) - self.y0.max(o.y0)).max(0.0);
        let inter = w * h;
        let union = self.area() + o.area() - inter;
        (if union > 0.0 { inter / union } else { 0.0 }, inter / self.area().min(o.area()).max(1e-6))
    }
}

/// Run the detector on a square window of a level (`x`, `y`, `side` in level pixels).
fn detect_window(m: &Models, level: &Level, x: f32, y: f32, side: f32, out: &mut Vec<Det>) -> Result<()> {
    let mut input = vec![0f32; DET * DET * 3];
    let step = side / DET as f32;
    for v in 0..DET {
        for u in 0..DET {
            let p = level.sample(x + (u as f32 + 0.5) * step, y + (v as f32 + 0.5) * step);
            let i = (v * DET + u) * 3;
            for k in 0..3 {
                input[i + k] = p[k] / 127.5 - 1.0;
            }
        }
    }
    let t = tract_ndarray::Array4::from_shape_vec((1, DET, DET, 3), input)?.into_tensor();
    let res = m.detector.run(tvec!(t.into()))?;
    // Two outputs: 896 x 16 box regressions and 896 x 1 scores, in either order.
    let (mut boxes, mut scores) = (None, None);
    for o in res.iter() {
        let s = o.try_as_plain_ram()?.as_slice::<f32>()?;
        if s.len() == 896 * 16 {
            boxes = Some(s);
        } else if s.len() == 896 {
            scores = Some(s);
        }
    }
    let (Some(boxes), Some(scores)) = (boxes, scores) else {
        return Err(anyhow!("unexpected face detector output"));
    };
    let to_img = |px: f32, py: f32| [(x + px * side) / level.scale, (y + py * side) / level.scale];
    for (i, a) in m.anchors.iter().enumerate() {
        let score = 1.0 / (1.0 + (-scores[i].clamp(-50.0, 50.0)).exp());
        if score < DET_SCORE {
            continue;
        }
        let r = &boxes[i * 16..i * 16 + 16];
        let (cx, cy) = (r[0] / DET as f32 + a[0], r[1] / DET as f32 + a[1]);
        let (w, h) = (r[2] / DET as f32, r[3] / DET as f32);
        let p0 = to_img(cx - w / 2.0, cy - h / 2.0);
        let p1 = to_img(cx + w / 2.0, cy + h / 2.0);
        // A face cut by the window edge is found whole in a neighbouring window.
        let edge = 0.02;
        if side < level.up().0.max(level.up().1) as f32 && (cx - w / 2.0 < -edge || cy - h / 2.0 < -edge || cx + w / 2.0 > 1.0 + edge || cy + h / 2.0 > 1.0 + edge) {
            continue;
        }
        out.push(Det {
            x0: p0[0],
            y0: p0[1],
            x1: p1[0],
            y1: p1[1],
            eye_a: to_img(r[4] / DET as f32 + a[0], r[5] / DET as f32 + a[1]),
            eye_b: to_img(r[6] / DET as f32 + a[0], r[7] / DET as f32 + a[1]),
            score,
        });
    }
    Ok(())
}

/// Window origins covering `len` with windows of `side`, overlapping by at least a quarter.
fn starts(len: usize, side: usize) -> Vec<f32> {
    if len <= side {
        return vec![0.0];
    }
    let n = ((len - side) as f32 / (side as f32 * 0.75)).ceil() as usize + 1;
    (0..n).map(|i| (len - side) as f32 * i as f32 / (n - 1) as f32).collect()
}

/// One step of the search, coarse to fine. Step 0 looks at the whole photo and at 128 px
/// windows of the 192 px copy (faces from about a tenth of the long edge); step 1 at windows
/// of the 384 px copy (from about 5%); step 2 at the 768 px copy (from about 2.5%: the faces
/// of a group photo). `None` when there is no such step.
fn detect(m: &Models, levels: &[Level], step: usize) -> Result<Option<Vec<Det>>> {
    let Some(level) = levels.iter().rev().nth(step) else { return Ok(None) };
    let (w, h) = level.up();
    if step > 2 || (step > 0 && w.max(h) < 2 * DET) {
        return Ok(None);
    }
    let mut dets = Vec::new();
    if step == 0 {
        // The whole photo, letterboxed into a square.
        let long = w.max(h) as f32;
        detect_window(m, level, (w as f32 - long) / 2.0, (h as f32 - long) / 2.0, long, &mut dets)?;
    }
    if w.min(h) >= DET {
        for y in starts(h, DET) {
            for x in starts(w, DET) {
                detect_window(m, level, x, y, DET as f32, &mut dets)?;
            }
        }
    }
    // Keep the most confident of overlapping boxes.
    dets.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Det> = Vec::new();
    for d in dets {
        if kept.iter().all(|k| {
            let (iou, contained) = k.overlap(&d);
            iou < NMS_IOU && contained < 0.6
        }) {
            kept.push(d);
        }
    }
    Ok(Some(kept))
}

// ---- Mesh and eyes ----

// MediaPipe face mesh points around each eye: corner, two upper lid, corner, two lower lid.
const EYE_A: [usize; 6] = [33, 160, 158, 133, 153, 144];
const EYE_B: [usize; 6] = [362, 385, 387, 263, 373, 380];

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Eye aspect ratio and eye width from six points.
fn eye_ratio(p: &[[f32; 2]], idx: &[usize; 6]) -> (f32, f32) {
    let width = dist(p[idx[0]], p[idx[3]]);
    if width < 1e-3 {
        return (0.0, 0.0);
    }
    ((dist(p[idx[1]], p[idx[5]]) + dist(p[idx[2]], p[idx[4]])) / (2.0 * width), width)
}

/// Lay the mesh on one detected face and measure its eyes.
fn mesh_face(m: &Models, levels: &[Level], d: &Det) -> Result<Option<Face>> {
    // The face square: 1.5 x the box, turned so the eyes are level (as MediaPipe crops it).
    let (cx, cy) = ((d.x0 + d.x1) / 2.0, (d.y0 + d.y1) / 2.0);
    let side = (d.x1 - d.x0).max(d.y1 - d.y0) * 1.5;
    let angle = (d.eye_b[1] - d.eye_a[1]).atan2(d.eye_b[0] - d.eye_a[0]);
    let (sin, cos) = angle.sin_cos();
    // Sample from the level where the square is closest to (and at least) 256 px.
    let level = levels.iter().rev().find(|l| side * l.scale >= MESH as f32).unwrap_or(&levels[0]);
    let mut input = vec![0f32; MESH * MESH * 3];
    for v in 0..MESH {
        for u in 0..MESH {
            let (du, dv) = (((u as f32 + 0.5) / MESH as f32 - 0.5) * side, ((v as f32 + 0.5) / MESH as f32 - 0.5) * side);
            let (sx, sy) = (cx + du * cos - dv * sin, cy + du * sin + dv * cos);
            let p = level.sample(sx * level.scale, sy * level.scale);
            let i = (v * MESH + u) * 3;
            for k in 0..3 {
                input[i + k] = p[k] / 255.0;
            }
        }
    }
    let t = tract_ndarray::Array4::from_shape_vec((1, MESH, MESH, 3), input)?.into_tensor();
    let res = m.mesh.run(tvec!(t.into()))?;
    let mut points: Option<Vec<[f32; 2]>> = None;
    let mut presence: Option<f32> = None;
    for o in res.iter() {
        let s = o.try_as_plain_ram()?.as_slice::<f32>()?;
        if s.len() == 478 * 3 {
            points = Some(s.chunks_exact(3).map(|p| [p[0], p[1]]).collect());
        } else if s.len() == 1 && presence.is_none() {
            presence = Some(1.0 / (1.0 + (-s[0].clamp(-50.0, 50.0)).exp()));
        }
    }
    let points = points.ok_or_else(|| anyhow!("unexpected face mesh output"))?;
    if presence.unwrap_or(1.0) < MESH_SCORE {
        return Ok(None);
    }
    let (ra, wa) = eye_ratio(&points, &EYE_A);
    let (rb, wb) = eye_ratio(&points, &EYE_B);
    // In profile the far eye is squeezed to a sliver and its ratio means nothing.
    let hidden = |w: f32, other: f32| w < 0.45 * other;
    let (ra, rb) = (if hidden(wa, wb) { -1.0 } else { ra }, if hidden(wb, wa) { -1.0 } else { rb });
    // Which eye is on the image's left, after the crop's turn.
    let mid = |idx: &[usize; 6]| (points[idx[0]][0] + points[idx[3]][0]) * cos - (points[idx[0]][1] + points[idx[3]][1]) * sin;
    let (left, right) = if mid(&EYE_A) <= mid(&EYE_B) { (ra, rb) } else { (rb, ra) };
    let round = |v: f32| (v * 1000.0).round() / 1000.0;
    let (w, h) = levels[0].up();
    Ok(Some(Face {
        x: round(cx / w as f32),
        y: round(cy / h as f32),
        size: round((d.x1 - d.x0).max(d.y1 - d.y0) / w.max(h) as f32),
        left: round(left),
        right: round(right),
    }))
}

/// Every face in the photo with its eye openness, largest first. An empty list means no face.
/// `orientation` is the photo's EXIF orientation (1 when unknown): faces are looked for upright.
pub fn find_faces(img: &RgbImage, orientation: u32) -> Result<Vec<Face>> {
    let m = models().map_err(|e| anyhow!(e))?;
    let levels = pyramid(img, orientation)?;
    // Coarse to fine, stopping at the first step that finds a real face: once a large face is
    // in the photo, much smaller ones are bystanders, and every window costs time.
    let mut step = 0;
    while let Some(mut dets) = detect(m, &levels, step)? {
        dets.sort_by(|a, b| b.area().total_cmp(&a.area()));
        let largest = dets.first().map(|d| d.area().sqrt()).unwrap_or(0.0);
        let mut faces = Vec::new();
        for d in dets.iter().take(MAX_FACES) {
            if d.area().sqrt() < 0.3 * largest {
                break;
            }
            if let Some(f) = mesh_face(m, &levels, d)? {
                faces.push(f);
            }
        }
        if !faces.is_empty() {
            return Ok(faces);
        }
        step += 1;
    }
    Ok(Vec::new())
}
