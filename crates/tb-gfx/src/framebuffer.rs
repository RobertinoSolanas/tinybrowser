//! The framebuffer: an ARGB8 pixel buffer with alpha compositing.
//!
//! Coordinates are device pixels (`u32`), but every drawing entry point takes
//! `f32` geometry (`Rect` from the layout phase) and rounds outward the way
//! browsers do: a 1 px border at `y = 10.4` covers row 10..12.
//!
//! All `*_clip` variants take an additional clip rectangle — the layout engine
//! passes the padding box of every `overflow: hidden` ancestor, which is how
//! clipping works without a clip stack data structure.

use tb_core::{Color, Rect};

/// A pixel buffer, row-major, `width * height` pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Color>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Framebuffer {
        let w = width.min(4096);
        let h = height.min(16384);
        Framebuffer {
            width: w,
            height: h,
            pixels: vec![Color::WHITE; (w * h) as usize],
        }
    }

    pub fn clear(&mut self, c: Color) {
        for p in self.pixels.iter_mut() {
            *p = c;
        }
    }

    /// Fill everything with `c`, ignoring alpha (used for the page background).
    pub fn fill_opaque(&mut self, c: Color) {
        let solid = Color::rgba(c.r, c.g, c.b, 255);
        for p in self.pixels.iter_mut() {
            *p = solid;
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::from_points(
            tb_core::Point::new(0.0, 0.0),
            tb_core::Point::new(self.width as f32, self.height as f32),
        )
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> Color {
        if x >= self.width || y >= self.height {
            return Color::TRANSPARENT;
        }
        self.pixels[(y * self.width + x) as usize]
    }

    /// Alpha-composite `c` onto the pixel at `(x, y)`; out of range is a no-op.
    #[inline]
    pub fn blend_at(&mut self, x: i32, y: i32, c: Color) {
        if c.a == 0 || x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.width || y >= self.height {
            return;
        }
        let i = (y * self.width + x) as usize;
        self.pixels[i] = c.blend_over(self.pixels[i]);
    }

    /// Paint a rectangle (alpha aware), optionally clipped.
    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        self.fill_rect_clip(r, None, c);
    }

    pub fn fill_rect_clip(&mut self, r: Rect, clip: Option<Rect>, c: Color) {
        if c.a == 0 || r.is_empty() {
            return;
        }
        let area = match clip {
            Some(clip) => r.intersect(clip),
            None => r,
        };
        if area.is_empty() {
            return;
        }
        let x0 = area.left().max(0.0).floor() as i32;
        let y0 = area.top().max(0.0).floor() as i32;
        let x1 = area.right().min(self.width as f32).ceil() as i32;
        let y1 = area.bottom().min(self.height as f32).ceil() as i32;
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend_at(x, y, c);
            }
        }
    }

    /// Horizontal line of `len` px starting at `(x, y)`.
    pub fn hline(&mut self, x: i32, y: i32, len: i32, c: Color) {
        for i in 0..len {
            self.blend_at(x + i, y, c);
        }
    }

    pub fn vline(&mut self, x: i32, y: i32, len: i32, c: Color) {
        for i in 0..len {
            self.blend_at(x, y + i, c);
        }
    }

    /// Outline a rect with per-side widths/colours. `style` decides whether the
    /// line is solid, dashed (4 on / 3 off) or dotted (1 on / 1 off).
    pub fn stroke_rect(
        &mut self,
        r: Rect,
        widths: [f32; 4],
        colors: [Color; 4],
        dash: Dash,
        clip: Option<Rect>,
    ) {
        let (t, ri, b, l) = (
            widths[0].max(0.0) as i32,
            widths[1].max(0.0) as i32,
            widths[2].max(0.0) as i32,
            widths[3].max(0.0) as i32,
        );
        let x0 = r.left().round() as i32;
        let y0 = r.top().round() as i32;
        let x1 = r.right().round() as i32;
        let y1 = r.bottom().round() as i32;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        // top / bottom
        for (y_off, w, col, phase) in [(0, t, colors[0], 0), (1, b, colors[2], 1)] {
            if w == 0 {
                continue;
            }
            let band = if y_off == 0 { y0..(y0 + w) } else { (y1 - w)..y1 };
            for y in band {
                for x in x0..x1 {
                    if dash.paints(x - x0, y - y0, phase) {
                        self.blend_clip(x, y, col, clip);
                    }
                }
            }
        }
        // left / right (inside the horizontal bands)
        for (x_off, w, col, phase) in [(0, l, colors[3], 2), (1, ri, colors[1], 3)] {
            if w == 0 {
                continue;
            }
            let band = if x_off == 0 { x0..(x0 + w) } else { (x1 - w)..x1 };
            for x in band {
                for y in (y0 + t)..(y1 - b) {
                    if dash.paints(x - x0, y - y0, phase) {
                        self.blend_clip(x, y, col, clip);
                    }
                }
            }
        }
    }

    #[inline]
    fn blend_clip(&mut self, x: i32, y: i32, c: Color, clip: Option<Rect>) {
        if let Some(clip) = clip {
            if !clip.contains(tb_core::Point::new(x as f32 + 0.5, y as f32 + 0.5)) {
                return;
            }
        }
        self.blend_at(x, y, c);
    }

    /// Rounded corners: fill a rect minus the four corner "outside" areas.
    /// Cheap, no anti-aliasing (we are a tiny browser).
    pub fn fill_round_rect(&mut self, r: Rect, radius: f32, c: Color, clip: Option<Rect>) {
        if radius <= 0.0 {
            return self.fill_rect_clip(r, clip, c);
        }
        let rad = radius.min(r.width.min(r.height) / 2.0);
        self.fill_rect_clip(
            Rect::from_points(
                tb_core::Point::new(r.left(), r.top() + rad),
                tb_core::Point::new(r.right(), r.bottom() - rad),
            ),
            clip,
            c,
        );
        self.fill_rect_clip(
            Rect::from_points(
                tb_core::Point::new(r.left() + rad, r.top()),
                tb_core::Point::new(r.right() - rad, r.bottom()),
            ),
            clip,
            c,
        );
        for (cx, cy) in [
            (r.left() + rad, r.top() + rad),
            (r.right() - rad, r.top() + rad),
            (r.left() + rad, r.bottom() - rad),
            (r.right() - rad, r.bottom() - rad),
        ] {
            for dy in 0..rad.ceil() as i32 {
                for dx in 0..rad.ceil() as i32 {
                    let px = cx + dx as f32 + 0.5;
                    let py = cy + dy as f32 + 0.5;
                    let (qx, qy) = (px - cx, py - cy);
                    if qx * qx + qy * qy > rad * rad {
                        continue;
                    }
                    // mirror into all four corners
                    self.blend_clip(px.floor() as i32, py.floor() as i32, c, clip);
                }
            }
        }
    }

    /// How many pixels differ from `c` (used by screenshot tests).
    pub fn count_non_matching(&self, c: Color) -> usize {
        self.pixels.iter().filter(|p| **p != c).count()
    }

    pub fn is_blank(&self) -> bool {
        match self.pixels.first() {
            None => true,
            Some(first) => self.pixels.iter().all(|p| p == first),
        }
    }

    /// Count of pixels in a colour band — handy for "did the header render".
    pub fn count_in_band(&self, y0: u32, y1: u32, c: Color) -> usize {
        let mut n = 0;
        for y in y0..y1.min(self.height) {
            for x in 0..self.width {
                if self.get(x, y) == c {
                    n += 1;
                }
            }
        }
        n
    }

    /// Horizontal/vertical profile of "ink" (non-background) pixels — used by
    /// tests to assert that something is actually on the page.
    pub fn ink_rows(&self, bg: Color) -> Vec<usize> {
        let mut rows = Vec::new();
        for y in 0..self.height {
            let n = (0..self.width)
                .filter(|x| self.get(*x, y) != bg)
                .count();
            if n > 0 {
                rows.push(n);
            }
        }
        rows
    }
}

