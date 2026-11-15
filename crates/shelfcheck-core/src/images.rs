//! Local image sizes for Google's 500 x 500 minimum (`--images DIR`).
//!
//! Only the file header is read (PNG, GIF, JPEG, WebP). Nothing is fetched:
//! an image is found by the last path segment of its URL in the folder given.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::{Field, Record};

/// Bytes read from the start of each image; enough for every header kind
/// here, including JPEG files with large metadata before the frame header.
const HEAD: u64 = 256 * 1024;

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) << 8 | u32::from(*b.get(i + 1)?))
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8)
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(be16(b, i)? << 16 | be16(b, i + 2)?)
}

fn le24(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8 | u32::from(*b.get(i + 2)?) << 16)
}

/// (width, height) from an image header, or None for an unknown format.
pub fn size_from_bytes(b: &[u8]) -> Option<(u32, u32)> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some((be32(b, 16)?, be32(b, 20)?));
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some((le16(b, 6)?, le16(b, 8)?));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            // Start-of-frame markers, except DHT (C4), JPG (C8) and DAC (CC).
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC
            {
                return Some((be16(b, i + 7)?, be16(b, i + 5)?));
            }
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) || marker == 0xFF
            {
                i += if marker == 0xFF { 1 } else { 2 };
                continue;
            }
            let len = be16(b, i + 2)? as usize;
            i += 2 + len;
        }
        return None;
    }
    if b.len() >= 30 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return match &b[12..16] {
            b"VP8X" => Some((le24(b, 24)? + 1, le24(b, 27)? + 1)),
            b"VP8 " => Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF)),
            b"VP8L" => {
                let bits = u32::from(*b.get(21)?)
                    | u32::from(*b.get(22)?) << 8
                    | u32::from(*b.get(23)?) << 16
                    | u32::from(*b.get(24)?) << 24;
                Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
            }
            _ => None,
        };
    }
    None
}

pub fn size_of_file(path: &Path) -> Option<(u32, u32)> {
    let f = File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(HEAD).read_to_end(&mut buf).ok()?;
    size_from_bytes(&buf)
}

/// The local file for an image URL: its last path segment in `dir`.
pub fn local_file(dir: &Path, url: &str) -> Option<PathBuf> {
    let no_query = url.split(['?', '#']).next()?;
    let name = no_query.rsplit('/').next()?;
    if name.is_empty() || name.contains("..") || name.contains('\\') {
        return None;
    }
    let p = dir.join(name);
    if p.is_file() { Some(p) } else { None }
}

/// Add `image_width` and `image_height` to a record from its local
/// `image_url` file, located at the `image_url` cell, so the Google pack's
/// range rules can check them. Returns false when the record has an image
/// URL but no readable local file.
pub fn add_image_fields(rec: &mut Record, dir: &Path) -> bool {
    let Some(url_field) = rec.get("image_url").filter(|f| f.is_present()).cloned() else {
        return true;
    };
    let Some(path) = local_file(dir, url_field.value()) else {
        return false;
    };
    let Some((w, h)) = size_of_file(&path) else {
        return false;
    };
    for (name, v) in [("image_width", w), ("image_height", h)] {
        let mut f: Field = url_field.clone();
        f.raw = v.to_string();
        f.items = None;
        f.source = Arc::from(format!("{} ({})", url_field.source, name.replace('_', " ")).as_str());
        rec.set(Arc::from(name), f);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_gif_headers() {
        let mut png = vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(size_from_bytes(&png), Some((640, 480)));
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x20, 0x03, 0x58, 0x02]);
        assert_eq!(size_from_bytes(&gif), Some((800, 600)));
    }

    #[test]
    fn jpeg_frame_header() {
        // SOI, an APP0 segment of length 16, then SOF0 with height 300, width 400.
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        j.extend_from_slice(&[0u8; 14]);
        j.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0x2C, 0x01, 0x90, 0x03]);
        j.extend_from_slice(&[0u8; 12]);
        assert_eq!(size_from_bytes(&j), Some((400, 300)));
    }

    #[test]
    fn urls_to_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.png"), b"x").expect("write");
        assert!(local_file(dir.path(), "https://x.example/img/a.png?w=1").is_some());
        assert!(local_file(dir.path(), "https://x.example/img/b.png").is_none());
        assert!(local_file(dir.path(), "https://x.example/img/").is_none());
    }
}
