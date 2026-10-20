//! Eight metrics, per-tile sharpness, a 64-bit dHash and the capture time of one image.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::decode::RgbImage;
use crate::scan::Format;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub sharpness: f32,
    pub tenengrad: f32,
    pub mean_luma: f32,
    pub clip_shadows: f32,
    pub clip_highlights: f32,
    pub contrast: f32,
    pub noise: f32,
    pub colorfulness: f32,
}

const T: usize = 8;
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Raw per-tile sums of one image: what both the CPU and the GPU path produce, so the
/// finishing math (sorting, medians, the formulas) is shared and identical.
#[derive(Clone, Debug)]
pub struct TileSums {
    /// Per tile, over interior pixels: 0 count, 1 luma-128, 2 (luma-128)^2, 3 lap, 4 lap^2,
    /// 5 sobel energy, 6 |noise kernel|, 7 rg, 8 rg^2, 9 yb, 10 yb^2.
    pub tiles: [[f64; 11]; 64],
    /// Pixels with luma under 4.5, and with any channel >= 251, over all pixels.
    pub lo: u64,
    pub hi: u64,
    pub n_all: f64,
}

/// dHash grid: summed luma and pixel count per cell of a 9 x 8 grid.
#[derive(Clone, Debug)]
pub struct HashCells {
    pub sums: [[f64; 9]; 8],
    pub counts: [[f64; 9]; 8],
}

/// One pass over an 8 x 8 tile grid. Returns the metrics and the Laplacian variance of every
/// tile in row-major order (0 for an empty tile).
pub fn analyze(img: &RgbImage) -> (Metrics, [f32; 64]) {
    match tile_sums(img) {
        Some(s) => finish(&s),
        None => (Metrics::default(), [0.0; 64]),
    }
}

/// The CPU pass that fills [`TileSums`].
pub fn tile_sums(img: &RgbImage) -> Option<TileSums> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 3 || h < 3 {
        return None;
    }
    let luma: Vec<f32> = img
        .rgb
        .chunks_exact(3)
        .take(w * h)
        .map(|p| LUMA[0] * p[0] as f32 + LUMA[1] * p[1] as f32 + LUMA[2] * p[2] as f32)
        .collect();

    // Clipping over all pixels
    let (mut lo, mut hi) = (0u64, 0u64);
    for (i, p) in img.rgb.chunks_exact(3).take(w * h).enumerate() {
        if luma[i] < 4.5 {
            lo += 1;
        }
        if p[0].max(p[1]).max(p[2]) >= 251 {
            hi += 1;
        }
    }

    let mut tiles = [[0f64; 11]; T * T];
    for y in 1..h - 1 {
        let ty = y * T / h;
        let (up, row, down) = ((y - 1) * w, y * w, (y + 1) * w);
        // Accumulate a row in f32 per tile column, then fold into f64 to keep precision.
        let mut acc = [[0f32; 11]; T];
        for x in 1..w - 1 {
            let tx = x * T / w;
            let i = (row + x) * 3;
            let (r, g, b) = (img.rgb[i] as f32, img.rgb[i + 1] as f32, img.rgb[i + 2] as f32);
            let c = luma[row + x];
            let (n, s, wv, e) = (luma[up + x], luma[down + x], luma[row + x - 1], luma[row + x + 1]);
            let (nw, ne, sw, se) = (luma[up + x - 1], luma[up + x + 1], luma[down + x - 1], luma[down + x + 1]);
            let lap = n + s + wv + e - 4.0 * c;
            let gx = (ne + 2.0 * e + se) - (nw + 2.0 * wv + sw);
            let gy = (sw + 2.0 * s + se) - (nw + 2.0 * n + ne);
            let nz = 4.0 * c - 2.0 * (n + s + wv + e) + (nw + ne + sw + se);
            let (rg, yb, lc) = (r - g, 0.5 * (r + g) - b, c - 128.0);
            let v = [
                1.0,
                lc,
                lc * lc,
                lap,
                lap * lap,
                gx * gx + gy * gy,
                nz.abs(),
                rg,
                rg * rg,
                yb,
                yb * yb,
            ];
            let t = &mut acc[tx];
            for k in 0..11 {
                t[k] += v[k];
            }
        }
        for tx in 0..T {
            let t = &mut tiles[ty * T + tx];
            for k in 0..11 {
                t[k] += acc[tx][k] as f64;
            }
        }
    }
    Some(TileSums { tiles, lo, hi, n_all: (w * h) as f64 })
}

