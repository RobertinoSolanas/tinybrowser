//! A from-scratch PNG encoder.
//!
//! Layout: signature · `IHDR` · `IDAT`(s) · `IEND`, every chunk length-prefixed
//! with its CRC-32 over type+data. The pixel data is filtered with filter type 0
//! (none) per scanline and compressed with our own zlib/DEFLATE
//! ([`tb_core::zlib_deflate`]).
//!
//! Colour type 6 (RGBA, 8 bit) — no palette, no interlacing, no filtering
//! cleverness: pages are large and mostly flat, which DEFLATE handles well.

use crate::framebuffer::Framebuffer;
use tb_core::{crc32, zlib_deflate, Color};

pub const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Encode a framebuffer as PNG bytes.
pub fn encode(fb: &Framebuffer) -> Vec<u8> {
    let mut out = Vec::with_capacity((fb.width * fb.height).max(1024) as usize);
    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR: width height bitdepth colortype compression filter interlace
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&fb.width.to_be_bytes());
    ihdr.extend_from_slice(&fb.height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // colour type: RGBA
    ihdr.push(0); // compression method (deflate)
    ihdr.push(0); // filter method
    ihdr.push(0); // interlace (none)
    chunk(&mut out, b"IHDR", &ihdr);

    // raw scanlines with a filter byte in front
    let mut raw = Vec::with_capacity((fb.width * fb.height * 4) as usize + fb.height as usize);
    for y in 0..fb.height {
        raw.push(0);
        for x in 0..fb.width {
            let c = fb.get(x, y);
            raw.extend_from_slice(&[c.r, c.g, c.b, c.a]);
        }
    }
    let compressed = zlib_deflate(&raw);
    // split into ~64 KiB IDAT chunks (readers accept any split)
    for piece in compressed.chunks(65_536) {
        chunk(&mut out, b"IDAT", piece);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encode with a `tEXt` comment chunk (provenance is nice to have).
pub fn encode_with_comment(fb: &Framebuffer, comment: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(4096);
    out.extend_from_slice(&PNG_SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&fb.width.to_be_bytes());
    ihdr.extend_from_slice(&fb.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let mut text = b"Software".to_vec();
    text.push(0);
    text.extend_from_slice(comment.as_bytes());
    chunk(&mut out, b"tEXt", &text);
    let mut raw = Vec::with_capacity((fb.width * fb.height * 4) as usize + fb.height as usize);
    for y in 0..fb.height {
        raw.push(0);
        for x in 0..fb.width {
            let c = fb.get(x, y);
            raw.extend_from_slice(&[c.r, c.g, c.b, c.a]);
        }
    }
    for piece in zlib_deflate(&raw).chunks(65_536) {
        chunk(&mut out, b"IDAT", piece);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// A minimal PNG *reader*, used by tests to verify what we wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct PngImage {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub comment: Option<String>,
    pub pixels: Vec<Color>,
}

/// Parse a PNG produced by [`encode`] (8-bit RGBA/RGB, non-interlaced).
pub fn decode(data: &[u8]) -> Result<PngImage, String> {
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        return Err("not a PNG (bad signature)".into());
    }
    let mut pos = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0;
    let mut color_type = 0;
    let mut interlace = 0;
    let mut comment = None;
    let mut idat: Vec<u8> = Vec::new();
    while pos + 8 <= data.len() {
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body_start = pos + 8;
        let body_end = body_start + len;
        if body_end + 4 > data.len() {
            return Err("truncated chunk".into());
        }
        let body = &data[body_start..body_end];
        let want_crc = u32::from_be_bytes([
            data[body_end],
            data[body_end + 1],
            data[body_end + 2],
            data[body_end + 3],
        ]);
        let mut buf = Vec::with_capacity(4 + body.len());
        buf.extend_from_slice(kind);
        buf.extend_from_slice(body);
        if crc32(&buf) != want_crc {
            return Err(format!(
                "CRC mismatch in chunk {}",
                String::from_utf8_lossy(kind)
            ));
        }
        match kind {
            b"IHDR" => {
                if body.len() != 13 {
                    return Err("bad IHDR length".into());
                }
                width = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                height = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                bit_depth = body[8];
                color_type = body[9];
                interlace = body[12];
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"tEXt" => {
                if let Some(nul) = body.iter().position(|b| *b == 0) {
                    comment = Some(String::from_utf8_lossy(&body[nul + 1..]).into_owned());
                }
            }
            b"IEND" => break,
            _ => {}
        }
        pos = body_end + 4;
    }
    if width == 0 || height == 0 {
        return Err("missing IHDR".into());
    }
    if interlace != 0 {
        return Err("interlaced PNG not supported".into());
    }
    if bit_depth != 8 {
        return Err(format!("bit depth {bit_depth} not supported"));
    }
    let bytes_per_pixel = match color_type {
        6 => 4usize,
        2 => 3usize,
        _ => return Err(format!("colour type {color_type} not supported")),
    };
    let raw = tb_core::zlib_inflate(&idat).map_err(|e| format!("inflate: {e}"))?;
    let stride = (width as usize) * bytes_per_pixel;
    let expect = (stride + 1) * height as usize;
    if raw.len() < expect {
        return Err(format!("short pixel data: {} < {expect}", raw.len()));
    }
    let mut pixels = vec![Color::TRANSPARENT; (width * height) as usize];
    for y in 0..height as usize {
        let filter = raw[y * (stride + 1)];
        if filter != 0 {
            return Err(format!("filter {filter} not supported (we write 0)"));
        }
        let row = &raw[y * (stride + 1) + 1..y * (stride + 1) + 1 + stride];
        for x in 0..width as usize {
            let (r, g, b, a) = if bytes_per_pixel == 4 {
                (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3])
            } else {
                (row[x * 3], row[x * 3 + 1], row[x * 3 + 2], 255)
            };
            pixels[y * width as usize + x] = Color::rgba(r, g, b, a);
        }
    }
    Ok(PngImage {
        width,
        height,
        bit_depth,
        color_type,
        comment,
        pixels,
    })
}

/// Write a PNG to disk (creating parent directories).
pub fn write_file(path: &std::path::Path, fb: &Framebuffer) -> tb_core::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(
        path,
        encode_with_comment(fb, "tinybrowser — hand written PNG encoder"),
    )
    .map_err(|e| tb_core::TbError::Io(format!("cannot write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Framebuffer {
        let mut fb = Framebuffer::new(6, 4);
        fb.clear(Color::WHITE);
        fb.fill_rect(Rect::new(0.0, 0.0, 3.0, 2.0), Color::RED);
        fb.blend_at(5, 3, Color::rgba(0, 0, 255, 128));
        fb
    }

    use tb_core::Rect;

    #[test]
    fn signature_and_chunks() {
        let bytes = encode(&sample());
        assert_eq!(&bytes[..8], &PNG_SIGNATURE);
        assert_eq!(&bytes[12..16], b"IHDR");
        let tail = &bytes[bytes.len() - 12..];
        assert_eq!(&tail[4..8], b"IEND");
        // the CRC of a chunk covers type + data; IEND is the empty payload
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn roundtrip_pixels() {
        let fb = sample();
        let bytes = encode(&fb);
        let img = decode(&bytes).expect("decode");
        assert_eq!(img.width, 6);
        assert_eq!(img.height, 4);
        assert_eq!(img.bit_depth, 8);
        assert_eq!(img.color_type, 6);
        for y in 0..4 {
            for x in 0..6 {
                assert_eq!(img.pixels[(y * 6 + x) as usize], fb.get(x, y), "{x},{y}");
            }
        }
    }

    #[test]
    fn comment_roundtrips() {
        let bytes = encode_with_comment(&sample(), "tinybrowser test");
        let img = decode(&bytes).unwrap();
        assert_eq!(img.comment.as_deref(), Some("tinybrowser test"));
    }

    #[test]
    fn crc_vectors() {
        // the standard check values
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(tb_core::adler32(b"123456789"), 0x091E_01DE);
        assert_eq!(tb_core::adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn broken_files_are_rejected() {
        let mut bytes = encode(&sample());
        assert!(decode(&bytes).is_ok());
        // damage a pixel byte inside IDAT -> CRC of that chunk must fail
        let idat = bytes.windows(4).position(|w| w == b"IDAT").unwrap();
        bytes[idat + 10] ^= 0xFF;
        assert!(decode(&bytes).is_err());
        assert!(decode(b"not a png at all").is_err());
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn large_frame_encodes_and_decodes() {
        let mut fb = Framebuffer::new(320, 240);
        fb.clear(Color::rgb(10, 20, 30));
        for i in 0..40 {
            fb.fill_rect(
                Rect::new(i as f32 * 4.0, i as f32 * 3.0, 20.0, 10.0),
                Color::GREEN,
            );
        }
        let bytes = encode(&fb);
        let img = decode(&bytes).unwrap();
        assert_eq!(img.pixels.len(), 320 * 240);
        // i=0 paints the rect x=0..20, y=0..10 -> (0,0) and (5,5) are green
        assert_eq!(img.pixels[0], Color::GREEN);
        assert_eq!(img.pixels[5 * 320 + 5], Color::GREEN);
        // far bottom right is untouched by every rect
        assert_eq!(img.pixels[230 * 320 + 300], Color::rgb(10, 20, 30));
        // our DEFLATE must actually compress flat pages
        assert!(bytes.len() < 320 * 240 / 2, "png size {}", bytes.len());
    }

    #[test]
    fn write_file_creates_directories() {
        let dir = std::env::temp_dir().join("tb-gfx-test");
        let path = dir.join("nested/shot.png");
        let _ = std::fs::remove_dir_all(&dir);
        write_file(&path, &sample()).expect("write");
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[..8], &PNG_SIGNATURE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
