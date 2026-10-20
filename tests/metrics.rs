//! Metrics, noise/clipping, dHash, RAW path and format routing.

mod common;

use bestofburst::analyze::{analyze, dhash, hamming};
use bestofburst::decode::{self, raw, RgbImage};
use bestofburst::scan::{self, Format};
use common::*;

#[test]
fn flat_gray_has_no_sharpness_or_contrast() {
    let (m, tiles) = analyze(&gray(512, 384, 128));
    assert!(m.sharpness < 0.01, "sharpness {}", m.sharpness);
    assert!(m.contrast < 0.001, "contrast {}", m.contrast);
    assert!(tiles.iter().all(|&t| t < 0.01));
}

#[test]
fn checkerboard_is_sharp() {
    let (m, _) = analyze(&checkerboard(512, 384, 8));
    assert!(m.sharpness > 1000.0, "sharpness {}", m.sharpness);
    assert!(m.contrast > 0.2);
}

#[test]
fn blur_strictly_reduces_sharpness() {
    let base = from_luma(512, 384, {
        let mut rng = Rng::new(7);
        let noise: Vec<f64> = (0..512 * 384).map(|_| rng.uniform() * 255.0).collect();
        move |x, y| noise[(y * 512 + x) as usize]
    });
    let s0 = analyze(&base).0.sharpness;
    let s1 = analyze(&blur(&base, 1.0)).0.sharpness;
    let s2 = analyze(&blur(&base, 2.0)).0.sharpness;
    let s4 = analyze(&blur(&base, 4.0)).0.sharpness;
    assert!(s0 > s1 && s1 > s2 && s2 > s4, "{s0} {s1} {s2} {s4}");
}

#[test]
fn noise_sigma_five_is_measured() {
    let mut rng = Rng::new(42);
    let noise: Vec<f64> = (0..800 * 600).map(|_| rng.gauss()).collect();
    let img = from_luma(800, 600, |x, y| 128.0 + 5.0 * noise[(y * 800 + x) as usize]);
    let (m, _) = analyze(&img);
    assert!((m.noise - 5.0).abs() <= 0.75, "noise {}", m.noise);
}

#[test]
fn clipping_extremes() {
    assert_eq!(analyze(&gray(256, 256, 255)).0.clip_highlights, 1.0);
    assert_eq!(analyze(&gray(256, 256, 0)).0.clip_shadows, 1.0);
}

#[test]
fn dhash_behaviour() {
    let a = scene(640, 480, 1);
    assert_eq!(hamming(dhash(&a), dhash(&a)), 0);
    let brighter = RgbImage {
        width: a.width,
        height: a.height,
        rgb: a.rgb.iter().map(|&v| ((v as f32) * 1.1).min(255.0) as u8).collect(),
    };
    let d = hamming(dhash(&a), dhash(&brighter));
    assert!(d <= 4, "brightness distance {d}");
    let other = scene(640, 480, 99);
    let d = hamming(dhash(&a), dhash(&other));
    assert!(d > 20, "unrelated distance {d}");
}

#[test]
fn raw_path_picks_baseline_not_lossless() {
    let preview = encode_jpeg(&scene(1200, 800, 3), 85);
    let mut data = Vec::new();
    data.extend_from_slice(b"II*\0\x08\0\0\0");
    data.extend(std::iter::repeat(0xAB).take(5000));
    // Fake lossless (C3) frame claiming 6000 x 4000: raw sensor data, must be skipped.
    data.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xC3, 0x00, 0x0B, 0x0C, 0x0F, 0xA0, 0x17, 0x70, 0x01, 0x01, 0x11, 0x00]);
    data.extend(std::iter::repeat(0x55).take(3000));
    let at = data.len();
    data.extend_from_slice(&preview);
    data.extend(std::iter::repeat(0x00).take(1000));

    let cands = raw::jpeg_candidates(&data);
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].as_ptr(), data[at..].as_ptr());
    let img = decode::decode_jpeg_bytes(cands[0]).expect("decode preview");
    assert_eq!((img.width, img.height), (1200, 800));

    // Through the router as a .nef file.
    let dir = scratch("rawpath");
    let p = dir.join("fake.nef");
    std::fs::write(&p, &data).unwrap();
    assert_eq!(scan::detect_file(&p).unwrap(), Some(Format::Raw));
    let d = decode::decode_for_analysis(&p, Format::Raw).expect("raw decode");
    assert_eq!(d.decoder.name(), "raw-preview");
}