/// Turn tile sums into the eight metrics and the tile sharpness.
pub fn finish(s: &TileSums) -> (Metrics, [f32; 64]) {
    let var = |s: f64, q: f64, n: f64| (q / n - (s / n).powi(2)).max(0.0);

    // Tile sharpness for subject comparison, before empty tiles are dropped.
    let mut tile_sharp = [0f32; 64];
    for (k, t) in s.tiles.iter().enumerate() {
        if t[0] > 0.0 {
            tile_sharp[k] = (var(t[3], t[4], t[0]) * 10.0).round() as f32 / 10.0;
        }
    }

    let tiles: Vec<[f64; 11]> = s.tiles.iter().copied().filter(|t| t[0] > 0.0).collect();
    if tiles.is_empty() {
        return (Metrics::default(), tile_sharp);
    }
    let top_k = ((tiles.len() as f64) * 0.10).ceil().max(1.0) as usize;
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;

    let mut lapv: Vec<f64> = tiles.iter().map(|t| var(t[3], t[4], t[0])).collect();
    lapv.sort_by(|a, b| b.total_cmp(a));
    let mut ten: Vec<f64> = tiles.iter().map(|t| t[5] / t[0]).collect();
    ten.sort_by(|a, b| b.total_cmp(a));

    let sum = |k: usize| tiles.iter().map(|t| t[k]).sum::<f64>();
    let n = sum(0);

    // Noise: flattest 25% of tiles by Sobel energy, median sigma
    let mut flat: Vec<&[f64; 11]> = tiles.iter().collect();
    flat.sort_by(|a, b| (a[5] / a[0]).total_cmp(&(b[5] / b[0])));
    let mut sig: Vec<f64> = flat[..(tiles.len() / 4).max(1)]
        .iter()
        .map(|t| (std::f64::consts::PI / 2.0).sqrt() * t[6] / (6.0 * t[0]))
        .collect();
    sig.sort_by(|a, b| a.total_cmp(b));

    let (mrg, myb) = (sum(7) / n, sum(9) / n);
    let (srg, syb) = (var(sum(7), sum(8), n).sqrt(), var(sum(9), sum(10), n).sqrt());

    let m = Metrics {
        sharpness: mean(&lapv[..top_k]) as f32,
        tenengrad: mean(&ten[..top_k]) as f32,
        mean_luma: ((sum(1) / n + 128.0) / 255.0) as f32,
        clip_shadows: (s.lo as f64 / s.n_all) as f32,
        clip_highlights: (s.hi as f64 / s.n_all) as f32,
        contrast: (var(sum(1), sum(2), n).sqrt() / 255.0) as f32,
        noise: sig[sig.len() / 2] as f32,
        colorfulness: ((srg * srg + syb * syb).sqrt() + 0.3 * (mrg * mrg + myb * myb).sqrt()) as f32,
    };
    (m, tile_sharp)
}

/// 64-bit difference hash: luma averaged into a 9x8 grid, bit = left < right.
pub fn dhash(img: &RgbImage) -> u64 {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return 0;
    }
    let mut cells = HashCells { sums: [[0f64; 9]; 8], counts: [[0f64; 9]; 8] };
    for y in 0..h {
        let r = y * 8 / h;
        for x in 0..w {
            let i = (y * w + x) * 3;
            let v = LUMA[0] as f64 * img.rgb[i] as f64
                + LUMA[1] as f64 * img.rgb[i + 1] as f64
                + LUMA[2] as f64 * img.rgb[i + 2] as f64;
            let c = x * 9 / w;
            cells.sums[r][c] += v;
            cells.counts[r][c] += 1.0;
        }
    }
    dhash_from_cells(&cells)
}

