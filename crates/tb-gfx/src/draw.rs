//! Paint primitives used by the render phase.
//!
//! Deliberately independent of `tb-style`: the renderer maps a
//! `ComputedStyle` onto these small structs, so `tb-gfx` stays a leaf of the
//! presentation context and can be tested (and reused) on its own.

use crate::framebuffer::{Dash, Framebuffer};
use tb_core::{Color, Rect};

/// Background + border of one box.
#[derive(Debug, Clone)]
pub struct BoxPaint {
    pub background: Color,
    pub border_widths: [f32; 4],
    pub border_colors: [Color; 4],
    pub border_dash: Dash,
    pub radius: f32,
}

impl Default for BoxPaint {
    fn default() -> Self {
        BoxPaint {
            background: Color::TRANSPARENT,
            border_widths: [0.0; 4],
            border_colors: [Color::BLACK; 4],
            border_dash: Dash::Solid,
            radius: 0.0,
        }
    }
}

impl BoxPaint {
    /// Paint background and border of a box given in *border box* coordinates.
    pub fn paint(&self, fb: &mut Framebuffer, rect: Rect, clip: Option<Rect>) {
        if rect.is_empty() {
            return;
        }
        if self.background.a > 0 {
            if self.radius > 0.0 {
                fb.fill_round_rect(rect, self.radius, self.background, clip);
            } else {
                fb.fill_rect_clip(rect, clip, self.background);
            }
        }
        let has_border = self.border_widths.iter().any(|w| *w > 0.0);
        if has_border {
            fb.stroke_rect(rect, self.border_widths, self.border_colors, self.border_dash, clip);
        }
    }
}

/// How a run of text is drawn.
#[derive(Debug, Clone)]
pub struct TextPaint {
    pub color: Color,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub letter_spacing: f32,
}

impl Default for TextPaint {
    fn default() -> Self {
        TextPaint {
            color: Color::BLACK,
            size: 16.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            letter_spacing: 0.0,
        }
    }
}

impl TextPaint {
    /// Draw one line of text; `(x, baseline)` in device pixels. Returns the
    /// width actually used.
    pub fn draw(
        &self,
        fb: &mut Framebuffer,
        x: f32,
        baseline: f32,
        text: &str,
        clip: Option<Rect>,
    ) -> f32 {
        let font = crate::font::Font::system();
        let w = if self.letter_spacing != 0.0 {
            font.draw_spaced(
                fb, x, baseline, text, self.size, self.bold, self.letter_spacing, self.color, clip,
            )
        } else if self.italic {
            self.draw_oblique(fb, x, baseline, text, clip)
        } else {
            font.draw(fb, x, baseline, text, self.size, self.bold, self.color, clip)
        };
        if self.underline {
            self.draw_decoration(fb, x, baseline, w, 1.0, clip);
        }
        if self.strike {
            self.draw_decoration(fb, x, baseline - font.ascent(self.size) * 0.35, w, 1.0, clip);
        }
        w
    }

    /// Poor-man's italic: shift each glyph row a bit further right per row.
    fn draw_oblique(&self, fb: &mut Framebuffer, x: f32, baseline: f32, text: &str, clip: Option<Rect>) -> f32 {
        let font = crate::font::Font::system();
        let scale = font.scale(self.size);
        let mut pen_x = x.round() as i32;
        let top = baseline.round() as i32 - font.ascent(self.size) as i32;
        for c in text.chars() {
            let adv = ((crate::font_data::GLYPH_WIDTH + 1) * scale as usize) as i32;
            if c == '\t' {
                pen_x += (adv * 4) as i32;
                continue;
            }
            if c != ' ' {
                if let Some(rows) = font.rows(c) {
                    for (ry, bits) in rows.iter().enumerate() {
                        if *bits == 0 {
                            continue;
                        }
                        // shear: one pixel right for every two rows
                        let shear = (ry / 2) as i32;
                        for cx in 0..crate::font_data::GLYPH_WIDTH {
                            if bits & (1 << (4 - cx)) == 0 {
                                continue;
                            }
                            for dy in 0..scale as i32 {
                                for dx in 0..scale as i32 {
                                    let px = pen_x + (cx * scale as usize) as i32 + dx + shear;
                                    let py = top + (ry * scale as usize) as i32 + dy;
                                    if let Some(clip) = clip {
                                        if !clip.contains(tb_core::Point::new(
                                            px as f32 + 0.5,
                                            py as f32 + 0.5,
                                        )) {
                                            continue;
                                        }
                                    }
                                    fb.blend_at(px, py, self.color);
                                }
                            }
                        }
                    }
                }
            }
            pen_x += adv;
        }
        (pen_x - x.round() as i32) as f32
    }