fn write_with_image(img: &RgbImage, path: &std::path::Path, fmt: image::ImageFormat) {
    let buf = image::RgbImage::from_raw(img.width, img.height, img.rgb.clone()).unwrap();
    image::DynamicImage::ImageRgb8(buf).save_with_format(path, fmt).expect("encode fixture");
}

#[test]
fn every_pure_rust_format_decodes() {
    let dir = scratch("formats");
    let img = scene(320, 240, 5);
    save_jpeg(&img, &dir.join("a.jpg"));
    write_with_image(&img, &dir.join("a.png"), image::ImageFormat::Png);
    write_with_image(&img, &dir.join("a.tif"), image::ImageFormat::Tiff);
    write_with_image(&img, &dir.join("a.webp"), image::ImageFormat::WebP);
    write_with_image(&img, &dir.join("a.bmp"), image::ImageFormat::Bmp);
    write_with_image(&img, &dir.join("a.gif"), image::ImageFormat::Gif);
    write_with_image(&img, &dir.join("a.tga"), image::ImageFormat::Tga);
    write_with_image(&img, &dir.join("a.qoi"), image::ImageFormat::Qoi);
    write_with_image(&img, &dir.join("a.ppm"), image::ImageFormat::Pnm);
    // 16-bit TIFF
    let img16 = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(300, 200, |x, y| {
        image::Rgb([(x * 200) as u16, (y * 300) as u16, 30000])
    });
    image::DynamicImage::ImageRgb16(img16).save_with_format(dir.join("b16.tiff"), image::ImageFormat::Tiff).unwrap();
    // A PNG renamed to .jpg decodes by signature.
    std::fs::copy(dir.join("a.png"), dir.join("renamed.jpg")).unwrap();

    let expect = [
        ("a.jpg", Format::Jpeg, "zune-jpeg"),
        ("a.png", Format::Png, "image"),
        ("a.tif", Format::Tiff, "image"),
        ("b16.tiff", Format::Tiff, "image"),
        ("a.webp", Format::Webp, "image"),
        ("a.bmp", Format::Bmp, "image"),
        ("a.gif", Format::Gif, "image"),
        ("a.tga", Format::Tga, "image"),
        ("a.qoi", Format::Qoi, "image"),
        ("a.ppm", Format::Pnm, "image"),
        ("renamed.jpg", Format::Png, "image"),
    ];
    for (name, fmt, dec) in expect {
        let p = dir.join(name);
        let detected = scan::detect_file(&p).unwrap();
        assert_eq!(detected, Some(fmt), "{name}");
        let d = decode::decode_for_analysis(&p, fmt).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(d.decoder.name(), dec, "{name}");
        assert!(d.image.width >= 64 && d.image.height >= 64);
    }
}

#[test]
fn wic_formats_decode_or_report_missing_codec() {
    // No pure-Rust HEIC/AVIF encoder exists, so this checks the routing and the message with a
    // truncated HEIC header: either WIC reports a missing codec or it fails to parse the file.
    let dir = scratch("wic");
    let p = dir.join("x.heic");
    let mut data = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic".to_vec();
    data.extend(std::iter::repeat(0).take(20000));
    std::fs::write(&p, &data).unwrap();
    assert_eq!(scan::detect_file(&p).unwrap(), Some(Format::Heif));
    match decode::decode_full(&p, Format::Heif) {
        Ok(_) => panic!("garbage HEIC must not decode"),
        Err(e) => {
            let msg = format!("{e:#}");
            if bestofburst::decode::wic::is_missing_codec(&msg) {
                eprintln!("skipping: HEIF codec not installed ({msg})");
            } else {
                assert!(msg.contains("wic"), "{msg}");
            }
        }
    }
}

