//! Folder walk, skip rules and signature-based format detection.

use std::fs::File;
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

const MIN_SIZE: u64 = 10 * 1024;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Tiff,
    Webp,
    Bmp,
    Gif,
    Tga,
    Qoi,
    Pnm,
    Jxl,
    Heif,
    Avif,
    Jxr,
    Raw,
}

impl Format {
    /// Name stored in `state.json` and `report.csv`.
    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "jpeg",
            Format::Png => "png",
            Format::Tiff => "tiff",
            Format::Webp => "webp",
            Format::Bmp => "bmp",
            Format::Gif => "gif",
            Format::Tga => "tga",
            Format::Qoi => "qoi",
            Format::Pnm => "pnm",
            Format::Jxl => "jxl",
            Format::Heif => "heif",
            Format::Avif => "avif",
            Format::Jxr => "jxr",
            Format::Raw => "raw",
        }
    }

    pub fn from_name(s: &str) -> Option<Format> {
        Some(match s {
            "jpeg" => Format::Jpeg,
            "png" => Format::Png,
            "tiff" => Format::Tiff,
            "webp" => Format::Webp,
            "bmp" => Format::Bmp,
            "gif" => Format::Gif,
            "tga" => Format::Tga,
            "qoi" => Format::Qoi,
            "pnm" => Format::Pnm,
            "jxl" => Format::Jxl,
            "heif" => Format::Heif,
            "avif" => Format::Avif,
            "jxr" => Format::Jxr,
            "raw" => Format::Raw,
            _ => return None,
        })
    }
}

const RAW_EXTS: &[&str] = &[
    "3fr", "ari", "arw", "cr2", "dcr", "dng", "erf", "fff", "iiq", "k25", "kdc", "mef", "mos", "mrw",
    "nef", "nrw", "orf", "pef", "raw", "rw2", "rwl", "sr2", "srf", "srw", "cr3", "raf", "crw", "x3f",
];

/// Extensions never worth opening (sidecars, video, documents).
const SKIP_EXTS: &[&str] = &[
    "xmp", "thm", "aae", "mov", "mp4", "m4v", "avi", "mts", "m2ts", "mkv", "wav", "mp3", "lrv",
    "txt", "json", "csv", "xml", "pdf", "db", "ini", "lnk", "zip", "pp3", "dop", "cos", "on1",
];

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn is_raw_ext(ext: &str) -> bool {
    RAW_EXTS.contains(&ext)
}

fn by_extension(ext: &str) -> Option<Format> {
    Some(match ext {
        "jpg" | "jpeg" | "jpe" | "jfif" => Format::Jpeg,
        "png" => Format::Png,
        "tif" | "tiff" => Format::Tiff,
        "webp" => Format::Webp,
        "bmp" => Format::Bmp,
        "gif" => Format::Gif,
        "tga" => Format::Tga,
        "qoi" => Format::Qoi,
        "pbm" | "pgm" | "ppm" => Format::Pnm,
        "jxl" => Format::Jxl,
        "heic" | "heif" | "hif" => Format::Heif,
        "avif" => Format::Avif,
        "jxr" | "wdp" | "hdp" => Format::Jxr,
        e if is_raw_ext(e) => Format::Raw,
        _ => return None,
    })
}

/// Classify by the first bytes; fall back to the extension only when no signature matches.
pub fn detect(head: &[u8], ext: &str) -> Option<Format> {
    let starts = |sig: &[u8]| head.len() >= sig.len() && &head[..sig.len()] == sig;
    let at = |off: usize, sig: &[u8]| head.len() >= off + sig.len() && &head[off..off + sig.len()] == sig;

    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some(Format::Jpeg);
    }
    if starts(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some(Format::Png);
    }
    if starts(b"FUJIFILMCCD-RAW") || starts(b"FOVb") {
        return Some(Format::Raw);
    }
    if starts(b"II") && at(6, b"HEAPCCDR") {
        return Some(Format::Raw);
    }
    if starts(b"IIRO") || starts(b"IIU\0") {
        return Some(Format::Raw);
    }
    if starts(b"II\xBC") {
        return Some(Format::Jxr);
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        // A TIFF header with a RAW extension is a TIFF-based RAW (CR2, NEF, ARW, DNG, ...).
        return Some(if is_raw_ext(ext) { Format::Raw } else { Format::Tiff });
    }
    if starts(b"RIFF") && at(8, b"WEBP") {
        return Some(Format::Webp);
    }
    if starts(&[0xFF, 0x0A]) || starts(&[0, 0, 0, 0x0C, 0x4A, 0x58, 0x4C, 0x20]) {
        return Some(Format::Jxl);
    }
    if at(4, b"ftyp") && head.len() >= 12 {
        let brand = &head[8..12];
        match brand {
            b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
                // mif1/msf1 are generic HEIF brands that AVIF files also use; trust .avif there.
                if (brand == b"mif1" || brand == b"msf1") && ext == "avif" {
                    return Some(Format::Avif);
                }
                return Some(Format::Heif);
            }
            b"avif" | b"avis" => return Some(Format::Avif),
            b"crx " => return Some(Format::Raw),
            _ => {}
        }
    }
    if starts(b"qoif") {
        return Some(Format::Qoi);
    }
    if starts(b"GIF8") {
        return Some(Format::Gif);
    }
    if starts(b"BM") {
        return Some(Format::Bmp);
    }
    if head.len() >= 2 && head[0] == b'P' && (b'1'..=b'6').contains(&head[1]) {
        return Some(Format::Pnm);
    }
    // TGA has no signature, and anything else unrecognised is judged by extension.
    by_extension(ext)
}

