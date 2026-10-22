//! # tb-gfx — software rasterizer, bitmap font, PNG encoder
//!
//! Everything needed to turn boxes and text into pixels, with **no** image or
//! font library:
//!
//! * [`Framebuffer`] — ARGB8 buffer, alpha compositing, rect/line/stroke/round
//!   filling, per-call clipping (the layout passes the `overflow:hidden` clip).
//! * [`font::Font`] — a hand written 5×7 bitmap font (ASCII 32–126 plus German
//!   umlauts, dashes, quotes, `€`), integer scaling, synthesised bold/italic.
//! * [`png`] — PNG encoder (signature, `IHDR`, `IDAT`, `IEND`, CRC-32) on top of
//!   our own zlib/DEFLATE from `tb-core`, plus a small reader for tests.
//! * [`draw`] — the primitives the render phase uses: box background + borders,
//!   text runs, `<img>` placeholders.
//!
//! ```
//! use tb_gfx::{Framebuffer, png, draw::TextPaint};
//! let mut fb = Framebuffer::new(120, 24);
//! fb.clear(tb_core::Color::WHITE);
//! TextPaint::default().draw(&mut fb, 4.0, 16.0, "tinybrowser", None);
//! let bytes = png::encode(&fb);
//! assert_eq!(&bytes[..8], &png::PNG_SIGNATURE);
//! let back = png::decode(&bytes).unwrap();
//! assert_eq!(back.width, 120);
//! ```

pub mod draw;
pub mod font;
pub mod font_data;
pub mod framebuffer;
pub mod png;

pub use draw::{BoxPaint, TextPaint};
pub use font::{metrics, Font};
pub use framebuffer::{Dash, Framebuffer};
pub use png::{decode as png_decode, encode as png_encode, PNG_SIGNATURE};

/// The whole-page screenshot convenience: encode and optionally write.
pub fn screenshot(fb: &Framebuffer, path: Option<&std::path::Path>) -> tb_core::Result<Vec<u8>> {
    let bytes = png::encode_with_comment(
        fb,
        "tinybrowser 0.1 — hand written HTML/CSS/JS engines, own PNG encoder",
    );
    if let Some(path) = path {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| tb_core::TbError::Io(format!("cannot create {}: {e}", dir.display())))?;
            }
        }
        std::fs::write(path, &bytes)
            .map_err(|e| tb_core::TbError::Io(format!("cannot write {}: {e}", path.display())))?;
    }
    Ok(bytes)
}

/// A tiny demo page (used by `cargo run -p tb-gfx --example shot` and tests):
/// proves font + box + PNG work together without the browser being finished.
pub fn demo_framebuffer() -> Framebuffer {
    use tb_core::{Color, Rect};
    let mut fb = Framebuffer::new(320, 120);
    fb.clear(Color::rgb(0xf2, 0xf2, 0xf2));
    let header = BoxPaint {
        background: Color::rgb(0x11, 0x11, 0x44),
        ..Default::default()
    };
    header.paint(&mut fb, Rect::new(0.0, 0.0, 320.0, 34.0), None);
    TextPaint {
        color: Color::WHITE,
        size: 21.0,
        bold: true,
        ..Default::default()
    }
    .draw(&mut fb, 10.0, 24.0, "tinybrowser", None);
    let card = BoxPaint {
        background: Color::WHITE,
        border_widths: [1.0; 4],
        border_colors: [Color::rgb(0xcc, 0xcc, 0xcc); 4],
        border_dash: Dash::Solid,
        radius: 6.0,
    };
    card.paint(&mut fb, Rect::new(10.0, 44.0, 300.0, 60.0), None);
    TextPaint {
        color: Color::rgb(0x22, 0x22, 0x22),
        size: 14.0,
        ..Default::default()
    }
    .draw(&mut fb, 20.0, 66.0, "Hallo taz - eigene Engine!", None);
    TextPaint {
        color: Color::rgb(0x00, 0x33, 0x99),
        size: 14.0,
        underline: true,
        ..Default::default()
    }
    .draw(&mut fb, 20.0, 88.0, "https://taz.de/", None);
    fb
}

#[cfg(test)]
mod tests {
    use super::*;
    use tb_core::Color;

    #[test]
    fn demo_renders_and_encodes() {
        let fb = demo_framebuffer();
        assert!(!fb.is_blank());
        assert!(fb.count_non_matching(Color::rgb(0xf2, 0xf2, 0xf2)) > 3000);
        let bytes = screenshot(&fb, None).unwrap();
        let img = png_decode(&bytes).unwrap();
        assert_eq!(img.width, 320);
        assert_eq!(img.height, 120);
        // header really is at the top, and the link row has blue ink
        assert_eq!(img.pixels[5], Color::rgb(0x11, 0x11, 0x44));
        let blue = img
            .pixels
            .iter()
            .filter(|c| **c == Color::rgb(0x00, 0x33, 0x99))
            .count();
        assert!(blue > 50, "link pixels: {blue}");
    }

    #[test]
    fn screenshot_writes_a_file() {
        let path = std::env::temp_dir().join("tb-gfx-shot.png");
        let _ = std::fs::remove_file(&path);
        let bytes = screenshot(&demo_framebuffer(), Some(&path)).unwrap();
        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, bytes);
        assert_eq!(&on_disk[..8], &PNG_SIGNATURE);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn font_metrics_match_layout_expectations() {
        let (ascent, descent, line) = metrics(16.0);
        assert_eq!((ascent, descent, line), (12.0, 2.0, 16.0));
        assert!(metrics(0.0).2 > 0.0, "never a zero line box");
    }
}
