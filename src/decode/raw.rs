//! RAW path: the largest embedded baseline/progressive JPEG.

/// All embedded JPEGs of at least 1,000 px on the long edge, largest first.
pub fn jpeg_candidates(data: &[u8]) -> Vec<&[u8]> {
    let mut found: Vec<(&[u8], u64)> = Vec::new();
    let finder = memchr::memmem::Finder::new(&[0xFF, 0xD8, 0xFF]);
    for start in finder.find_iter(data) {
        let slice = &data[start..];
        if let Some((w, h)) = jpeg_dimensions(slice) {
            if w.max(h) >= 1000 {
                found.push((slice, w as u64 * h as u64));
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found.into_iter().map(|(s, _)| s).collect()
}

/// Width and height from the frame header, only for baseline/progressive frames (C0, C1, C2).
/// Lossless (C3) and other frame types are raw sensor data and are never picked.
pub fn jpeg_dimensions(d: &[u8]) -> Option<(u16, u16)> {
    let mut i = 2; // past SOI
    for _ in 0..64 {
        if *d.get(i)? != 0xFF {
            return None;
        }
        let mut m = *d.get(i + 1)?;
        while m == 0xFF {
            i += 1;
            m = *d.get(i + 1)?;
        }
        match m {
            0xC0 | 0xC1 | 0xC2 => {
                let h = u16::from_be_bytes([*d.get(i + 5)?, *d.get(i + 6)?]);
                let w = u16::from_be_bytes([*d.get(i + 7)?, *d.get(i + 8)?]);
                return (w > 0 && h > 0).then_some((w, h));
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return None,
            0xD8 | 0xD9 | 0xDA => return None,
            _ => {
                let len = u16::from_be_bytes([*d.get(i + 2)?, *d.get(i + 3)?]) as usize;
                if len < 2 {
                    return None;
                }
                i += 2 + len;
            }
        }
    }
    None
}