/// Detect a single file by reading its first 64 bytes.
pub fn detect_file(path: &Path) -> std::io::Result<Option<Format>> {
    let mut head = [0u8; 64];
    let mut f = File::open(path)?;
    let mut n = 0;
    while n < head.len() {
        let r = f.read(&mut head[n..])?;
        if r == 0 {
            break;
        }
        n += r;
    }
    Ok(detect(&head[..n], &ext_of(path)))
}

#[derive(Clone, Debug)]
pub struct ScanItem {
    /// Path relative to the input folder, with Windows separators. The key in `state.json`.
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub format: Format,
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    if entry.file_name().to_string_lossy().starts_with('.') {
        return true;
    }
    entry
        .metadata()
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
        .unwrap_or(false)
}

pub fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (dunce_canonical(a), dunce_canonical(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// Canonical path, compared case-insensitively on Windows.
pub fn dunce_canonical(p: &Path) -> Option<String> {
    std::fs::canonicalize(p)
        .ok()
        .map(|c| c.to_string_lossy().to_lowercase())
}

/// Walk `input` recursively, skipping the output folder, hidden files, files under 10 KB and
/// anything that is not a supported image.
pub fn scan(input: &Path, out: &Path, stop: Option<&std::sync::atomic::AtomicBool>) -> Vec<ScanItem> {
    let out_canon = dunce_canonical(out);
    let mut items = Vec::new();
    let walker = WalkDir::new(input).follow_links(false).into_iter().filter_entry(|e| {
        if is_hidden(e) {
            return false;
        }
        if e.file_type().is_dir() && e.depth() > 0 {
            if let Some(oc) = &out_canon {
                if dunce_canonical(e.path()).as_ref() == Some(oc) {
                    return false;
                }
            } else if same_path(e.path(), out) {
                return false;
            }
        }
        true
    });
    for entry in walker.flatten() {
        if let Some(s) = stop {
            if s.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = ext_of(path);
        if SKIP_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() < MIN_SIZE {
            continue;
        }
        let Ok(Some(format)) = detect_file(path) else { continue };
        let rel = path
            .strip_prefix(input)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('/', "\\");
        items.push(ScanItem {
            rel,
            abs: path.to_path_buf(),
            size: meta.len(),
            mtime_ms: mtime_ms(&meta),
            format,
        });
    }
    items.sort_by(|a, b| a.rel.cmp(&b.rel));
    items
}

/// Default output folder: `<input>\_bestofburst`.
pub fn default_out(input: &Path) -> PathBuf {
    input.join("_bestofburst")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures() {
        assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE1], "png"), Some(Format::Jpeg));
        assert_eq!(detect(b"II*\0rest", "tif"), Some(Format::Tiff));
        assert_eq!(detect(b"II*\0rest", "nef"), Some(Format::Raw));
        assert_eq!(detect(b"IIRO....", "orf"), Some(Format::Raw));
        assert_eq!(detect(b"\0\0\0\x18ftypheic\0\0\0\0", "jpg"), Some(Format::Heif));
        assert_eq!(detect(b"\0\0\0\x18ftypavif\0\0\0\0", "heic"), Some(Format::Avif));
        assert_eq!(detect(b"\0\0\0\x18ftypcrx \0\0\0\0", "cr3"), Some(Format::Raw));
        assert_eq!(detect(b"RIFF\0\0\0\0WEBPVP8 ", "bin"), Some(Format::Webp));
        assert_eq!(detect(b"FUJIFILMCCD-RAW 0201", "raf"), Some(Format::Raw));
        assert_eq!(detect(b"\x00\x00\x02\x00", "tga"), Some(Format::Tga));
        assert_eq!(detect(b"nothing", "txt"), None);
    }
}
