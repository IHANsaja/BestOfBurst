//! Route a file to its decoder, producing 8-bit RGB, then shrink it to the analysis size.

pub mod raw;
pub mod wic;

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use fast_image_resize as fr;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::scan::Format;

/// Long edge used for analysis.
pub const ANALYSIS_EDGE: u32 = 2048;
/// Images smaller than this on either side fail with "too small".
pub const MIN_EDGE: u32 = 64;

#[derive(Clone, Debug, Default)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl RgbImage {
    pub fn new(width: u32, height: u32, rgb: Vec<u8>) -> Result<Self> {
        if rgb.len() < (width as usize) * (height as usize) * 3 || width == 0 || height == 0 {
            bail!("decoder returned a truncated image");
        }
        Ok(RgbImage { width, height, rgb })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoder {
    ZuneJpeg,
    Image,
    JxlOxide,
    RawPreview,
    Wic,
}

impl Decoder {
    pub fn name(self) -> &'static str {
        match self {
            Decoder::ZuneJpeg => "zune-jpeg",
            Decoder::Image => "image",
            Decoder::JxlOxide => "jxl-oxide",
            Decoder::RawPreview => "raw-preview",
            Decoder::Wic => "wic",
        }
    }
}

pub struct Decoded {
    pub image: RgbImage,
    pub decoder: Decoder,
}

/// Decode a JPEG held in memory with zune-jpeg, always to RGB.
pub fn decode_jpeg_bytes(data: &[u8]) -> Result<RgbImage> {
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGB)
        .set_max_width(1 << 16)
        .set_max_height(1 << 16)
        .set_strict_mode(false);
    let mut dec = JpegDecoder::new_with_options(ZCursor::new(data), options);
    dec.decode_headers().map_err(|e| anyhow!("jpeg: {e:?}"))?;
    let info = dec.info().ok_or_else(|| anyhow!("jpeg: no header"))?;
    let pixels = dec.decode().map_err(|e| anyhow!("jpeg: {e:?}"))?;
    let (w, h) = (info.width as u32, info.height as u32);
    let n = (w as usize) * (h as usize);
    let channels = if n == 0 { 0 } else { pixels.len() / n };
    match channels {
        3 => RgbImage::new(w, h, pixels),
        1 => RgbImage::new(w, h, pixels.iter().flat_map(|&v| [v, v, v]).collect()),
        c if c > 3 => RgbImage::new(
            w,
            h,
            pixels.chunks_exact(c).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        ),
        _ => bail!("jpeg: unexpected channel count"),
    }
}

fn decode_with_image(data: &[u8], format: Format) -> Result<RgbImage> {
    let hint = match format {
        Format::Png => Some(image::ImageFormat::Png),
        Format::Tiff => Some(image::ImageFormat::Tiff),
        Format::Webp => Some(image::ImageFormat::WebP),
        Format::Bmp => Some(image::ImageFormat::Bmp),
        Format::Gif => Some(image::ImageFormat::Gif),
        Format::Tga => Some(image::ImageFormat::Tga),
        Format::Qoi => Some(image::ImageFormat::Qoi),
        Format::Pnm => Some(image::ImageFormat::Pnm),
        _ => None,
    };
    let img = match hint {
        Some(f) => image::load_from_memory_with_format(data, f),
        None => image::load_from_memory(data),
    }
    .context("image")?;
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    RgbImage::new(w, h, rgb.into_raw())
}

fn decode_jxl(data: &[u8]) -> Result<RgbImage> {
    use jxl_oxide::{EnumColourEncoding, JxlImage, RenderingIntent};
    let mut img = JxlImage::builder()
        .read(std::io::Cursor::new(data))
        .map_err(|e| anyhow!("jxl: {e}"))?;
    img.request_color_encoding(EnumColourEncoding::srgb(RenderingIntent::Relative));
    let render = img.render_frame(0).map_err(|e| anyhow!("jxl: {e}"))?;
    let mut stream = render.stream_no_alpha();
    let (w, h, c) = (stream.width(), stream.height(), stream.channels() as usize);
    let mut buf = vec![0u8; w as usize * h as usize * c];
    stream.write_to_buffer(&mut buf);
    match c {
        3 => RgbImage::new(w, h, buf),
        1 => RgbImage::new(w, h, buf.iter().flat_map(|&v| [v, v, v]).collect()),
        c if c > 3 => RgbImage::new(w, h, buf.chunks_exact(c).flat_map(|p| [p[0], p[1], p[2]]).collect()),
        _ => bail!("jxl: unexpected channel count"),
    }
}

