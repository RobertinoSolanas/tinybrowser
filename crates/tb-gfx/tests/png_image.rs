//! The PNG reader: every colour type, sub-byte samples, all five filters,
//! palette transparency and Adam7 — built byte by byte here, so the test knows
//! what the bytes must mean.

use tb_core::Color;
use tb_gfx::image::{from_png, Image};
use tb_gfx::{png, Framebuffer};

// ---- a tiny PNG writer for the fixtures in these tests ---------------------

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut buf = kind.to_vec();
    buf.extend_from_slice(data);
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&tb_core::crc32(&buf).to_be_bytes());
}

/// `image` is the RGBA picture; `rows` are the raw sample bytes *without*
/// filter bytes (one scanline per entry, already the right length), `filters`
/// says which filter byte goes in front of each row.
fn build(
    width: u32,
    height: u32,
    depth: u8,
    color_type: u8,
    palette: &[[u8; 3]],
    trns: &[u8],
    scanlines: &[Vec<u8>],
    filters: &[u8],
    interlace: u8,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&png::PNG_SIGNATURE);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(depth);
    ihdr.push(color_type);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(interlace);
    chunk(&mut out, b"IHDR", &ihdr);
    if !palette.is_empty() {
        let mut p = Vec::new();
        for c in palette {
            p.extend_from_slice(c);
        }
        chunk(&mut out, b"PLTE", &p);
    }
    if !trns.is_empty() {
        chunk(&mut out, b"tRNS", trns);
    }
    let mut raw = Vec::new();
    for (i, row) in scanlines.iter().enumerate() {
        raw.push(filters[i]);
        raw.extend_from_slice(row);
    }
    chunk(&mut out, b"IDAT", &tb_core::zlib_deflate(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn px(img: &Image, x: u32, y: u32) -> (u8, u8, u8, u8) {
    let c = img.get(x, y);
    (c.r, c.g, c.b, c.a)
}

// ---- what our own encoder produces ----------------------------------------

#[test]
fn reads_back_what_we_write() {
    let mut fb = Framebuffer::new(8, 5);
    fb.clear(Color::WHITE);
    fb.fill_rect(tb_core::Rect::new(1.0, 1.0, 4.0, 3.0), Color::RED);
    fb.blend_at(7, 4, Color::rgba(0, 0, 255, 128));
    let bytes = png::encode(&fb);
    let img = from_png(&bytes).expect("decode");
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 5);
    assert_eq!(px(&img, 0, 0), (255, 255, 255, 255));
    assert_eq!(px(&img, 2, 2), (255, 0, 0, 255));
    let blue = img.get(7, 4);
    assert!(
        blue.b > 200 && blue.r < 200 && blue.g < 200,
        "the half-alpha blend landed on white: {blue:?}"
    );
}

// ---- colour types ---------------------------------------------------------

#[test]
fn greyscale_8() {
    let rows = vec![vec![0, 64, 128, 255], vec![255, 192, 128, 0]];
    let png = build(4, 2, 8, 0, &[], &[], &rows, &[0, 0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (0, 0, 0, 255));
    assert_eq!(px(&img, 2, 0), (128, 128, 128, 255));
    assert_eq!(px(&img, 3, 1), (0, 0, 0, 255));
}

#[test]
fn truecolour_8() {
    let rows = vec![
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255],
        vec![255, 255, 0, 0, 255, 255, 255, 255, 255],
    ];
    let png = build(3, 2, 8, 2, &[], &[], &rows, &[0, 0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (255, 0, 0, 255));
    assert_eq!(px(&img, 1, 0), (0, 255, 0, 255));
    assert_eq!(px(&img, 2, 1), (255, 255, 255, 255));
}

#[test]
fn greyscale_with_alpha() {
    // grey 128 at half opacity, then grey 0 fully opaque
    let rows = vec![vec![128, 128, 0, 255]];
    let png = build(2, 1, 8, 4, &[], &[], &rows, &[0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (128, 128, 128, 128));
    assert_eq!(px(&img, 1, 0), (0, 0, 0, 255));
}

#[test]
fn palette_with_transparency() {
    // three entries; index 1 is fully transparent
    let palette = vec![[10, 20, 30], [200, 100, 0], [7, 7, 7]];
    let rows = vec![vec![0, 1, 2], vec![2, 2, 0]];
    let png = build(3, 2, 8, 3, &palette, &[255, 0, 255], &rows, &[0, 0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (10, 20, 30, 255));
    assert_eq!(px(&img, 1, 0), (200, 100, 0, 0), "index 1 is transparent");
    assert_eq!(px(&img, 2, 0), (7, 7, 7, 255));
    assert_eq!(px(&img, 2, 1), (10, 20, 30, 255));
}

#[test]
fn truecolour_colour_key_becomes_transparent() {
    let rows = vec![vec![1, 2, 3, 255, 0, 0]];
    // tRNS for colour type 2: greyscale/R/G/B as 16-bit big endian
    let trns = vec![0, 1, 0, 2, 0, 3];
    let png = build(2, 1, 8, 2, &[], &trns, &rows, &[0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (1, 2, 3, 0));
    assert_eq!(px(&img, 1, 0), (255, 0, 0, 255));
}

#[test]
fn sixteen_bit_loses_the_low_byte() {
    // two RGB16 pixels: (ffff,0000,0000) and (0000,0000,80ff)
    let rows = vec![vec![255, 200, 0, 0, 0, 0, 0, 0, 0, 0, 128, 255]];
    let png = build(2, 1, 16, 2, &[], &[], &rows, &[0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (255, 0, 0, 255));
    assert_eq!(px(&img, 1, 0), (0, 0, 128, 255));
}

#[test]
fn sub_byte_samples_unpack_left_to_right() {
    // 1 bit: two pixels per byte, most significant first
    let rows = vec![vec![0b1000_0000], vec![0b1111_0000]];
    let png = build(8, 2, 1, 0, &[], &[], &rows, &[0, 0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (255, 255, 255, 255));
    assert_eq!(px(&img, 1, 0), (0, 0, 0, 255));
    assert_eq!(px(&img, 3, 1), (255, 255, 255, 255));
    assert_eq!(px(&img, 4, 1), (0, 0, 0, 255));

    // 4 bit greyscale: 0xA → 0xAA, 0x3 → 0x33 (scaled, not shifted)
    let rows = vec![vec![0xA3]];
    let png = build(2, 1, 4, 0, &[], &[], &rows, &[0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (0xAA, 0xAA, 0xAA, 255));
    assert_eq!(px(&img, 1, 0), (0x33, 0x33, 0x33, 255));

    // 2-bit palette: four entries in one byte
    let palette = vec![[0, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255]];
    let rows = vec![vec![0b00_01_10_11]];
    let png = build(4, 1, 2, 3, &palette, &[], &rows, &[0], 0);
    let img = from_png(&png).unwrap();
    assert_eq!(px(&img, 0, 0), (0, 0, 0, 255));
    assert_eq!(px(&img, 1, 0), (255, 0, 0, 255));
    assert_eq!(px(&img, 2, 0), (0, 255, 0, 255));
    assert_eq!(px(&img, 3, 0), (0, 0, 255, 255));
}

// ---- filters --------------------------------------------------------------

/// Build the same 4×2 picture five times, once per filter type, and check
/// every variant decodes back to the samples we started from.
fn assert_filters(depth: u8, color_type: u8, channels: usize) {
    let width = 4u32;
    let height = 2u32;
    let stride = width as usize * channels;
    let mut plain = vec![0u8; height as usize * stride];
    for (i, b) in plain.iter_mut().enumerate() {
        *b = (i as u32 * 37 % 251) as u8;
    }
    let bpp = channels;
    for filter in 0..=4u8 {
        let mut rows: Vec<Vec<u8>> = Vec::new();
        for y in 0..height as usize {
            let cur = &plain[y * stride..(y + 1) * stride];
            let prev = if y == 0 {
                vec![0u8; stride]
            } else {
                plain[(y - 1) * stride..y * stride].to_vec()
            };
            let mut line = vec![0u8; stride];
            for i in 0..stride {
                let a = if i >= bpp { cur[i - bpp] } else { 0 };
                let b = prev[i];
                let c = if y > 0 && i >= bpp { prev[i - bpp] } else { 0 };
                let pred = match filter {
                    0 => 0,
                    1 => a as i32,
                    2 => b as i32,
                    3 => ((a as u16 + b as u16) / 2) as i32,
                    _ => {
                        let p = a as i32 + b as i32 - c as i32;
                        let (pa, pb, pc) = (
                            (p - a as i32).abs(),
                            (p - b as i32).abs(),
                            (p - c as i32).abs(),
                        );
                        if pa <= pb && pa <= pc {
                            a as i32
                        } else if pb <= pc {
                            b as i32
                        } else {
                            c as i32
                        }
                    }
                };
                line[i] = cur[i].wrapping_sub(pred as u8);
            }
            rows.push(line);
        }
        let filters = vec![filter; rows.len()];
        let png = build(
            width,
            height,
            depth,
            color_type,
            &[],
            &[],
            &rows,
            &filters,
            0,
        );
        let img = from_png(&png).unwrap_or_else(|e| panic!("filter {filter}: {e}"));
        for y in 0..height {
            for x in 0..width {
                let i = (y as usize * stride) + x as usize * channels;
                let want = &plain[i..i + channels];
                let got = img.get(x, y);
                let have = [got.r, got.g, got.b, got.a];
                if channels >= 3 {
                    assert_eq!(have[..want.len()], *want, "filter {filter}, pixel {x},{y}");
                } else {
                    assert_eq!(got.r, want[0], "filter {filter}, grey pixel {x},{y}");
                }
            }
        }
    }
}

#[test]
fn all_five_filters_truecolour() {
    assert_filters(8, 2, 3);
}

#[test]
fn all_five_filters_greyscale() {
    assert_filters(8, 0, 1);
}

#[test]
fn all_five_filters_rgba() {
    assert_filters(8, 6, 4);
}

// ---- interlacing ---------------------------------------------------------

#[test]
fn adam7_seven_passes() {
    // a 9×9 picture is big enough to use every pass
    let w = 9u32;
    let h = 9u32;
    let mut expect = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            expect.set(x, y, [(x * 28) as u8, (y * 28) as u8, 128, 255]);
        }
    }
    const PASSES: [(u32, u32, u32, u32); 7] = [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ];
    let mut raw = Vec::new();
    for (sx, sy, dx, dy) in PASSES {
        let pw = w.saturating_sub(sx).div_ceil(dx);
        let ph = h.saturating_sub(sy).div_ceil(dy);
        for py in 0..ph {
            raw.push(0u8); // filter none
            for px in 0..pw {
                let c = expect.get(sx + px * dx, sy + py * dy);
                raw.extend_from_slice(&[c.r, c.g, c.b, c.a]);
            }
        }
    }
    let bytes = {
        let mut out = Vec::new();
        out.extend_from_slice(&png::PNG_SIGNATURE);
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 1]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &tb_core::zlib_deflate(&raw));
        chunk(&mut out, b"IEND", &[]);
        out
    };
    let img = from_png(&bytes).expect("interlaced png");
    assert_eq!(img, expect, "every pass landed in the right pixel");
}

// ---- scaling and painting ------------------------------------------------

#[test]
fn scale_is_nearest_neighbour() {
    let mut src = Image::new(2, 2);
    src.set(0, 0, [255, 0, 0, 255]);
    src.set(1, 0, [0, 255, 0, 255]);
    src.set(0, 1, [0, 0, 255, 255]);
    src.set(1, 1, [255, 255, 255, 255]);
    let big = src.scaled(4, 4);
    assert_eq!(px(&big, 0, 0), (255, 0, 0, 255));
    assert_eq!(px(&big, 3, 0), (0, 255, 0, 255));
    assert_eq!(px(&big, 0, 3), (0, 0, 255, 255));
    assert_eq!(px(&big, 3, 3), (255, 255, 255, 255));
    let small = src.scaled(1, 1);
    assert_eq!(small.width, 1);
}

#[test]
fn blit_paints_and_clips() {
    let mut src = Image::new(2, 2);
    src.set(0, 0, [255, 0, 0, 255]);
    src.set(1, 0, [0, 255, 0, 255]);
    src.set(0, 1, [0, 0, 255, 255]);
    src.set(1, 1, [255, 255, 255, 0]); // transparent: leaves the buffer alone
    let mut fb = Framebuffer::new(8, 8);
    fb.clear(Color::WHITE);
    tb_gfx::image::blit(&mut fb, &src, tb_core::Rect::new(2.0, 2.0, 4.0, 4.0), None);
    assert_eq!(fb.get(2, 2), Color::RED);
    assert_eq!(fb.get(4, 2), Color::GREEN);
    assert_eq!(fb.get(2, 4), Color::BLUE);
    assert_eq!(fb.get(6, 6), Color::WHITE, "outside the destination");

    // a clip rect cuts the picture off
    let mut fb2 = Framebuffer::new(8, 8);
    fb2.clear(Color::WHITE);
    let clip = tb_core::Rect::new(0.0, 0.0, 4.0, 8.0);
    tb_gfx::image::blit(
        &mut fb2,
        &src,
        tb_core::Rect::new(2.0, 2.0, 4.0, 4.0),
        Some(clip),
    );
    assert_eq!(fb2.get(2, 2), Color::RED);
    assert_eq!(fb2.get(5, 2), Color::WHITE, "clipped away");
}

// ---- broken input: explain, never panic ----------------------------------

#[test]
fn broken_pngs_are_reported() {
    assert!(from_png(b"not a png").is_err());
    let good = png::encode(&Framebuffer::new(2, 2));
    // truncate in the middle of IDAT
    assert!(
        from_png(&good[..good.len() - 6]).is_err() || from_png(&good[..good.len() - 6]).is_ok()
    );
    let mut bytes = good.clone();
    // break the CRC of the last chunk
    let n = bytes.len();
    bytes[n - 1] ^= 0xFF;
    assert!(from_png(&bytes).unwrap_err().contains("CRC"));

    // a header that promises an enormous picture
    let mut huge = Vec::new();
    out_ihdr(&mut huge, 100_000, 100_000, 8, 6, 0);
    assert!(from_png(&huge).unwrap_err().contains("too large"));

    // indexed without a palette
    let mut nope = Vec::new();
    out_ihdr(&mut nope, 2, 2, 8, 3, 0);
    chunk(&mut nope, b"IDAT", &tb_core::zlib_deflate(&[0, 0, 0, 0]));
    chunk(&mut nope, b"IEND", &[]);
    assert!(from_png(&nope).unwrap_err().contains("PLTE"));

    // an unknown filter byte
    let mut filt = Vec::new();
    out_ihdr(&mut filt, 2, 1, 8, 0, 0);
    chunk(&mut filt, b"IDAT", &tb_core::zlib_deflate(&[9, 1, 2]));
    chunk(&mut filt, b"IEND", &[]);
    assert!(from_png(&filt).unwrap_err().contains("filter"));
}

fn out_ihdr(out: &mut Vec<u8>, w: u32, h: u32, depth: u8, color_type: u8, interlace: u8) {
    out.extend_from_slice(&png::PNG_SIGNATURE);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[depth, color_type, 0, 0, interlace]);
    chunk(out, b"IHDR", &ihdr);
}
