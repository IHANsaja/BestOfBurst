#![allow(dead_code)]
//! Shared helpers: synthetic images, a tiny PRNG and scratch folders.

use std::path::{Path, PathBuf};

use bestofburst::decode::RgbImage;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn gauss(&mut self) -> f64 {
        let u1 = self.uniform().max(1e-12);
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

pub fn gray(w: u32, h: u32, v: u8) -> RgbImage {
    RgbImage { width: w, height: h, rgb: vec![v; (w * h * 3) as usize] }
}

pub fn from_luma(w: u32, h: u32, f: impl Fn(u32, u32) -> f64) -> RgbImage {
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = f(x, y).round().clamp(0.0, 255.0) as u8;
            rgb.extend_from_slice(&[v, v, v]);
        }
    }
    RgbImage { width: w, height: h, rgb }
}

pub fn checkerboard(w: u32, h: u32, cell: u32) -> RgbImage {
    from_luma(w, h, |x, y| if ((x / cell) + (y / cell)) % 2 == 0 { 30.0 } else { 220.0 })
}

/// A smooth random field: random values on a coarse grid, bilinearly interpolated, plus colour.
pub fn scene(w: u32, h: u32, seed: u64) -> RgbImage {
    let mut rng = Rng::new(seed);
    let (gw, gh) = (7usize, 6usize);
    let grid: Vec<[f64; 3]> = (0..gw * gh)
        .map(|_| [rng.uniform() * 200.0 + 20.0, rng.uniform() * 200.0 + 20.0, rng.uniform() * 200.0 + 20.0])
        .collect();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f64 / (w - 1) as f64 * (gw - 1) as f64;
            let fy = y as f64 / (h - 1) as f64 * (gh - 1) as f64;
            let (x0, y0) = ((fx as usize).min(gw - 2), (fy as usize).min(gh - 2));
            let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
            for c in 0..3 {
                let v = grid[y0 * gw + x0][c] * (1.0 - tx) * (1.0 - ty)
                    + grid[y0 * gw + x0 + 1][c] * tx * (1.0 - ty)
                    + grid[(y0 + 1) * gw + x0][c] * (1.0 - tx) * ty
                    + grid[(y0 + 1) * gw + x0 + 1][c] * tx * ty;
                rgb.push(v.round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    RgbImage { width: w, height: h, rgb }
}

/// Separable Gaussian blur with sigma = radius.
pub fn blur(img: &RgbImage, sigma: f64) -> RgbImage {
    let r = (sigma * 3.0).ceil() as i64;
    let k: Vec<f64> = (-r..=r).map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp()).collect();
    let ks: f64 = k.iter().sum();
    let (w, h) = (img.width as i64, img.height as i64);
    let src: Vec<f64> = img.rgb.iter().map(|&v| v as f64).collect();
    let mut tmp = vec![0f64; src.len()];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut s = 0.0;
                for (j, kv) in k.iter().enumerate() {
                    let xx = (x + j as i64 - r).clamp(0, w - 1);
                    s += kv * src[((y * w + xx) * 3 + c) as usize];
                }
                tmp[((y * w + x) * 3 + c) as usize] = s / ks;
            }
        }
    }
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut s = 0.0;
                for (j, kv) in k.iter().enumerate() {
                    let yy = (y + j as i64 - r).clamp(0, h - 1);
                    s += kv * tmp[((yy * w + x) * 3 + c) as usize];
                }
                out[((y * w + x) * 3 + c) as usize] = (s / ks).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    RgbImage { width: img.width, height: img.height, rgb: out }
}

/// A fresh, empty scratch folder under the system temp dir.
pub fn scratch(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("bestofburst-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create scratch dir");
    p
}

pub fn encode_jpeg(img: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    enc.encode(&img.rgb, img.width, img.height, image::ExtendedColorType::Rgb8).expect("encode jpeg");
    out
}

pub fn save_jpeg(img: &RgbImage, path: &Path) {
    std::fs::write(path, encode_jpeg(img, 92)).expect("write jpeg");
}

/// Content hash of every file under `dir` (FNV-1a over bytes), keyed by relative path.
pub fn hash_tree(dir: &Path, skip: &Path) -> std::collections::BTreeMap<String, u64> {
    let mut m = std::collections::BTreeMap::new();
    for e in walkdir::WalkDir::new(dir).into_iter().flatten() {
        if e.path().starts_with(skip) || !e.file_type().is_file() {
            continue;
        }
        let data = std::fs::read(e.path()).expect("read");
        let mut h: u64 = 0xcbf29ce484222325;
        for b in &data {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        m.insert(e.path().strip_prefix(dir).unwrap_or(e.path()).to_string_lossy().to_string(), h);
    }
    m
}