    fn draw_decoration(
        &self,
        fb: &mut Framebuffer,
        x: f32,
        y: f32,
        width: f32,
        thickness: f32,
        clip: Option<Rect>,
    ) {
        fb.fill_rect_clip(
            Rect::new(x.round(), y.round(), width, thickness),
            clip,
            self.color,
        );
    }
}

/// The `<img>` placeholder: our engine does not decode images, so we draw a
/// bordered grey box with the `alt` text (browsers show the same thing when a
/// picture is missing — for us every picture is missing).
pub fn paint_placeholder(
    fb: &mut Framebuffer,
    rect: Rect,
    alt: &str,
    text_size: f32,
    clip: Option<Rect>,
) {
    if rect.is_empty() {
        return;
    }
    let bg = Color::rgb(0xee, 0xee, 0xee);
    let line = Color::rgb(0xaa, 0xaa, 0xaa);
    fb.fill_rect_clip(rect, clip, bg);
    fb.stroke_rect(rect, [1.0; 4], [line; 4], Dash::Solid, clip);
    // an "image" glyph: mountain + sun, scaled into the box
    let sun = Rect::new(
        rect.left() + rect.width * 0.18,
        rect.top() + rect.height * 0.18,
        (rect.width.min(rect.height) * 0.14).max(2.0),
        (rect.width.min(rect.height) * 0.14).max(2.0),
    );
    fb.fill_rect_clip(sun, clip, Color::rgb(0xdd, 0xcc, 0x88));
    if rect.width > 24.0 && rect.height > 18.0 {
        let steps = (rect.width / 3.0).max(1.0) as i32;
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let x = (rect.left() + t * rect.width).round() as i32;
            let peak = rect.bottom() - rect.height * 0.45 * (1.0 - (t - 0.35).abs() * 2.0);
            let y = peak.round() as i32;
            for yy in y..rect.bottom().round() as i32 - 1 {
                fb.blend_at(x, yy, Color::rgb(0xbb, 0xcc, 0xbb));
            }
        }
    }
    if !alt.trim().is_empty() && rect.width > 30.0 && rect.height > 14.0 {
        let paint = TextPaint {
            color: Color::rgb(0x60, 0x60, 0x60),
            size: text_size,
            ..Default::default()
        };
        let font = crate::font::Font::system();
        let baseline = rect.bottom() - font.descent(text_size) - 2.0;
        let inner = Rect::new(
            rect.left() + 3.0,
            rect.top() + 2.0,
            rect.width - 6.0,
            rect.height - 4.0,
        );
        // one line of alt text, clipped to the box
        paint.draw(fb, inner.left(), baseline, alt.trim(), Some(inner));
    }
}

