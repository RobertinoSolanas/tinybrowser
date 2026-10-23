//! Decoded pictures: PNG in, RGBA out — with **no** image library.
//!
//! The wire format is walked in [`crate::png`]; this module does the parts an
//! `<img>` actually needs: inflate the `IDAT` stream, undo the per-scanline
//! filter (None/Sub/Up/Average/Paeth), unpack sub-byte samples, apply the
//! palette and `tRNS`, and hand back straight RGBA8 — including Adam7
//! interlacing, because progressive PNGs are common on real pages.
//!
//! What is *not* here: JPEG, WebP, APNG animation. Those need their own
//! decoders; an undecodable picture becomes the placeholder box, which is what
//! a browser does when a picture never arrives.

use crate::framebuffer::Framebuffer;
use crate::png;
use tb_core::{Color, Rect};

/// A decoded picture in RGBA8, row-major, 4 bytes per pixel.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Image {
    /// A page holds hundreds of these; dumping megabytes of pixels helps nobody.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Image {}×{} rgba8", self.width, self.height)
    }
}

/// Anything bigger than this is not a picture on a web page, it is an attack
/// on our heap (or a mistake) — either way: refuse and draw the placeholder.
pub const MAX_DIM: u32 = 8192;

impl Image {
    pub fn new(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            pixels: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// width / height, or 0 for an empty picture.
    pub fn aspect_ratio(&self) -> f32 {
        if self.height == 0 || self.width == 0 {
            0.0
        } else {
            self.width as f32 / self.height as f32
        }
    }

    pub fn get(&self, x: u32, y: u32) -> Color {
        if x >= self.width || y >= self.height {
            return Color::TRANSPARENT;
        }
        let i = ((y * self.width + x) as usize) * 4;
        Color::rgba(
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        )
    }

    /// Set one pixel (test fixtures and compositing do this).
    pub fn set(&mut self, x: u32, y: u32, p: [u8; 4]) {
        let i = ((y * self.width + x) as usize) * 4;
        self.pixels[i..i + 4].copy_from_slice(&p);
    }

    /// The picture scaled to `(w, h)` — nearest neighbour, which is the right
    /// choice for a 5×7-font browser: no filtering allocations, and the pixels
    /// stay pixels.
    pub fn scaled(&self, w: u32, h: u32) -> Image {
        if w == 0 || h == 0 {
            return Image::new(0, 0);
        }
        if w == self.width && h == self.height {
            return self.clone();
        }
        let mut out = Image::new(w, h);
        for y in 0..h {
            let sy =
                ((y as u64 * self.height as u64) / h as u64).min(self.height as u64 - 1) as u32;
            for x in 0..w {
                let sx =
                    ((x as u64 * self.width as u64) / w as u64).min(self.width as u64 - 1) as u32;
                let i = (sy as usize * self.width as usize + sx as usize) * 4;
                let p = [
                    self.pixels[i],
                    self.pixels[i + 1],
                    self.pixels[i + 2],
                    self.pixels[i + 3],
                ];
                out.set(x, y, p);
            }
        }
        out
    }
}

/// Decode a PNG into RGBA8.
pub fn from_png(data: &[u8]) -> Result<Image, String> {
    let chunks = png::read_chunks(data)?;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut idat: Vec<u8> = Vec::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut transparency: Vec<u8> = Vec::new();

    for (kind, body) in &chunks {
        match kind {
            b"IHDR" => {
                if body.len() != 13 {
                    return Err("bad IHDR length".into());
                }
                width = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                height = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                bit_depth = body[8];
                color_type = body[9];
                if body[10] != 0 {
                    return Err(format!("compression method {} unsupported", body[10]));
                }
                if body[11] != 0 {
                    return Err("unknown filter method".into());
                }
                interlace = body[12];
            }
            b"PLTE" => {
                palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => transparency = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            _ => {}
        }
    }
    if width == 0 || height == 0 {
        return Err("missing IHDR".into());
    }
    if width > MAX_DIM || height > MAX_DIM {
        return Err(format!("picture too large: {width}×{height}"));
    }
    let channels = match color_type {
        0 => 1usize, // greyscale
        2 => 3,      // truecolour
        3 => 1,      // indexed
        4 => 2,      // greyscale + alpha
        6 => 4,      // truecolour + alpha
        other => return Err(format!("colour type {other} unsupported")),
    };
    if !matches!(bit_depth, 1 | 2 | 4 | 8 | 16) {
        return Err(format!("bit depth {bit_depth} unsupported"));
    }
    if color_type == 3 && palette.is_empty() {
        return Err("indexed PNG without PLTE".into());
    }
    if !(interlace == 0 || interlace == 1) {
        return Err(format!("interlace method {interlace} unknown"));
    }

    let raw = tb_core::zlib_inflate(&idat).map_err(|e| format!("inflate: {e}"))?;

    let bits_per_row = width as usize * channels * bit_depth as usize;
    let stride = bits_per_row.div_ceil(8);
    // bytes per *pixel* as the filter sees it (never below one, per the spec)
    let bpp = ((channels * bit_depth as usize) / 8).max(1);

    let mut img = Image::new(width, height);
    if interlace == 0 {
        let need = (stride + 1) * height as usize;
        if raw.len() < need {
            return Err(format!("short pixel data: {} < {need}", raw.len()));
        }
        let scan = unfilter(&raw[..need], stride, bpp)?;
        fill_rect(
            &mut img,
            &scan,
            stride,
            0,
            0,
            1,
            1,
            color_type,
            bit_depth,
            &palette,
            &transparency,
        )?;
    } else {
        // Adam7: seven independent sub-images, each with its own filter bytes
        const PASSES: [(u32, u32, u32, u32); 7] = [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ];
        let mut pos = 0usize;
        for (sx, sy, dx, dy) in PASSES {
            let pw = width.saturating_sub(sx).div_ceil(dx);
            let ph = height.saturating_sub(sy).div_ceil(dy);
            if pw == 0 || ph == 0 {
                continue;
            }
            let pbits = pw as usize * channels * bit_depth as usize;
            let pstride = pbits.div_ceil(8);
            let need = (pstride + 1) * ph as usize;
            if pos + need > raw.len() {
                return Err("short interlaced pixel data".into());
            }
            let scan = unfilter(&raw[pos..pos + need], pstride, bpp)?;
            pos += need;
            fill_rect(
                &mut img,
                &scan,
                pstride,
                sx,
                sy,
                dx,
                dy,
                color_type,
                bit_depth,
                &palette,
                &transparency,
            )?;
        }
    }
    Ok(img)
}

/// Undo the scanline filters of one (sub-)image: `raw` holds `1 + stride`
/// bytes per row, the result is the plain sample bytes.
fn unfilter(raw: &[u8], stride: usize, bpp: usize) -> Result<Vec<u8>, String> {
    let rows = raw.len() / (stride + 1);
    let mut out = vec![0u8; rows * stride];
    for (row, line) in raw.chunks_exact(stride + 1).enumerate() {
        let filter = line[0];
        let src = &line[1..];
        // the row above is read while this row is written: split once and the
        // filters stay a single pass
        let (prev_rows, cur_rows) = out.split_at_mut(row * stride);
        let cur = &mut cur_rows[..stride];
        let prev: &[u8] = if row == 0 {
            &[]
        } else {
            let n = prev_rows.len();
            &prev_rows[n - stride..n]
        };
        match filter {
            0 => cur.copy_from_slice(src),
            1 => {
                // Sub: add the pixel to the left
                for i in 0..stride {
                    let left = if i >= bpp { cur[i - bpp] } else { 0 };
                    cur[i] = src[i].wrapping_add(left);
                }
            }
            2 => {
                // Up: add the pixel above
                for i in 0..stride {
                    let up = prev.get(i).copied().unwrap_or(0);
                    cur[i] = src[i].wrapping_add(up);
                }
            }
            3 => {
                // Average: add the mean of left and above
                for i in 0..stride {
                    let left = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
                    let up = prev.get(i).copied().unwrap_or(0) as u16;
                    cur[i] = src[i].wrapping_add(((left + up) / 2) as u8);
                }
            }
            4 => {
                // Paeth: add whichever neighbour is closest to left+up−ul
                for i in 0..stride {
                    let a = if i >= bpp { cur[i - bpp] as i32 } else { 0 };
                    let b = prev.get(i).copied().unwrap_or(0) as i32;
                    let c = if i >= bpp {
                        prev.get(i - bpp).copied().unwrap_or(0) as i32
                    } else {
                        0
                    };
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    let pred = if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    };
                    cur[i] = src[i].wrapping_add(pred as u8);
                }
            }
            other => return Err(format!("unknown filter type {other}")),
        }
    }
    Ok(out)
}

/// Write the samples of one (sub-)row set into `img`, honouring the Adam7 step.
#[allow(clippy::too_many_arguments)]
fn fill_rect(
    img: &mut Image,
    scan: &[u8],
    stride: usize,
    start_x: u32,
    start_y: u32,
    step_x: u32,
    step_y: u32,
    color_type: u8,
    bit_depth: u8,
    palette: &[[u8; 3]],
    transparency: &[u8],
) -> Result<(), String> {
    let rows = scan.len() / stride;
    for r in 0..rows as u32 {
        let y = start_y + r * step_y;
        if y >= img.height {
            break;
        }
        let row = &scan[r as usize * stride..(r as usize + 1) * stride];
        for (i, x0) in (start_x..).step_by(step_x as usize).enumerate() {
            if x0 >= img.width {
                break;
            }
            let p = sample(row, i, color_type, bit_depth, palette, transparency);
            img.set(x0, y, p);
        }
    }
    Ok(())
}

/// One pixel out of a scanline, as RGBA8.
fn sample(
    row: &[u8],
    index: usize,
    color_type: u8,
    bit_depth: u8,
    palette: &[[u8; 3]],
    transparency: &[u8],
) -> [u8; 4] {
    let channels = match color_type {
        2 => 3usize,
        4 => 2,
        6 => 4,
        _ => 1,
    };
    // a palette sample *is* an index — scaling it would be wrong
    if color_type == 3 {
        let idx = read_raw_index(row, index, bit_depth) as usize;
        let rgb = palette.get(idx).copied().unwrap_or([0, 0, 0]);
        return [
            rgb[0],
            rgb[1],
            rgb[2],
            transparency.get(idx).copied().unwrap_or(255),
        ];
    }
    // read `channels` samples, scaled to 8 bits
    let mut s = [255u8; 4];
    for (c, slot) in s.iter_mut().take(channels).enumerate() {
        *slot = read_sample(row, index * channels + c, bit_depth);
    }
    match color_type {
        0 => {
            // a grey colour key becomes fully transparent
            let a = grey_key(s[0], transparency);
            [s[0], s[0], s[0], a]
        }
        2 => {
            let a = grey_key3([s[0], s[1], s[2]], transparency);
            [s[0], s[1], s[2], a]
        }
        4 => [s[0], s[0], s[0], s[1]],
        _ => [s[0], s[1], s[2], s[3]],
    }
}

/// The raw value of sample `n` — no bit-depth scaling (palette indices).
fn read_raw_index(row: &[u8], n: usize, bit_depth: u8) -> u8 {
    match bit_depth {
        8 => row.get(n).copied().unwrap_or(0),
        16 => row.get(n * 2).copied().unwrap_or(0),
        depth => {
            let bits = depth as usize;
            let bit = n * bits;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            let shift = 8 - (bit % 8) - bits;
            let mask = (1u16 << bits) - 1;
            ((byte >> shift) as u16 & mask) as u8
        }
    }
}

/// Sample `n` of a scanline as 8 bits (16-bit loses its low byte: the browser
/// we are has 8-bit pixels).
fn read_sample(row: &[u8], n: usize, bit_depth: u8) -> u8 {
    match bit_depth {
        8 => row.get(n).copied().unwrap_or(0),
        16 => row.get(n * 2).copied().unwrap_or(0),
        depth => {
            let bits = depth as usize;
            let bit = n * bits;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            let shift = 8 - (bit % 8) - bits;
            let raw = (byte >> shift) as u16 & ((1 << bits) - 1);
            // scale 1/2/4-bit up to 8 bits (0b101 → 0b10110101)
            let mul = 255 / ((1u16 << bits) - 1) as u8;
            (raw as u8) * mul
        }
    }
}

fn grey_key(v: u8, transparency: &[u8]) -> u8 {
    if transparency.len() >= 2 {
        let key = (transparency[0] as u16) << 8 | transparency[1] as u16;
        // the key is 16-bit; compare against the high byte we kept
        if v as u16 == (key >> 8) {
            return 0;
        }
    }
    255
}

fn grey_key3(rgb: [u8; 3], transparency: &[u8]) -> u8 {
    if transparency.len() >= 6 {
        let key = [transparency[1], transparency[3], transparency[5]];
        if rgb == key {
            return 0;
        }
    }
    255
}

/// Paint `img` into `dst` (device pixels), alpha blended and clipped.
pub fn blit(fb: &mut Framebuffer, img: &Image, dst: Rect, clip: Option<Rect>) {
    if img.width == 0 || img.height == 0 || dst.width <= 0.0 || dst.height <= 0.0 {
        return;
    }
    let x0 = dst.left().floor() as i32;
    let y0 = dst.top().floor() as i32;
    let w = dst.width.round().max(1.0) as i32;
    let h = dst.height.round().max(1.0) as i32;
    for py in 0..h {
        for px in 0..w {
            let (dx, dy) = (x0 + px, y0 + py);
            if let Some(c) = clip {
                if !c.contains(tb_core::Point::new(dx as f32 + 0.5, dy as f32 + 0.5)) {
                    continue;
                }
            }
            // nearest neighbour over the destination grid
            let sx = (px as u64 * img.width as u64 / w as u64).min(img.width as u64 - 1) as u32;
            let sy = (py as u64 * img.height as u64 / h as u64).min(img.height as u64 - 1) as u32;
            let c = img.get(sx, sy);
            if c.a == 0 {
                continue;
            }
            fb.blend_at(dx, dy, c);
        }
    }
}