fn decode_raw_preview(path: &Path) -> Result<RgbImage> {
    let file = std::fs::File::open(path)?;
    // SAFETY: the file is opened read-only and BestOfBurst never writes originals. Another
    // process truncating it mid-read could fault; catch_unwind does not cover that, but it is
    // the documented trade-off of memory-mapping.
    let map = unsafe { memmap2::Mmap::map(&file)? };
    let mut last_err = anyhow!("no embedded JPEG of at least 1000 px");
    for candidate in raw::jpeg_candidates(&map) {
        match decode_jpeg_bytes(candidate) {
            Ok(img) => return Ok(img),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// Decode a file at full size through the routing table, falling through to WIC last.
pub fn decode_full(path: &Path, format: Format) -> Result<Decoded> {
    let mut errors: Vec<String> = Vec::new();
    let read = || std::fs::read(path).with_context(|| format!("cannot read {}", path.display()));

    let first: Option<Result<Decoded>> = match format {
        Format::Jpeg => Some(read().and_then(|d| decode_jpeg_bytes(&d)).map(|image| Decoded {
            image,
            decoder: Decoder::ZuneJpeg,
        })),
        Format::Png | Format::Webp | Format::Bmp | Format::Gif | Format::Tga | Format::Qoi | Format::Pnm => {
            Some(read().and_then(|d| decode_with_image(&d, format)).map(|image| Decoded {
                image,
                decoder: Decoder::Image,
            }))
        }
        Format::Tiff => {
            let r = read().and_then(|d| decode_with_image(&d, format)).map(|image| Decoded {
                image,
                decoder: Decoder::Image,
            });
            match r {
                Ok(d) => Some(Ok(d)),
                Err(e) => {
                    errors.push(format!("{e:#}"));
                    Some(decode_raw_preview(path).map(|image| Decoded {
                        image,
                        decoder: Decoder::RawPreview,
                    }))
                }
            }
        }
        Format::Jxl => Some(read().and_then(|d| decode_jxl(&d)).map(|image| Decoded {
            image,
            decoder: Decoder::JxlOxide,
        })),
        Format::Raw => Some(decode_raw_preview(path).map(|image| Decoded {
            image,
            decoder: Decoder::RawPreview,
        })),
        Format::Heif | Format::Avif | Format::Jxr => None,
    };

    match first {
        Some(Ok(d)) => return Ok(d),
        Some(Err(e)) => errors.push(format!("{e:#}")),
        None => {}
    }

    match wic::decode_wic(path) {
        Ok(image) => Ok(Decoded { image, decoder: Decoder::Wic }),
        Err(e) => {
            let wic_msg = format!("{e:#}");
            // A missing-codec message is the most useful thing to show for WIC-only formats and
            // RAW files; for everything else the pure-Rust decoder's error says more.
            let codec_hint = matches!(format, Format::Heif | Format::Avif | Format::Jxr | Format::Raw);
            if errors.is_empty() || (codec_hint && wic::is_missing_codec(&wic_msg)) {
                Err(anyhow!(wic_msg))
            } else {
                errors.push(format!("wic: {wic_msg}"));
                Err(anyhow!(errors.join("; ")))
            }
        }
    }
}

/// Downscale so the long edge is at most `edge` (never upscale), using Catmull-Rom.
pub fn resize_long_edge(img: RgbImage, edge: u32) -> Result<RgbImage> {
    let long = img.width.max(img.height);
    if long <= edge {
        return Ok(img);
    }
    let scale = edge as f64 / long as f64;
    let nw = ((img.width as f64 * scale).round() as u32).max(1);
    let nh = ((img.height as f64 * scale).round() as u32).max(1);
    let src = fr::images::Image::from_vec_u8(img.width, img.height, img.rgb, fr::PixelType::U8x3)
        .map_err(|e| anyhow!("resize: {e:?}"))?;
    let mut dst = fr::images::Image::new(nw, nh, fr::PixelType::U8x3);
    let mut resizer = fr::Resizer::new();
    let opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::CatmullRom));
    resizer
        .resize(&src, &mut dst, &opts)
        .map_err(|e| anyhow!("resize: {e:?}"))?;
    RgbImage::new(nw, nh, dst.into_vec())
}

/// Decode and shrink to the 2,048 px analysis size.
pub fn decode_for_analysis(path: &Path, format: Format) -> Result<Decoded> {
    let d = decode_full(path, format)?;
    if d.image.width < MIN_EDGE || d.image.height < MIN_EDGE {
        bail!("too small");
    }
    let image = resize_long_edge(d.image, ANALYSIS_EDGE)?;
    Ok(Decoded { image, decoder: d.decoder })
}

/// EXIF orientation 1..8 (1 when unknown). Display only.
pub fn read_orientation(path: &Path, format: Format) -> u32 {
    let from = |r: &mut dyn FnMut() -> Option<exif::Exif>| -> Option<u32> {
        let ex = r()?;
        let f = ex.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
        f.value.get_uint(0)
    };
    let mut container = || -> Option<exif::Exif> {
        let file = std::fs::File::open(path).ok()?;
        let mut r = std::io::BufReader::new(file);
        exif::Reader::new().read_from_container(&mut r).ok()
    };
    if let Some(o) = from(&mut container) {
        return o;
    }
    if format == Format::Raw {
        if let Some(o) = raw_embedded_exif(path).and_then(|ex| {
            ex.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        }) {
            return o;
        }
    }
    1
}

/// EXIF from the embedded JPEG previews of a RAW file, for containers kamadak-exif cannot read.
pub fn raw_embedded_exif(path: &Path) -> Option<exif::Exif> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: read-only map of an original, see decode_raw_preview.
    let map = unsafe { memmap2::Mmap::map(&file).ok()? };
    let finder = memchr::memmem::Finder::new(&[0xFF, 0xD8, 0xFF]);
    for start in finder.find_iter(&map).take(16) {
        let mut cur = std::io::Cursor::new(&map[start..]);
        if let Ok(ex) = exif::Reader::new().read_from_container(&mut cur) {
            return Some(ex);
        }
    }
    None
}