#[test]
fn too_small_fails() {
    let dir = scratch("small");
    let p = dir.join("tiny.jpg");
    save_jpeg(&scene(48, 300, 2), &p);
    let e = decode::decode_for_analysis(&p, Format::Jpeg).err().expect("must fail");
    assert!(format!("{e:#}").contains("too small"));
}

#[test]
fn resize_never_upscales_and_caps_long_edge() {
    let big = scene(3000, 1000, 4);
    let r = decode::resize_long_edge(big, 2048).unwrap();
    assert_eq!((r.width, r.height), (2048, 683));
    let small = scene(800, 600, 4);
    let r = decode::resize_long_edge(small, 2048).unwrap();
    assert_eq!((r.width, r.height), (800, 600));
}

fn textured(w: u32, h: u32, seed: u64) -> RgbImage {
    let mut img = scene(w, h, seed);
    let mut rng = Rng::new(seed);
    for v in img.rgb.iter_mut() {
        *v = (*v as f64 + 14.0 * rng.gauss()).round().clamp(0.0, 255.0) as u8;
    }
    img
}

fn close(a: f32, b: f32, rel: f32, abs: f32) -> bool {
    (a - b).abs() <= abs.max(rel * a.abs().max(b.abs()))
}

#[test]
fn gpu_matches_cpu() {
    use bestofburst::analyze::{dhash_from_cells, finish};
    let g = match bestofburst::gpu::get() {
        Ok(g) => g,
        Err(why) => {
            eprintln!("skipping: no GPU ({why})");
            return;
        }
    };
    // Odd sizes exercise the unaligned upload tail; 1500 x 999 needs no resize.
    for (w, h, seed) in [(3001u32, 2001u32, 11u64), (1500, 999, 12), (2400, 3600, 13)] {
        let img = textured(w, h, seed);
        let small = decode::resize_long_edge(img.clone(), 2048).unwrap();
        let (cm, ct) = analyze(&small);
        let ch = dhash(&small);
        let o = g.analyze(&img, 2048, 320).expect("gpu analyze");
        assert_eq!((o.width, o.height), (small.width, small.height), "{w}x{h}");
        let (gm, gt) = finish(&o.sums);
        assert!(close(cm.sharpness, gm.sharpness, 0.02, 0.5), "sharpness {} vs {}", cm.sharpness, gm.sharpness);
        assert!(close(cm.tenengrad, gm.tenengrad, 0.02, 1.0), "tenengrad {} vs {}", cm.tenengrad, gm.tenengrad);
        assert!(close(cm.mean_luma, gm.mean_luma, 0.0, 0.003), "luma {} vs {}", cm.mean_luma, gm.mean_luma);
        assert!(close(cm.contrast, gm.contrast, 0.0, 0.003), "contrast {} vs {}", cm.contrast, gm.contrast);
        assert!(close(cm.noise, gm.noise, 0.04, 0.05), "noise {} vs {}", cm.noise, gm.noise);
        assert!(close(cm.colorfulness, gm.colorfulness, 0.02, 0.2), "color {} vs {}", cm.colorfulness, gm.colorfulness);
        assert!(close(cm.clip_highlights, gm.clip_highlights, 0.0, 0.002));
        assert!(close(cm.clip_shadows, gm.clip_shadows, 0.0, 0.002));
        for k in 0..64 {
            assert!(close(ct[k], gt[k], 0.03, 1.0), "tile {k}: {} vs {}", ct[k], gt[k]);
        }
        assert!(hamming(ch, dhash_from_cells(&o.cells)) <= 2, "dhash");
        assert_eq!(o.thumb.width.max(o.thumb.height), 320.min(small.width.max(small.height)));
    }
}