/// Line style for our border rendering (CSS `border-style` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dash {
    Solid,
    Dashed,
    Dotted,
    /// two lines 1px with a gap (CSS `double`)
    Double,
}

impl Dash {
    pub fn paints(self, x: i32, y: i32, phase: i32) -> bool {
        match self {
            Dash::Solid => true,
            Dash::Dashed => {
                let t = if phase < 2 { x } else { y };
                (t % 7 + 7) % 7 < 4
            }
            Dash::Dotted => {
                let t = if phase < 2 { x } else { y };
                (t % 2 + 2) % 2 == 0
            }
            Dash::Double => {
                let t = if phase < 2 { y } else { x };
                t % 3 != 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_and_get() {
        let mut fb = Framebuffer::new(4, 2);
        assert!(fb.is_blank());
        fb.clear(Color::RED);
        assert_eq!(fb.get(0, 0), Color::RED);
        assert_eq!(fb.get(3, 1), Color::RED);
        assert_eq!(fb.pixels.len(), 8);
    }

    #[test]
    fn blend_is_alpha_aware() {
        let mut fb = Framebuffer::new(2, 2);
        fb.clear(Color::WHITE);
        // 50% black over white
        fb.blend_at(0, 0, Color::rgba(0, 0, 0, 128));
        let c = fb.get(0, 0);
        assert!(c.r < 200 && c.r > 60, "got {c:?}");
        // fully transparent: nothing changes
        fb.blend_at(1, 1, Color::TRANSPARENT);
        assert_eq!(fb.get(1, 1), Color::WHITE);
    }

    #[test]
    fn out_of_bounds_is_safe() {
        let mut fb = Framebuffer::new(2, 2);
        fb.blend_at(-5, 100, Color::RED);
        fb.fill_rect(
            Rect::from_points(tb_core::Point::new(-10.0, -10.0), tb_core::Point::new(1000.0, 1000.0)),
            Color::RED,
        );
        assert_eq!(fb.get(0, 0), Color::RED);
    }

    #[test]
    fn clipping_trims_the_paint() {
        let mut fb = Framebuffer::new(10, 10);
        fb.clear(Color::WHITE);
        let clip = Rect::from_points(tb_core::Point::new(0.0, 0.0), tb_core::Point::new(5.0, 10.0));
        fb.fill_rect_clip(
            Rect::from_points(tb_core::Point::new(0.0, 0.0), tb_core::Point::new(10.0, 10.0)),
            Some(clip),
            Color::BLACK,
        );
        assert_eq!(fb.get(0, 0), Color::BLACK);
        assert_eq!(fb.get(4, 9), Color::BLACK);
        assert_eq!(fb.get(5, 0), Color::WHITE);
        assert_eq!(fb.get(9, 9), Color::WHITE);
    }

    #[test]
    fn stroke_draws_four_sides() {
        let mut fb = Framebuffer::new(12, 12);
        fb.clear(Color::WHITE);
        let r = Rect::from_points(tb_core::Point::new(2.0, 2.0), tb_core::Point::new(10.0, 8.0));
        fb.stroke_rect(r, [1.0; 4], [Color::BLACK; 4], Dash::Solid, None);
        assert_eq!(fb.get(2, 2), Color::BLACK, "corner");
        assert_eq!(fb.get(6, 2), Color::BLACK, "top");
        assert_eq!(fb.get(6, 7), Color::BLACK, "bottom");
        assert_eq!(fb.get(2, 5), Color::BLACK, "left");
        assert_eq!(fb.get(9, 5), Color::BLACK, "right");
        assert_eq!(fb.get(6, 5), Color::WHITE, "inside stays");
    }

    #[test]
    fn dashed_and_dotted_have_gaps() {
        for (dash, expect_gaps) in [(Dash::Solid, false), (Dash::Dashed, true), (Dash::Dotted, true)] {
            let mut fb = Framebuffer::new(24, 3);
            fb.clear(Color::WHITE);
            let r = Rect::from_points(tb_core::Point::new(0.0, 0.0), tb_core::Point::new(24.0, 1.0));
            fb.stroke_rect(r, [1.0, 0.0, 0.0, 0.0], [Color::BLACK; 4], dash, None);
            let inks = (0..24).filter(|x| fb.get(*x, 0) == Color::BLACK).count();
            assert!(inks > 0, "{dash:?} painted nothing");
            assert_eq!((inks < 24), expect_gaps, "{dash:?} painted {inks}/24");
        }
    }

    #[test]
    fn rounded_rect_covers_center_and_corners_less() {
        let mut fb = Framebuffer::new(20, 20);
        fb.clear(Color::WHITE);
        let r = Rect::from_points(tb_core::Point::new(2.0, 2.0), tb_core::Point::new(18.0, 18.0));
        fb.fill_round_rect(r, 6.0, Color::BLACK, None);
        assert_eq!(fb.get(10, 10), Color::BLACK);
        assert_eq!(fb.get(2, 2), Color::WHITE, "corner is cut");
        assert_eq!(fb.get(10, 2), Color::BLACK, "top edge middle");
    }

    #[test]
    fn helpers_report_ink() {
        let mut fb = Framebuffer::new(8, 4);
        fb.clear(Color::WHITE);
        fb.fill_rect(
            Rect::from_points(tb_core::Point::new(0.0, 1.0), tb_core::Point::new(8.0, 2.0)),
            Color::BLACK,
        );
        assert_eq!(fb.count_non_matching(Color::WHITE), 8);
        assert_eq!(fb.ink_rows(Color::WHITE), vec![8]);
        assert_eq!(fb.count_in_band(1, 2, Color::BLACK), 8);
    }
}