/// The dHash bits from the 9 x 8 cell averages.
pub fn dhash_from_cells(cells: &HashCells) -> u64 {
    let mut hash = 0u64;
    for r in 0..8 {
        for c in 0..8 {
            let a = cells.sums[r][c] / cells.counts[r][c].max(1.0);
            let b = cells.sums[r][c + 1] / cells.counts[r][c + 1].max(1.0);
            if a < b {
                hash |= 1 << (r * 8 + c);
            }
        }
    }
    hash
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

fn ascii_field(ex: &exif::Exif, tag: exif::Tag) -> Option<String> {
    let f = ex.get_field(tag, exif::In::PRIMARY)?;
    match &f.value {
        exif::Value::Ascii(v) => v.first().map(|s| String::from_utf8_lossy(s).trim().to_string()),
        _ => None,
    }
}

/// DateTimeOriginal (+ SubSecTimeOriginal) as milliseconds of a naive time.
pub fn exif_capture_ms(ex: &exif::Exif) -> Option<i64> {
    let dt = ascii_field(ex, exif::Tag::DateTimeOriginal)?;
    let naive = chrono::NaiveDateTime::parse_from_str(dt.get(..19)?, "%Y:%m:%d %H:%M:%S").ok()?;
    let mut ms = naive.and_utc().timestamp_millis();
    if let Some(sub) = ascii_field(ex, exif::Tag::SubSecTimeOriginal) {
        let digits: String = sub.chars().filter(|c| c.is_ascii_digit()).take(3).collect();
        if !digits.is_empty() {
            let padded = format!("{digits:0<3}");
            ms += padded.parse::<i64>().unwrap_or(0);
        }
    }
    Some(ms)
}

/// Modified time converted to the same naive local timeline as EXIF times.
pub fn mtime_as_naive_ms(mtime_ms: i64) -> i64 {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(mtime_ms) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => {
            t.naive_local().and_utc().timestamp_millis()
        }
        chrono::LocalResult::None => mtime_ms,
    }
}

/// Capture time from EXIF, the embedded JPEG of a RAW file, or the modified time.
pub fn capture_ms(path: &Path, format: Format, mtime_ms: i64) -> i64 {
    let container = (|| {
        let file = std::fs::File::open(path).ok()?;
        let mut r = std::io::BufReader::new(file);
        let ex = exif::Reader::new().read_from_container(&mut r).ok()?;
        exif_capture_ms(&ex)
    })();
    if let Some(ms) = container {
        return ms;
    }
    if format == Format::Raw || format == Format::Tiff {
        if let Some(ms) = crate::decode::raw_embedded_exif(path).and_then(|ex| exif_capture_ms(&ex)) {
            return ms;
        }
    }
    mtime_as_naive_ms(mtime_ms)
}

/// ISO 8601 without a time zone, e.g. 2026-05-14T16:02:11.340.
pub fn format_capture(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.naive_utc().format("%Y-%m-%dT%H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}

/// 16-hex-digit FNV-1a hash of the relative path: the thumbnail id.
pub fn fnv1a_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Save the analysis image as a 320 px JPEG (quality 80).
pub fn write_thumb(img: &RgbImage, dest: &Path) -> anyhow::Result<()> {
    let small = if img.width.max(img.height) <= 320 { img.clone() } else { crate::decode::resize_long_edge(img.clone(), 320)? };
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = dest.with_extension("jpg.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut w = std::io::BufWriter::new(file);
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 80);
        enc.encode(&small.rgb, small.width, small.height, image::ExtendedColorType::Rgb8)?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}