/// Apply EXIF orientation (display only; analysis ignores orientation on purpose).
pub fn apply_orientation(img: RgbImage, orientation: u32) -> RgbImage {
    if !(2..=8).contains(&orientation) {
        return img;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let swap = orientation >= 5;
    let (nw, nh) = if swap { (h, w) } else { (w, h) };
    let mut out = vec![0u8; nw * nh * 3];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                8 => (y, w - 1 - x),
                _ => (x, y),
            };
            let s = (y * w + x) * 3;
            let d = (ny * nw + nx) * 3;
            out[d..d + 3].copy_from_slice(&img.rgb[s..s + 3]);
        }
    }
    RgbImage { width: nw as u32, height: nh as u32, rgb: out }
}

/// Crop a `cw` x `ch` window centred on (cx, cy) given as 0..1 fractions, clamped to the image.
pub fn crop_centered(img: &RgbImage, cx: f32, cy: f32, cw: u32, ch: u32) -> RgbImage {
    let cw = cw.min(img.width);
    let ch = ch.min(img.height);
    let x0 = ((cx * img.width as f32) as i64 - cw as i64 / 2).clamp(0, (img.width - cw) as i64) as usize;
    let y0 = ((cy * img.height as f32) as i64 - ch as i64 / 2).clamp(0, (img.height - ch) as i64) as usize;
    let mut out = Vec::with_capacity(cw as usize * ch as usize * 3);
    for y in y0..y0 + ch as usize {
        let s = (y * img.width as usize + x0) * 3;
        out.extend_from_slice(&img.rgb[s..s + cw as usize * 3]);
    }
    RgbImage { width: cw, height: ch, rgb: out }
}