/// A subtle checkerboard for transparent areas (used by screenshot tests).
pub fn paint_checkerboard(fb: &mut Framebuffer, cell: u32, a: Color, b: Color) {
    for y in 0..fb.height {
        for x in 0..fb.width {
            let c = if ((x / cell) + (y / cell)) % 2 == 0 { a } else { b };
            fb.pixels[(y * fb.width + x) as usize] = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_paint_fills_and_outlines() {
        let mut fb = Framebuffer::new(20, 20);
        fb.clear(Color::WHITE);
        let p = BoxPaint {
            background: Color::rgb(250, 250, 250),
            border_widths: [2.0; 4],
            border_colors: [Color::BLACK; 4],
            border_dash: Dash::Solid,
            radius: 0.0,
        };
        p.paint(&mut fb, Rect::new(4.0, 4.0, 12.0, 12.0), None);
        assert_eq!(fb.get(4, 4), Color::BLACK);
        assert_eq!(fb.get(5, 5), Color::BLACK);
        assert_eq!(fb.get(10, 10), Color::rgb(250, 250, 250));
        assert_eq!(fb.get(0, 0), Color::WHITE);
    }

    #[test]
    fn text_paint_underlines() {
        let mut fb = Framebuffer::new(60, 24);
        fb.clear(Color::WHITE);
        let p = TextPaint {
            underline: true,
            ..Default::default()
        };
        let w = p.draw(&mut fb, 2.0, 14.0, "link", None);
        assert!(w > 10.0);
        // some ink in the text area and a full underline row
        assert!(fb.count_in_band(4, 14, Color::BLACK) > 20);
        assert!(fb.count_in_band(14, 15, Color::BLACK) >= w as usize - 1);
    }

    #[test]
    fn italic_is_not_upright() {
        let font = crate::font::Font::system();
        let mut upright = Framebuffer::new(40, 20);
        let mut slanted = Framebuffer::new(40, 20);
        upright.clear(Color::WHITE);
        slanted.clear(Color::WHITE);
        font.draw(&mut upright, 2.0, 14.0, "AAAA", 14.0, false, Color::BLACK, None);
        let p = TextPaint {
            italic: true,
            size: 14.0,
            ..Default::default()
        };
        p.draw(&mut slanted, 2.0, 14.0, "AAAA", None);
        assert_ne!(upright, slanted, "italic must change the pixels");
        assert!(slanted.count_non_matching(Color::WHITE) > 0);
    }

    #[test]
    fn placeholder_has_frame_and_alt_text() {
        let mut fb = Framebuffer::new(120, 60);
        fb.clear(Color::WHITE);
        paint_placeholder(&mut fb, Rect::new(10.0, 10.0, 100.0, 40.0), "Fahrrad", 14.0, None);
        assert_eq!(fb.get(10, 10), Color::rgb(0xaa, 0xaa, 0xaa), "border");
        assert_eq!(fb.get(0, 0), Color::WHITE, "outside untouched");
        assert!(fb.count_non_matching(Color::WHITE) > 500);
        // tiny placeholders skip the text but keep the box
        paint_placeholder(&mut fb, Rect::new(0.0, 50.0, 12.0, 8.0), "text", 14.0, None);
        assert_eq!(fb.get(0, 50), Color::rgb(0xaa, 0xaa, 0xaa));
    }

    #[test]
    fn everything_respects_the_clip() {
        let mut fb = Framebuffer::new(40, 40);
        fb.clear(Color::WHITE);
        let clip = Rect::new(0.0, 0.0, 10.0, 40.0);
        let p = BoxPaint {
            background: Color::RED,
            border_widths: [3.0; 4],
            border_colors: [Color::BLACK; 4],
            ..Default::default()
        };
        p.paint(&mut fb, Rect::new(5.0, 5.0, 30.0, 30.0), Some(clip));
        assert_eq!(fb.get(30, 20), Color::WHITE, "right of the clip is clean");
        assert_eq!(fb.get(6, 6), Color::BLACK);
    }

    #[test]
    fn checkerboard_alternates() {
        let mut fb = Framebuffer::new(4, 4);
        paint_checkerboard(&mut fb, 2, Color::WHITE, Color::BLACK);
        assert_eq!(fb.get(0, 0), Color::WHITE);
        assert_eq!(fb.get(2, 0), Color::BLACK);
        assert_eq!(fb.get(0, 2), Color::BLACK);
    }
}
